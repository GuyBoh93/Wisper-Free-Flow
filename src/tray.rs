// System tray icon. Runs on the main thread.
//
// Windows: tray-icon's internal hidden window posts menu/click events into the
// thread's message queue, which we dispatch + drain.
//
// macOS: tray-icon must be created from the main thread, and the Cocoa run
// loop must be pumped. We use `tao::EventLoop` for that — it routes menu
// events through `MenuEvent::receiver()` exactly like on Windows.

use crate::recorder::list_input_devices;
use anyhow::{Context, Result};
use std::sync::mpsc::Sender;
use tray_icon::{
    Icon, TrayIcon, TrayIconBuilder,
    menu::{CheckMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu},
};

#[cfg(target_os = "windows")]
use std::{mem, time::Duration};
#[cfg(target_os = "windows")]
use windows_sys::Win32::UI::WindowsAndMessaging::*;

/// Control message from the tray (main thread) to the worker thread.
pub enum ControlEvent {
    SwitchModel(String),
    SetAutostart(bool),
    /// `None` = system default; `Some(name)` = match by cpal device name.
    SetInputDevice(Option<String>),
    /// Abort the current transcription (if any). The whisper inference keeps
    /// running but its result is discarded instead of being typed.
    CancelTranscription,
}

/// Models offered in the tray submenu. (id, label)
/// `id` matches the Hugging Face filename suffix (e.g. ggml-<id>.bin).
const MODELS: &[(&str, &str)] = &[
    ("tiny.en",      "Tiny.en — fastest (~75 MB)"),
    ("base.en",      "Base.en (~150 MB)"),
    ("base.en-q5_1", "Base.en quantized (~60 MB)"),
    ("small.en",     "Small.en (~500 MB)"),
    ("medium.en",    "Medium.en — best, slowest (~1.5 GB)"),
];

/// One row in the model submenu. Held in a Vec so the click handler can
/// find which model a clicked id maps to and update check marks.
struct ModelEntry {
    id: MenuId,
    name: String,
    item: CheckMenuItem,
}

/// One row in the microphone submenu. `name = None` represents the "System
/// default" row (uses cpal's `default_input_device()`); `Some(_)` is a named
/// device. Stored in a Vec so the click handler can resolve clicked ids
/// and update radio check marks.
struct MicEntry {
    id: MenuId,
    name: Option<String>,
    item: CheckMenuItem,
}

/// Bundle of everything `install()` hands back to the platform-specific pump.
/// Grouped into a struct so adding new menu items doesn't churn every
/// function signature.
struct TrayHandles {
    tray: TrayIcon,
    quit_id: MenuId,
    cancel_id: MenuId,
    models: Vec<ModelEntry>,
    mics: Vec<MicEntry>,
    autostart_item: CheckMenuItem,
}

#[cfg(target_os = "windows")]
pub fn run_until_quit(
    initial_model: String,
    initial_autostart: bool,
    initial_mic: Option<String>,
    ctrl_tx: Sender<ControlEvent>,
) -> Result<()> {
    let handles = install(&initial_model, initial_autostart, initial_mic.as_deref())?;
    let TrayHandles {
        tray,
        quit_id,
        cancel_id,
        models,
        mics,
        autostart_item,
    } = handles;
    let _tray_guard = tray;

    let rx = MenuEvent::receiver();
    let autostart_id = autostart_item.id().clone();

    unsafe {
        let mut msg: MSG = mem::zeroed();
        loop {
            while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }

            while let Ok(event) = rx.try_recv() {
                if event.id == quit_id {
                    tracing::info!("quit requested from tray");
                    return Ok(());
                }
                if event.id == cancel_id {
                    tracing::info!("tray: cancel transcription");
                    let _ = ctrl_tx.send(ControlEvent::CancelTranscription);
                    continue;
                }
                if event.id == autostart_id {
                    handle_autostart_click(&autostart_item, &ctrl_tx);
                    continue;
                }
                if mics.iter().any(|m| m.id == event.id) {
                    handle_mic_click(&event.id, &mics, &ctrl_tx);
                    continue;
                }
                handle_model_click(&event.id, &models, &ctrl_tx);
            }

            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

#[cfg(target_os = "macos")]
pub fn run_until_quit(
    initial_model: String,
    initial_autostart: bool,
    initial_mic: Option<String>,
    ctrl_tx: Sender<ControlEvent>,
) -> Result<()> {
    use tao::event::Event;
    use tao::event_loop::{ControlFlow, EventLoopBuilder};
    use tao::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS};

    let mut event_loop_builder = EventLoopBuilder::new();
    event_loop_builder.with_activation_policy(ActivationPolicy::Accessory);
    let event_loop = event_loop_builder.build();

    let handles = install(&initial_model, initial_autostart, initial_mic.as_deref())?;
    let TrayHandles {
        tray,
        quit_id,
        cancel_id,
        models,
        mics,
        autostart_item,
    } = handles;
    let _tray_guard = tray;

    let autostart_id = autostart_item.id().clone();
    let menu_rx = MenuEvent::receiver();

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        if matches!(event, Event::NewEvents(_)) {
            while let Ok(menu_event) = menu_rx.try_recv() {
                if menu_event.id == quit_id {
                    tracing::info!("quit requested from tray");
                    *control_flow = ControlFlow::Exit;
                } else if menu_event.id == cancel_id {
                    let _ = ctrl_tx.send(ControlEvent::CancelTranscription);
                } else if menu_event.id == autostart_id {
                    handle_autostart_click(&autostart_item, &ctrl_tx);
                } else if mics.iter().any(|m| m.id == menu_event.id) {
                    handle_mic_click(&menu_event.id, &mics, &ctrl_tx);
                } else {
                    handle_model_click(&menu_event.id, &models, &ctrl_tx);
                }
            }
        }
    })
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub fn run_until_quit(
    initial_model: String,
    initial_autostart: bool,
    initial_mic: Option<String>,
    ctrl_tx: Sender<ControlEvent>,
) -> Result<()> {
    let handles = install(&initial_model, initial_autostart, initial_mic.as_deref())?;
    let TrayHandles {
        tray,
        quit_id,
        cancel_id,
        models,
        mics,
        autostart_item,
    } = handles;
    let _tray_guard = tray;

    let autostart_id = autostart_item.id().clone();
    let rx = MenuEvent::receiver();
    while let Ok(event) = rx.recv() {
        if event.id == quit_id {
            return Ok(());
        }
        if event.id == cancel_id {
            let _ = ctrl_tx.send(ControlEvent::CancelTranscription);
            continue;
        }
        if event.id == autostart_id {
            handle_autostart_click(&autostart_item, &ctrl_tx);
            continue;
        }
        if mics.iter().any(|m| m.id == event.id) {
            handle_mic_click(&event.id, &mics, &ctrl_tx);
            continue;
        }
        handle_model_click(&event.id, &models, &ctrl_tx);
    }
    Ok(())
}

fn handle_autostart_click(item: &CheckMenuItem, ctrl_tx: &Sender<ControlEvent>) {
    let new_state = item.is_checked();
    tracing::info!("tray: autostart -> {}", new_state);
    if ctrl_tx
        .send(ControlEvent::SetAutostart(new_state))
        .is_err()
    {
        tracing::error!("worker disconnected — can't update autostart");
        // Revert the tick so the menu state still reflects reality.
        item.set_checked(!new_state);
    }
}

fn handle_mic_click(id: &MenuId, mics: &[MicEntry], ctrl_tx: &Sender<ControlEvent>) {
    let Some(picked) = mics.iter().find(|m| &m.id == id) else {
        return;
    };
    for m in mics {
        m.item.set_checked(m.id == *id);
    }
    let label = picked.name.as_deref().unwrap_or("(system default)");
    tracing::info!("tray: switching input device -> {label}");
    if ctrl_tx
        .send(ControlEvent::SetInputDevice(picked.name.clone()))
        .is_err()
    {
        tracing::error!("worker disconnected — can't switch input device");
    }
}

fn handle_model_click(id: &MenuId, models: &[ModelEntry], ctrl_tx: &Sender<ControlEvent>) {
    let Some(picked) = models.iter().find(|m| &m.id == id) else {
        return;
    };
    // Radio behaviour: only the picked one stays checked.
    for m in models {
        m.item.set_checked(m.id == *id);
    }
    tracing::info!("tray: switching model -> {}", picked.name);
    if ctrl_tx
        .send(ControlEvent::SwitchModel(picked.name.clone()))
        .is_err()
    {
        tracing::error!("worker disconnected — can't switch model");
    }
}

fn install(
    initial_model: &str,
    initial_autostart: bool,
    initial_mic: Option<&str>,
) -> Result<TrayHandles> {
    let menu = Menu::new();

    let version_label = MenuItem::new(
        format!("Whisper FreeFlow v{}", env!("CARGO_PKG_VERSION")),
        false,
        None,
    );
    menu.append(&version_label)
        .context("appending version label")?;
    menu.append(&PredefinedMenuItem::separator())
        .context("appending separator")?;

    let autostart_item = CheckMenuItem::new("Start at login", true, initial_autostart, None);
    menu.append(&autostart_item)
        .context("appending autostart toggle")?;

    // "Cancel transcription" is always enabled; the worker just ignores it
    // when there's nothing to cancel. Wiring up dynamic enable/disable would
    // require menu-state updates on every overlay transition, which isn't
    // worth the complexity for a button that costs nothing to mis-click.
    let cancel_item = MenuItem::new("Cancel transcription", true, None);
    let cancel_id = cancel_item.id().clone();
    menu.append(&cancel_item)
        .context("appending cancel item")?;

    menu.append(&PredefinedMenuItem::separator())
        .context("appending separator")?;

    // Mic submenu. Built once at startup — devices that arrive/leave later
    // won't show up until the next launch. Selecting one persists to config
    // via ControlEvent::SetInputDevice.
    let mic_submenu = Submenu::new("Microphone", true);
    let mut mics: Vec<MicEntry> = Vec::new();

    let default_checked = initial_mic.is_none();
    let default_item = CheckMenuItem::new("System default", true, default_checked, None);
    let default_id = default_item.id().clone();
    mic_submenu
        .append(&default_item)
        .context("appending default mic item")?;
    mics.push(MicEntry {
        id: default_id,
        name: None,
        item: default_item,
    });

    let device_names = list_input_devices();
    if !device_names.is_empty() {
        mic_submenu
            .append(&PredefinedMenuItem::separator())
            .context("appending mic separator")?;
    }
    let mut saved_mic_found = initial_mic.is_none();
    for name in device_names {
        let checked = initial_mic == Some(name.as_str());
        if checked {
            saved_mic_found = true;
        }
        let item = CheckMenuItem::new(&name, true, checked, None);
        let id = item.id().clone();
        mic_submenu
            .append(&item)
            .context("appending mic item")?;
        mics.push(MicEntry {
            id,
            name: Some(name),
            item,
        });
    }
    if !saved_mic_found {
        tracing::warn!(
            "configured input device `{}` not found on this system; falling back to system default",
            initial_mic.unwrap_or("")
        );
        // Re-tick "System default" so the menu reflects what's actually used.
        if let Some(m) = mics.first() {
            m.item.set_checked(true);
        }
    }
    menu.append(&mic_submenu)
        .context("appending mic submenu")?;

    let model_submenu = Submenu::new("Model", true);
    let mut models = Vec::with_capacity(MODELS.len());
    let mut found_initial = false;
    for (name, label) in MODELS {
        let checked = *name == initial_model;
        if checked {
            found_initial = true;
        }
        let item = CheckMenuItem::new(*label, true, checked, None);
        let id = item.id().clone();
        model_submenu
            .append(&item)
            .context("appending model item")?;
        models.push(ModelEntry {
            id,
            name: (*name).to_string(),
            item,
        });
    }
    if !found_initial {
        tracing::warn!(
            "configured model `{}` isn't in the tray menu; no check mark will be shown",
            initial_model
        );
    }
    menu.append(&model_submenu)
        .context("appending model submenu")?;
    menu.append(&PredefinedMenuItem::separator())
        .context("appending separator")?;

    let quit_item = MenuItem::new("Quit Whisper FreeFlow", true, None);
    let quit_id = quit_item.id().clone();
    menu.append(&quit_item).context("appending quit menu item")?;

    let (rgba, w, h) = generate_icon();
    let icon = Icon::from_rgba(rgba, w, h).context("building tray icon")?;

    let tooltip = format!(
        "Whisper FreeFlow v{} — hold hotkey to dictate",
        env!("CARGO_PKG_VERSION")
    );
    let tray = TrayIconBuilder::new()
        .with_tooltip(tooltip)
        .with_menu(Box::new(menu))
        .with_icon(icon)
        .build()
        .context("building tray icon")?;

    tracing::info!("tray icon installed");
    Ok(TrayHandles {
        tray,
        quit_id,
        cancel_id,
        models,
        mics,
        autostart_item,
    })
}

// Renders the Wispr FreeFlow logo mark — five rounded "waveform" bars from
// wispr_freeflow_logo_v3.svg — into a 64×64 RGBA tray icon. The text part of
// the logo is dropped: at tray-icon size the bars alone are the recognisable
// silhouette.
fn generate_icon() -> (Vec<u8>, u32, u32) {
    use tiny_skia::{Color, FillRule, Paint, PathBuilder, Pixmap, Transform};

    const SIZE: u32 = 64;

    // Bars exactly as in the SVG's <g transform="translate(290, 130)"> group —
    // (x, y, width, height). Heights are all 20, fully rounded (rx=10).
    const BARS: [(f32, f32, f32, f32); 5] = [
        (0.0, 0.0, 110.0, 20.0),
        (0.0, 34.0, 50.0, 20.0),
        (0.0, 68.0, 85.0, 20.0),
        (0.0, 102.0, 38.0, 20.0),
        (0.0, 136.0, 25.0, 20.0),
    ];
    const SRC_W: f32 = 110.0;
    const SRC_H: f32 = 156.0; // top of bar 1 (y=0) to bottom of bar 5 (y=156)

    let mut pixmap = Pixmap::new(SIZE, SIZE).expect("pixmap alloc");
    pixmap.fill(Color::TRANSPARENT);

    // Fit the bars inside SIZE×SIZE with a small breathing margin so they
    // don't look squashed against the edge on small tray renders.
    let padding = 6.0;
    let avail = SIZE as f32 - 2.0 * padding;
    let scale = (avail / SRC_W).min(avail / SRC_H);
    let scaled_w = SRC_W * scale;
    let scaled_h = SRC_H * scale;
    let off_x = (SIZE as f32 - scaled_w) / 2.0;
    let off_y = (SIZE as f32 - scaled_h) / 2.0;

    // Cream / off-white from the SVG (#FAF9F5). Full alpha — tray icons read
    // cleanest opaque against varying taskbar backgrounds.
    let mut paint = Paint::default();
    paint.set_color_rgba8(250, 249, 245, 255);
    paint.anti_alias = true;

    for (bx, by, bw, bh) in BARS {
        let x = off_x + bx * scale;
        let y = off_y + by * scale;
        let w = bw * scale;
        let h = bh * scale;
        let r = h / 2.0; // fully rounded ends, matching rx=10 / height=20

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
        if let Some(p) = pb.finish() {
            pixmap.fill_path(&p, &paint, FillRule::Winding, Transform::identity(), None);
        }
    }

    (pixmap.data().to_vec(), SIZE, SIZE)
}
