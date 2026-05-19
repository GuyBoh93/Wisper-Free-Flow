// Always-on-top, click-through overlay window.
//
// On Windows: a pure Win32 layered window (WS_EX_LAYERED + UpdateLayeredWindow)
// with per-pixel alpha, drawn each frame via tiny-skia and blitted with
// premultiplied BGRA. This mirrors the proven approach from the Python
// prototype's overlay.py.

use parking_lot::Mutex;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayState {
    Idle,
    Recording,
    Processing,
    NoMic,
}

pub struct OverlaySharedInner {
    pub state: OverlayState,
    pub audio_level: f32,
    /// 0.0..=1.0. Driven by whisper's progress callback during Processing;
    /// reset to 0 when entering Idle (so the next session starts clean
    /// instead of flashing the previous run's fill).
    pub progress: f32,
}

pub type SharedOverlay = Arc<Mutex<OverlaySharedInner>>;

pub fn new_shared() -> SharedOverlay {
    Arc::new(Mutex::new(OverlaySharedInner {
        state: OverlayState::Idle,
        audio_level: 0.0,
        progress: 0.0,
    }))
}

#[cfg(target_os = "windows")]
pub fn spawn(shared: SharedOverlay) {
    std::thread::spawn(move || {
        if let Err(e) = unsafe { win32::run(shared) } {
            tracing::error!("overlay thread error: {e:#}");
        }
    });
}

#[cfg(not(target_os = "windows"))]
pub fn spawn(_shared: SharedOverlay) {
    tracing::warn!("overlay not implemented on this platform yet");
}

#[cfg(target_os = "windows")]
mod win32 {
    use super::{OverlayState, SharedOverlay};
    use anyhow::{Result, anyhow};
    use std::ffi::c_void;
    use std::mem;
    use std::ptr;
    use std::time::Instant;
    use tiny_skia::{Color, FillRule, Paint, PathBuilder, Pixmap, Stroke, Transform};
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
    use windows_sys::Win32::Graphics::Gdi::*;
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::HiDpi::{
        DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::*;

    // Icon-sized pill, horizontally centred in the work area, sitting just
    // above the taskbar. Compact on purpose — a glance indicator, not a HUD.
    const WIDTH: i32 = 140;
    const HEIGHT: i32 = 44;
    const BOTTOM_MARGIN: i32 = 16; // gap above the taskbar
    const ANIM_MS: u32 = 33;
    const POLL_MS: u32 = 16;
    const TIMER_ANIM: usize = 1;
    const TIMER_POLL: usize = 2;

    struct Ctx {
        shared: SharedOverlay,
        pixmap: Pixmap,
        bgra: Vec<u8>,
        phase: f32,
        // Smoothly interpolated 0.0 (bars / recording) -> 1.0 (dots in
        // sine wave / processing). Lets the two states morph into each
        // other instead of cutting.
        morph: f32,
        target_morph: f32,
        last_state: OverlayState,
        is_shown: bool,
        _started: Instant,
    }

    pub unsafe fn run(shared: SharedOverlay) -> Result<()> {
        unsafe {
            // Per-monitor DPI awareness so window coordinates correspond to real
            // pixels on HiDPI displays (otherwise Win11 virtualises us and the
            // window can land off-screen or behind DWM compositing).
            let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);

            let hinst = GetModuleHandleW(ptr::null());
            if hinst.is_null() {
                return Err(anyhow!("GetModuleHandleW failed"));
            }

            let class_name = wide("WhisperFreeFlowOverlay");
            let window_name = wide("Whisper FreeFlow");

            let mut wc: WNDCLASSEXW = mem::zeroed();
            wc.cbSize = mem::size_of::<WNDCLASSEXW>() as u32;
            wc.lpfnWndProc = Some(wndproc);
            wc.hInstance = hinst;
            wc.lpszClassName = class_name.as_ptr();
            wc.hCursor = LoadCursorW(ptr::null_mut(), IDC_ARROW);

            if RegisterClassExW(&wc) == 0 {
                return Err(anyhow!("RegisterClassExW failed"));
            }

            // Work area excludes the taskbar, so anchoring to (left, bottom)
            // of that rect puts us just above it on every taskbar position.
            let mut work_area: RECT = mem::zeroed();
            let ok = SystemParametersInfoW(
                SPI_GETWORKAREA,
                0,
                &mut work_area as *mut _ as *mut c_void,
                0,
            );
            if ok == 0 {
                work_area.left = 0;
                work_area.top = 0;
                work_area.right = GetSystemMetrics(SM_CXSCREEN);
                work_area.bottom = GetSystemMetrics(SM_CYSCREEN);
            }
            let work_w = work_area.right - work_area.left;
            let x = work_area.left + (work_w - WIDTH) / 2;
            let y = work_area.bottom - HEIGHT - BOTTOM_MARGIN;

            let ex_style: u32 = WS_EX_LAYERED
                | WS_EX_TOPMOST
                | WS_EX_TRANSPARENT
                | WS_EX_TOOLWINDOW
                | WS_EX_NOACTIVATE;

            let hwnd = CreateWindowExW(
                ex_style,
                class_name.as_ptr(),
                window_name.as_ptr(),
                WS_POPUP,
                x,
                y,
                WIDTH,
                HEIGHT,
                ptr::null_mut(),
                ptr::null_mut(),
                hinst,
                ptr::null(),
            );

            if hwnd.is_null() {
                return Err(anyhow!("CreateWindowExW failed"));
            }

            let pixmap = Pixmap::new(WIDTH as u32, HEIGHT as u32)
                .ok_or_else(|| anyhow!("failed to allocate pixmap"))?;
            let bgra = vec![0u8; (WIDTH * HEIGHT * 4) as usize];

            let ctx = Box::new(Ctx {
                shared,
                pixmap,
                bgra,
                phase: 0.0,
                morph: 0.0,
                target_morph: 0.0,
                last_state: OverlayState::Idle,
                is_shown: false,
                _started: Instant::now(),
            });
            let ctx_raw = Box::into_raw(ctx);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, ctx_raw as isize);

            // Paint once before any ShowWindow happens — a layered window with
            // no UpdateLayeredWindow call is invisible, so doing this here means
            // the very first transition to Recording shows pixels instantly.
            {
                let ctx_ref = &mut *ctx_raw;
                render(ctx_ref);
                blit(ctx_ref, hwnd);
            }

            SetTimer(hwnd, TIMER_ANIM, ANIM_MS, None);
            SetTimer(hwnd, TIMER_POLL, POLL_MS, None);

            tracing::info!(
                "overlay window created at ({x},{y}) {WIDTH}x{HEIGHT} (work area {}x{}..{}x{})",
                work_area.left,
                work_area.top,
                work_area.right,
                work_area.bottom
            );

            let mut msg: MSG = mem::zeroed();
            while GetMessageW(&mut msg, ptr::null_mut(), 0, 0) > 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }

            Ok(())
        }
    }

    unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        unsafe {
            let ctx_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Ctx;
            if ctx_ptr.is_null() {
                return DefWindowProcW(hwnd, msg, wp, lp);
            }
            let ctx = &mut *ctx_ptr;

            match msg {
                WM_TIMER => {
                    match wp as usize {
                        TIMER_ANIM => on_anim(ctx, hwnd),
                        TIMER_POLL => on_poll(ctx, hwnd),
                        _ => {}
                    }
                    0
                }
                WM_DESTROY => {
                    KillTimer(hwnd, TIMER_ANIM);
                    KillTimer(hwnd, TIMER_POLL);
                    drop(Box::from_raw(ctx_ptr));
                    PostQuitMessage(0);
                    0
                }
                _ => DefWindowProcW(hwnd, msg, wp, lp),
            }
        }
    }

    unsafe fn on_poll(ctx: &mut Ctx, hwnd: HWND) {
        unsafe {
            let state = ctx.shared.lock().state;

            if state != ctx.last_state {
                tracing::info!("overlay state -> {state:?}");
                ctx.last_state = state;

                // Snap morph back to bars when we go fully idle, so the next
                // recording session starts clean rather than mid-transition.
                if state == OverlayState::Idle {
                    ctx.morph = 0.0;
                }

                ctx.target_morph = match state {
                    OverlayState::Recording => 0.0,
                    OverlayState::Processing => 1.0,
                    OverlayState::Idle => 0.0,
                    OverlayState::NoMic => 0.0,
                };
            }

            let should_show = state != OverlayState::Idle;
            if should_show != ctx.is_shown {
                let cmd = if should_show { SW_SHOWNOACTIVATE } else { SW_HIDE };
                ShowWindow(hwnd, cmd);
                ctx.is_shown = should_show;
            }
        }
    }

    unsafe fn on_anim(ctx: &mut Ctx, hwnd: HWND) {
        unsafe {
            if !ctx.is_shown {
                return;
            }
            ctx.phase += 0.18;
            // Critically-damped-ish exponential ease toward the target.
            ctx.morph += (ctx.target_morph - ctx.morph) * 0.22;
            render(ctx);
            blit(ctx, hwnd);
        }
    }

    fn render(ctx: &mut Ctx) {
        let (state, level, progress) = {
            let s = ctx.shared.lock();
            (s.state, s.audio_level, s.progress)
        };

        ctx.pixmap.fill(Color::TRANSPARENT);

        // Body: near-black pill with high alpha. Reads cleanly on light and
        // dark desktops without competing with the content behind it.
        if let Some(p) = rounded_rect_path(
            0.0,
            0.0,
            WIDTH as f32,
            HEIGHT as f32,
            HEIGHT as f32 / 2.0,
        ) {
            let mut bg = Paint::default();
            bg.set_color_rgba8(18, 20, 28, 222);
            bg.anti_alias = true;
            ctx.pixmap
                .fill_path(&p, &bg, FillRule::Winding, Transform::identity(), None);
        }

        // 1px glassy top-edge highlight — gives the pill depth without looking
        // skeuomorphic. Inset by 0.5px so the stroke sits inside the body.
        if let Some(p) = rounded_rect_path(
            0.5,
            0.5,
            WIDTH as f32 - 1.0,
            HEIGHT as f32 - 1.0,
            (HEIGHT as f32 - 1.0) / 2.0,
        ) {
            let mut paint = Paint::default();
            paint.set_color_rgba8(255, 255, 255, 26);
            paint.anti_alias = true;
            let stroke = Stroke {
                width: 1.0,
                ..Default::default()
            };
            ctx.pixmap
                .stroke_path(&p, &paint, &stroke, Transform::identity(), None);
        }

        match state {
            OverlayState::Idle => {}
            OverlayState::NoMic => draw_no_mic(&mut ctx.pixmap),
            OverlayState::Recording => {
                draw_visualizer(&mut ctx.pixmap, ctx.phase, level, ctx.morph);
            }
            OverlayState::Processing => {
                draw_visualizer(&mut ctx.pixmap, ctx.phase, level, ctx.morph);
                draw_progress_bar(&mut ctx.pixmap, progress);
            }
        }
    }

    // Thin progress bar at the bottom inside-edge of the pill. Sits below the
    // dots so the "alive" animation continues to read; the bar adds the
    // "how much longer?" cue the user needs. ~3px tall is enough to be
    // legible without competing with the visualiser for vertical space.
    fn draw_progress_bar(pixmap: &mut Pixmap, progress: f32) {
        let progress = progress.clamp(0.0, 1.0);
        let bar_h: f32 = 3.0;
        let inset: f32 = 12.0;
        let y = HEIGHT as f32 - bar_h - 4.0;
        let track_w = WIDTH as f32 - 2.0 * inset;

        // Faint track so the bar's extent is visible even at 0% progress.
        if let Some(p) = rounded_rect_path(inset, y, track_w, bar_h, bar_h / 2.0) {
            let mut paint = Paint::default();
            paint.set_color_rgba8(255, 255, 255, 38);
            paint.anti_alias = true;
            pixmap.fill_path(&p, &paint, FillRule::Winding, Transform::identity(), None);
        }

        // Filled portion — same blue family as the visualiser pips so the two
        // elements feel like one indicator instead of two.
        let fill_w = (track_w * progress).max(0.0);
        if fill_w > 0.5 {
            if let Some(p) = rounded_rect_path(inset, y, fill_w, bar_h, bar_h / 2.0) {
                let mut paint = Paint::default();
                paint.set_color_rgba8(200, 222, 255, 240);
                paint.anti_alias = true;
                pixmap.fill_path(&p, &paint, FillRule::Winding, Transform::identity(), None);
            }
        }
    }

    // Slashed-microphone glyph: capsule mic body, stand, and a red diagonal
    // strike. Geometry is hand-tuned for the 140x44 pill — change the pill
    // dims and these will need re-centering.
    fn draw_no_mic(pixmap: &mut Pixmap) {
        let mid_x = WIDTH as f32 / 2.0;
        let mid_y = HEIGHT as f32 / 2.0;

        // Mic body: vertical capsule.
        let body_w: f32 = 12.0;
        let body_h: f32 = 22.0;
        let bx = mid_x - body_w / 2.0;
        let by = mid_y - body_h / 2.0 - 2.0;
        if let Some(p) = rounded_rect_path(bx, by, body_w, body_h, body_w / 2.0) {
            let mut paint = Paint::default();
            paint.set_color_rgba8(220, 226, 240, 235);
            paint.anti_alias = true;
            pixmap.fill_path(&p, &paint, FillRule::Winding, Transform::identity(), None);
        }

        // U-shaped stand: shallow arc under the body, plus a short stem and
        // base bar. Drawn as a thick stroke for the arc and a filled pill for
        // the base, keeping the primitive set small.
        let arc_y = by + body_h + 1.0;
        let arc_w: f32 = 22.0;
        let arc_h: f32 = 8.0;
        let arc_x = mid_x - arc_w / 2.0;
        let mut arc = PathBuilder::new();
        arc.move_to(arc_x, arc_y);
        arc.quad_to(mid_x, arc_y + arc_h * 2.0, arc_x + arc_w, arc_y);
        if let Some(path) = arc.finish() {
            let mut paint = Paint::default();
            paint.set_color_rgba8(220, 226, 240, 235);
            paint.anti_alias = true;
            let stroke = Stroke { width: 2.0, ..Default::default() };
            pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
        }
        // Base bar.
        let base_w: f32 = 14.0;
        let base_h: f32 = 2.5;
        if let Some(p) = rounded_rect_path(
            mid_x - base_w / 2.0,
            arc_y + arc_h + 2.0,
            base_w,
            base_h,
            base_h / 2.0,
        ) {
            let mut paint = Paint::default();
            paint.set_color_rgba8(220, 226, 240, 235);
            paint.anti_alias = true;
            pixmap.fill_path(&p, &paint, FillRule::Winding, Transform::identity(), None);
        }

        // Red diagonal slash across the whole glyph. Drawn as a filled capsule
        // (rotated rect) so it gets anti-aliased edges and a uniform width
        // without needing a thick stroke + cap geometry.
        let slash_len: f32 = 36.0;
        let slash_w: f32 = 4.0;
        let angle = -45f32.to_radians();
        let cos = angle.cos();
        let sin = angle.sin();
        if let Some(p) = rounded_rect_path(
            -slash_len / 2.0,
            -slash_w / 2.0,
            slash_len,
            slash_w,
            slash_w / 2.0,
        ) {
            let transform = Transform::from_row(cos, sin, -sin, cos, mid_x, mid_y);
            let mut paint = Paint::default();
            paint.set_color_rgba8(255, 80, 90, 255);
            paint.anti_alias = true;
            pixmap.fill_path(&p, &paint, FillRule::Winding, transform, None);
        }
    }

    fn rounded_rect_path(x: f32, y: f32, w: f32, h: f32, r: f32) -> Option<tiny_skia::Path> {
        let mut pb = PathBuilder::new();
        pb.move_to(x + r, y);
        pb.line_to(x + w - r, y);
        pb.quad_to(x + w, y, x + w, y + r);
        pb.line_to(x + w, y + h - r);
        pb.quad_to(x + w, y + h, x + w - r, y + h);
        pb.line_to(x + r, y + h);
        pb.quad_to(x, y + h, x, y + h - r);
        pb.line_to(x, y + r);
        pb.quad_to(x, y, x + r, y);
        pb.close();
        pb.finish()
    }

    // Number / size of bars. A bar with height == width is a circle, so the
    // same primitive renders both the recording bars and the processing dots.
    const N_BARS: usize = 9;
    const BAR_W: f32 = 5.0;
    const BAR_GAP: f32 = 4.0;
    const BAR_MIN_H: f32 = 6.0;
    const BAR_MAX_H: f32 = 34.0;

    fn draw_visualizer(pixmap: &mut Pixmap, phase: f32, level: f32, morph: f32) {
        let total_w = N_BARS as f32 * BAR_W + (N_BARS as f32 - 1.0) * BAR_GAP;
        let mid_x = WIDTH as f32 / 2.0;
        let mid_y = HEIGHT as f32 / 2.0;
        let start_x = mid_x - total_w / 2.0;

        let dot_h = BAR_W; // "dot" = pill where height == width
        let wave_amp = (HEIGHT as f32 / 2.0 - dot_h / 2.0 - 4.0).max(4.0);

        // A baseline amount of liveness so the bars dance even in silence;
        // loud audio amplifies the range.
        let level = level.clamp(0.0, 1.0);
        let drive = 0.30 + level * 0.70;
        let morph = morph.clamp(0.0, 1.0);

        for i in 0..N_BARS {
            let x = start_x + i as f32 * (BAR_W + BAR_GAP);

            // Per-bar oscillation at slightly offset frequencies — fakes a
            // spectrum analyser without needing an FFT.
            let bar_phase = phase * 0.55 + i as f32 * 0.95;
            let osc = bar_phase.sin() * 0.5 + 0.5;
            let bar_h = BAR_MIN_H + (BAR_MAX_H - BAR_MIN_H) * osc * drive;

            // Sine-wave path the dots travel along while processing.
            let wave_phase = phase * 0.32 + i as f32 * 0.75;
            let wave_y = mid_y + wave_amp * wave_phase.sin();

            // Interpolate shape (bar height -> dot height) and position
            // (centered -> riding the wave).
            let h = lerp(bar_h, dot_h, morph);
            let y_center = lerp(mid_y, wave_y, morph);
            let y_top = y_center - h / 2.0;

            draw_pip(pixmap, x, y_top, BAR_W, h);
        }
    }

    fn draw_pip(pixmap: &mut Pixmap, x: f32, y: f32, w: f32, h: f32) {
        let r = w / 2.0;

        // Soft halo behind every pip — the unifying "glow" cue.
        if let Some(p) = rounded_rect_path(x - 1.5, y - 1.5, w + 3.0, h + 3.0, r + 1.5) {
            let mut paint = Paint::default();
            paint.set_color_rgba8(130, 175, 255, 70);
            paint.anti_alias = true;
            pixmap.fill_path(
                &p,
                &paint,
                FillRule::Winding,
                Transform::identity(),
                None,
            );
        }

        // Bright core in the same blue family as the halo.
        if let Some(p) = rounded_rect_path(x, y, w, h, r) {
            let mut paint = Paint::default();
            paint.set_color_rgba8(200, 222, 255, 255);
            paint.anti_alias = true;
            pixmap.fill_path(
                &p,
                &paint,
                FillRule::Winding,
                Transform::identity(),
                None,
            );
        }
    }

    fn lerp(a: f32, b: f32, t: f32) -> f32 {
        a + (b - a) * t
    }

    unsafe fn blit(ctx: &mut Ctx, hwnd: HWND) {
        unsafe {
            let rgba = ctx.pixmap.data();
            for i in 0..(WIDTH * HEIGHT) as usize {
                let r = rgba[i * 4] as u32;
                let g = rgba[i * 4 + 1] as u32;
                let b = rgba[i * 4 + 2] as u32;
                let a = rgba[i * 4 + 3] as u32;
                ctx.bgra[i * 4] = ((b * a) / 255) as u8;
                ctx.bgra[i * 4 + 1] = ((g * a) / 255) as u8;
                ctx.bgra[i * 4 + 2] = ((r * a) / 255) as u8;
                ctx.bgra[i * 4 + 3] = a as u8;
            }

            let hdc_screen = GetDC(ptr::null_mut());
            let hdc_mem = CreateCompatibleDC(hdc_screen);

            let mut bmi: BITMAPINFO = mem::zeroed();
            bmi.bmiHeader.biSize = mem::size_of::<BITMAPINFOHEADER>() as u32;
            bmi.bmiHeader.biWidth = WIDTH;
            bmi.bmiHeader.biHeight = -HEIGHT;
            bmi.bmiHeader.biPlanes = 1;
            bmi.bmiHeader.biBitCount = 32;
            bmi.bmiHeader.biCompression = BI_RGB as u32;

            let mut bits: *mut c_void = ptr::null_mut();
            let hbmp = CreateDIBSection(
                hdc_mem,
                &bmi,
                DIB_RGB_COLORS,
                &mut bits,
                ptr::null_mut(),
                0,
            );

            if !hbmp.is_null() && !bits.is_null() {
                ptr::copy_nonoverlapping(ctx.bgra.as_ptr(), bits as *mut u8, ctx.bgra.len());
                let old = SelectObject(hdc_mem, hbmp as _);

                let size = SIZE {
                    cx: WIDTH,
                    cy: HEIGHT,
                };
                let src_pt = POINT { x: 0, y: 0 };
                let blend = BLENDFUNCTION {
                    BlendOp: AC_SRC_OVER as u8,
                    BlendFlags: 0,
                    SourceConstantAlpha: 255,
                    AlphaFormat: AC_SRC_ALPHA as u8,
                };

                UpdateLayeredWindow(
                    hwnd,
                    hdc_screen,
                    ptr::null(),
                    &size,
                    hdc_mem,
                    &src_pt,
                    0,
                    &blend,
                    ULW_ALPHA,
                );

                SelectObject(hdc_mem, old);
                DeleteObject(hbmp as _);
            }

            DeleteDC(hdc_mem);
            ReleaseDC(ptr::null_mut(), hdc_screen);
        }
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }
}
