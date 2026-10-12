//! Renders the example exports in `docs/examples/` from a synthetic UI mock.
//!
//! ```text
//! cargo run -p paste2prompt-core --example export_demo -- docs/examples
//! ```

use std::path::PathBuf;

use paste2prompt_core::{
    Anchor, Editor, ImageSize, MirrorAxis, Op, Point, Region, RgbaImage, render_export,
};
use tiny_skia::{Color, FillRule, Paint, PathBuilder, Pixmap, Rect, Transform};

fn fill(p: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, rgb: [u8; 3]) {
    let mut paint = Paint::default();
    paint.set_color(Color::from_rgba8(rgb[0], rgb[1], rgb[2], 255));
    paint.anti_alias = true;
    if let Some(r) = Rect::from_xywh(x, y, w, h) {
        p.fill_rect(r, &paint, Transform::identity(), None);
    }
}

fn circle(p: &mut Pixmap, cx: f32, cy: f32, r: f32, rgb: [u8; 3]) {
    let mut paint = Paint::default();
    paint.set_color(Color::from_rgba8(rgb[0], rgb[1], rgb[2], 255));
    paint.anti_alias = true;
    if let Some(path) = PathBuilder::from_circle(cx, cy, r) {
        p.fill_path(
            &path,
            &paint,
            FillRule::Winding,
            Transform::identity(),
            None,
        );
    }
}

/// A 1280×720 mock of a landing page: nav bar, hero, three cards, a button.
fn mock_page() -> RgbaImage {
    let (w, h) = (1280, 720);
    let mut p = Pixmap::new(w, h).unwrap();
    p.fill(Color::from_rgba8(248, 249, 251, 255));
    fill(&mut p, 0.0, 0.0, 1280.0, 72.0, [30, 34, 44]);
    circle(&mut p, 48.0, 36.0, 16.0, [255, 196, 0]);
    for i in 0..4 {
        fill(
            &mut p,
            820.0 + i as f32 * 110.0,
            30.0,
            80.0,
            12.0,
            [150, 158, 172],
        );
    }
    fill(&mut p, 96.0, 128.0, 560.0, 40.0, [40, 44, 56]);
    fill(&mut p, 96.0, 188.0, 460.0, 16.0, [150, 158, 172]);
    fill(&mut p, 96.0, 216.0, 400.0, 16.0, [150, 158, 172]);
    fill(&mut p, 96.0, 262.0, 180.0, 52.0, [52, 120, 246]);
    fill(&mut p, 760.0, 120.0, 420.0, 220.0, [214, 226, 250]);
    circle(&mut p, 970.0, 230.0, 70.0, [120, 160, 240]);
    for i in 0..3 {
        let x = 96.0 + i as f32 * 372.0;
        fill(&mut p, x, 400.0, 340.0, 250.0, [255, 255, 255]);
        fill(&mut p, x, 400.0, 340.0, 4.0, [226, 230, 236]);
        circle(
            &mut p,
            x + 44.0,
            448.0,
            22.0,
            [[52, 120, 246], [0, 168, 120], [240, 120, 40]][i],
        );
        fill(&mut p, x + 24.0, 494.0, 220.0, 18.0, [40, 44, 56]);
        for j in 0..4 {
            fill(
                &mut p,
                x + 24.0,
                530.0 + j as f32 * 26.0,
                280.0 - j as f32 * 30.0,
                12.0,
                [170, 176, 188],
            );
        }
    }
    // Pixmap data is premultiplied, but every pixel is opaque, so it's also
    // straight RGBA.
    RgbaImage {
        width: w,
        height: h,
        pixels: p.data().to_vec(),
    }
}

fn pt(x: f64, y: f64) -> Point {
    Point::new(x, y)
}

fn main() {
    let out = PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "docs/examples".into()),
    );
    std::fs::create_dir_all(&out).unwrap();
    let image = mock_page();
    let size = ImageSize::new(image.width, image.height).unwrap();

    // Example 1: the spec's flavor of edit list.
    let mut ed = Editor::new(size);
    let button = Region::rect_from_pixels(size, (96.0, 262.0), (276.0, 314.0)).unwrap();
    ed.add_edit(
        button,
        Op::Translate { dx: 0.0, dy: 0.08 },
        Some("and make it green".into()),
    )
    .unwrap();
    let art = Region::rect_from_pixels(size, (760.0, 120.0), (1180.0, 340.0)).unwrap();
    ed.add_edit(art, Op::uniform_scale(0.75), None).unwrap();
    let icon = Region::polygon([
        pt(0.05, 0.585),
        pt(0.11, 0.57),
        pt(0.12, 0.66),
        pt(0.05, 0.665),
    ])
    .unwrap();
    ed.add_edit(
        icon,
        Op::Rotate {
            degrees: 45.0,
            anchor: Anchor::Center,
        },
        None,
    )
    .unwrap();
    let card = Region::rect_from_pixels(size, (840.0, 400.0), (1180.0, 650.0)).unwrap();
    ed.add_edit(
        card,
        Op::Hint,
        Some("replace this card with a pricing table".into()),
    )
    .unwrap();
    ed.set_instructions("tighten the spacing between the cards");
    let png = render_export(&image, ed.document()).unwrap();
    std::fs::write(out.join("export-landing.png"), png).unwrap();

    // Example 2: a phone-width crop (padded to the minimum width) with a
    // mirror and a move.
    let crop_w = 390u32;
    let mut pixels = Vec::new();
    for row in image.pixels.chunks_exact(image.width as usize * 4) {
        pixels.extend_from_slice(&row[..crop_w as usize * 4]);
    }
    let narrow = RgbaImage {
        width: crop_w,
        height: image.height,
        pixels,
    };
    let nsize = ImageSize::new(narrow.width, narrow.height).unwrap();
    let mut ed = Editor::new(nsize);
    let logo = Region::rect_from_pixels(nsize, (24.0, 14.0), (72.0, 58.0)).unwrap();
    ed.add_edit(
        logo,
        Op::Mirror {
            axis: MirrorAxis::Horizontal,
        },
        None,
    )
    .unwrap();
    let title = Region::rect_from_pixels(nsize, (96.0, 128.0), (390.0, 168.0)).unwrap();
    ed.add_edit(title, Op::Translate { dx: -0.15, dy: 0.0 }, None)
        .unwrap();
    let png = render_export(&narrow, ed.document()).unwrap();
    std::fs::write(out.join("export-narrow.png"), png).unwrap();
}
