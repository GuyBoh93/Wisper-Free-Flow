// Renders the NoMic overlay glyph (mic capsule + U-stand + red diagonal slash)
// to Docs/no-mic-preview.png at 4x scale. Mirrors draw_no_mic() in
// src/overlay.rs — keep in sync if that geometry changes.
//
//   cargo run --example gen_no_mic_preview

use std::path::PathBuf;
use tiny_skia::{Color, FillRule, Paint, PathBuilder, Pixmap, Stroke, Transform};

const WIDTH: i32 = 140;
const HEIGHT: i32 = 44;
const SCALE: i32 = 4;

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

fn main() {
    let w = (WIDTH * SCALE) as u32;
    let h = (HEIGHT * SCALE) as u32;
    let mut pm = Pixmap::new(w, h).unwrap();
    pm.fill(Color::TRANSPARENT);
    let scale = Transform::from_scale(SCALE as f32, SCALE as f32);

    // Pill body.
    if let Some(p) = rounded_rect_path(0.0, 0.0, WIDTH as f32, HEIGHT as f32, HEIGHT as f32 / 2.0) {
        let mut bg = Paint::default();
        bg.set_color_rgba8(18, 20, 28, 222);
        bg.anti_alias = true;
        pm.fill_path(&p, &bg, FillRule::Winding, scale, None);
    }
    if let Some(p) = rounded_rect_path(0.5, 0.5, WIDTH as f32 - 1.0, HEIGHT as f32 - 1.0, (HEIGHT as f32 - 1.0) / 2.0) {
        let mut paint = Paint::default();
        paint.set_color_rgba8(255, 255, 255, 26);
        paint.anti_alias = true;
        pm.stroke_path(&p, &paint, &Stroke { width: 1.0, ..Default::default() }, scale, None);
    }

    let mid_x = WIDTH as f32 / 2.0;
    let mid_y = HEIGHT as f32 / 2.0;

    // Mic body capsule.
    let body_w: f32 = 12.0;
    let body_h: f32 = 22.0;
    let bx = mid_x - body_w / 2.0;
    let by = mid_y - body_h / 2.0 - 2.0;
    if let Some(p) = rounded_rect_path(bx, by, body_w, body_h, body_w / 2.0) {
        let mut paint = Paint::default();
        paint.set_color_rgba8(220, 226, 240, 235);
        paint.anti_alias = true;
        pm.fill_path(&p, &paint, FillRule::Winding, scale, None);
    }

    // U-stand arc.
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
        pm.stroke_path(&path, &paint, &Stroke { width: 2.0, ..Default::default() }, scale, None);
    }

    // Stand base bar.
    let base_w: f32 = 14.0;
    let base_h: f32 = 2.5;
    if let Some(p) = rounded_rect_path(mid_x - base_w / 2.0, arc_y + arc_h + 2.0, base_w, base_h, base_h / 2.0) {
        let mut paint = Paint::default();
        paint.set_color_rgba8(220, 226, 240, 235);
        paint.anti_alias = true;
        pm.fill_path(&p, &paint, FillRule::Winding, scale, None);
    }

    // Red slash.
    let slash_len: f32 = 36.0;
    let slash_w: f32 = 4.0;
    let angle = -45f32.to_radians();
    let cos = angle.cos();
    let sin = angle.sin();
    if let Some(p) = rounded_rect_path(-slash_len / 2.0, -slash_w / 2.0, slash_len, slash_w, slash_w / 2.0) {
        let s = SCALE as f32;
        let transform = Transform::from_row(cos * s, sin * s, -sin * s, cos * s, mid_x * s, mid_y * s);
        let mut paint = Paint::default();
        paint.set_color_rgba8(255, 80, 90, 255);
        paint.anti_alias = true;
        pm.fill_path(&p, &paint, FillRule::Winding, transform, None);
    }

    let out = PathBuf::from("Docs/no-mic-preview.png");
    pm.save_png(&out).unwrap();
    println!("wrote {}", out.display());
}
