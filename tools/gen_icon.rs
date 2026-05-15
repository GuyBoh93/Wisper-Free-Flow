// Renders the Wispr FreeFlow logo (five rounded bars) to assets/icon.png at
// 1024×1024. cargo-packager picks this up via package.metadata.packager.icons
// and converts to .ico on Windows and .icns on macOS at packaging time.
//
//   cargo run --example gen_icon
//
// Re-run whenever the logo geometry changes. The PNG is committed to the
// repo so users building the installer don't need to regenerate it.

use std::fs;
use std::path::PathBuf;
use tiny_skia::{Color, FillRule, Paint, PathBuilder, Pixmap, Transform};

// Bars exactly as in wispr_freeflow_logo_v3.svg
// <g transform="translate(290, 130)">.
const BARS: [(f32, f32, f32, f32); 5] = [
    (0.0, 0.0, 110.0, 20.0),
    (0.0, 34.0, 50.0, 20.0),
    (0.0, 68.0, 85.0, 20.0),
    (0.0, 102.0, 38.0, 20.0),
    (0.0, 136.0, 25.0, 20.0),
];
const SRC_W: f32 = 110.0;
const SRC_H: f32 = 156.0;

fn rounded_rect(x: f32, y: f32, w: f32, h: f32, r: f32) -> Option<tiny_skia::Path> {
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

fn render(size: u32) -> Vec<u8> {
    let s = size as f32;
    let mut pixmap = Pixmap::new(size, size).expect("alloc");
    pixmap.fill(Color::TRANSPARENT);

    // Dark rounded-square background. The corner radius (~22 % of side) gives
    // the icon a recognisably modern macOS / Win11 silhouette.
    let bg_radius = s * 0.22;
    let mut bg = Paint::default();
    bg.set_color_rgba8(18, 20, 28, 255);
    bg.anti_alias = true;
    if let Some(p) = rounded_rect(0.0, 0.0, s, s, bg_radius) {
        pixmap.fill_path(&p, &bg, FillRule::Winding, Transform::identity(), None);
    }

    // Bars in cream (#FAF9F5), centred with a generous breathing margin so
    // the icon reads clearly at small sizes (16×16, 32×32 in Finder lists).
    let padding = s * 0.22;
    let avail = s - 2.0 * padding;
    let scale = (avail / SRC_W).min(avail / SRC_H);
    let scaled_w = SRC_W * scale;
    let scaled_h = SRC_H * scale;
    let off_x = (s - scaled_w) / 2.0;
    let off_y = (s - scaled_h) / 2.0;

    let mut fg = Paint::default();
    fg.set_color_rgba8(250, 249, 245, 255);
    fg.anti_alias = true;

    for (bx, by, bw, bh) in BARS {
        let x = off_x + bx * scale;
        let y = off_y + by * scale;
        let w = bw * scale;
        let h = bh * scale;
        let r = h / 2.0;
        if let Some(p) = rounded_rect(x, y, w, h, r) {
            pixmap.fill_path(&p, &fg, FillRule::Winding, Transform::identity(), None);
        }
    }

    pixmap.encode_png().expect("encode png")
}

fn main() -> std::io::Result<()> {
    let assets = PathBuf::from("assets");
    fs::create_dir_all(&assets)?;

    // 1024 is the source of truth — cargo-packager downsamples for every
    // size the platform needs (16, 32, 48, 128, 256, 512, 1024).
    let icon_path = assets.join("icon.png");
    fs::write(&icon_path, render(1024))?;
    println!("wrote {} (1024x1024)", icon_path.display());

    Ok(())
}
