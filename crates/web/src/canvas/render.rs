//! Canvas2D drawing of the image, marks, ghosts, handles and the region
//! being drawn. It follows the export's mark vocabulary (outline, numbered
//! badge, dashed ghost, translate arrow, mirror-axis glyph) but is not the
//! export: the core renders that (spec §Architecture).

use paste2prompt_core::{ImageSize, MirrorAxis, Op};
use wasm_bindgen::JsValue;
use web_sys::CanvasRenderingContext2d;

use super::controller::{Controller, Handle, Preview};
use super::geom::Px;
use super::source::SourceImage;
use super::view::ViewTransform;

const MARK: &str = "#ff2d55";
const HALO: &str = "rgba(255, 255, 255, 0.95)";
const GHOST: &str = "#0a84ff";
const SELECTED: &str = "#ffcc00";
const PREVIEW: &str = "#ff2d55";
const BADGE_RADIUS: f64 = 11.0;
const HANDLE_SIZE: f64 = 9.0;

/// Draws one frame. `ctx` is already scaled so one unit is one CSS pixel.
pub fn draw(
    ctx: &CanvasRenderingContext2d,
    css_w: f64,
    css_h: f64,
    image: &SourceImage,
    view: &ViewTransform,
    ctrl: &Controller,
) -> Result<(), JsValue> {
    ctx.clear_rect(0.0, 0.0, css_w, css_h);
    let Some(size) = image.size() else {
        return Ok(());
    };
    let (iw, ih) = (f64::from(size.width()), f64::from(size.height()));
    image.draw(
        ctx,
        view.offset_x,
        view.offset_y,
        iw * view.scale,
        ih * view.scale,
    )?;
    let Some(editor) = ctrl.editor() else {
        return Ok(());
    };
    let doc = editor.document();
    let selected = ctrl.selected();
    ctx.set_line_join("round");

    // Ghosts first, so source outlines and badges stay on top.
    for edit in doc.edits() {
        let Some(ghost) = edit.ghost(size) else {
            continue;
        };
        let pts = screen_points(view, &ghost.outline_pixels(size));
        dashed(ctx, &[6.0, 4.0])?;
        stroke_path(ctx, &pts, HALO, 4.0);
        stroke_path(ctx, &pts, GHOST, 2.0);
        dashed(ctx, &[])?;
        if let Op::Translate { .. } = edit.op() {
            let from = view.to_screen(size.to_pixels(edit.region().center()));
            let to = view.to_screen(size.to_pixels(ghost.center()));
            arrow(ctx, from, to);
        }
    }

    for (i, edit) in doc.edits().iter().enumerate() {
        let pts = screen_points(view, &edit.region().outline_pixels(size));
        let is_selected = selected == Some(edit.id());
        stroke_path(ctx, &pts, HALO, if is_selected { 6.0 } else { 4.0 });
        stroke_path(
            ctx,
            &pts,
            if is_selected { SELECTED } else { MARK },
            if is_selected { 3.0 } else { 2.0 },
        );
        if let Op::Mirror { axis } = edit.op() {
            mirror_glyph(ctx, view, size, edit.region(), *axis)?;
        }
        let corner = view.to_screen(size.to_pixels(edit.region().bounds().min()));
        badge(ctx, corner, i + 1, is_selected)?;
    }

    if let Some(edit) = ctrl.selected_edit() {
        for (handle, pos) in ctrl.handles() {
            let p = view.to_screen(pos);
            match handle {
                Handle::Corner(_) => {
                    let s = HANDLE_SIZE;
                    ctx.set_fill_style_str("#fff");
                    ctx.fill_rect(p.0 - s / 2.0, p.1 - s / 2.0, s, s);
                    ctx.set_stroke_style_str(GHOST);
                    ctx.set_line_width(1.5);
                    ctx.stroke_rect(p.0 - s / 2.0, p.1 - s / 2.0, s, s);
                }
                Handle::Rotate => {
                    let anchor = match edit.op() {
                        Op::Rotate { anchor, .. } => anchor.resolve(edit.region()),
                        _ => edit.region().center(),
                    };
                    let a = view.to_screen(size.to_pixels(anchor));
                    dashed(ctx, &[3.0, 3.0])?;
                    stroke_path_open(ctx, &[a, p], GHOST, 1.0);
                    dashed(ctx, &[])?;
                    ctx.begin_path();
                    ctx.arc(
                        p.0,
                        p.1,
                        HANDLE_SIZE / 2.0 + 1.0,
                        0.0,
                        std::f64::consts::TAU,
                    )?;
                    ctx.set_fill_style_str("#fff");
                    ctx.fill();
                    ctx.set_stroke_style_str(GHOST);
                    ctx.set_line_width(1.5);
                    ctx.stroke();
                }
            }
        }
    }

    match ctrl.preview() {
        Some(Preview::Rect(a, b)) => {
            let (a, b) = (view.to_screen(a), view.to_screen(b));
            let pts = [a, (b.0, a.1), b, (a.0, b.1)];
            dashed(ctx, &[4.0, 3.0])?;
            stroke_path(ctx, &pts, HALO, 3.0);
            stroke_path(ctx, &pts, PREVIEW, 1.5);
            dashed(ctx, &[])?;
        }
        Some(Preview::Lasso(points)) => {
            let pts = screen_points(view, &points);
            stroke_path_open(ctx, &pts, HALO, 3.0);
            stroke_path_open(ctx, &pts, PREVIEW, 1.5);
        }
        None => {}
    }
    Ok(())
}

fn screen_points(view: &ViewTransform, pts: &[Px]) -> Vec<Px> {
    pts.iter().map(|&p| view.to_screen(p)).collect()
}

fn dashed(ctx: &CanvasRenderingContext2d, pattern: &[f64]) -> Result<(), JsValue> {
    let arr = js_sys::Array::new();
    for &v in pattern {
        arr.push(&JsValue::from_f64(v));
    }
    ctx.set_line_dash(&arr)
}

fn trace(ctx: &CanvasRenderingContext2d, pts: &[Px]) {
    ctx.begin_path();
    if let Some((&first, rest)) = pts.split_first() {
        ctx.move_to(first.0, first.1);
        for p in rest {
            ctx.line_to(p.0, p.1);
        }
    }
}

fn stroke_path(ctx: &CanvasRenderingContext2d, pts: &[Px], color: &str, width: f64) {
    trace(ctx, pts);
    ctx.close_path();
    ctx.set_stroke_style_str(color);
    ctx.set_line_width(width);
    ctx.stroke();
}

fn stroke_path_open(ctx: &CanvasRenderingContext2d, pts: &[Px], color: &str, width: f64) {
    trace(ctx, pts);
    ctx.set_stroke_style_str(color);
    ctx.set_line_width(width);
    ctx.stroke();
}

fn arrow(ctx: &CanvasRenderingContext2d, from: Px, to: Px) {
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    let len = dx.hypot(dy);
    if len < 4.0 {
        return;
    }
    let (ux, uy) = (dx / len, dy / len);
    let head = 10.0_f64.min(len / 2.0);
    let wing = |s: f64| {
        (
            to.0 - ux * head - uy * head * 0.5 * s,
            to.1 - uy * head + ux * head * 0.5 * s,
        )
    };
    for (color, width) in [(HALO, 4.0), (GHOST, 2.0)] {
        stroke_path_open(ctx, &[from, to], color, width);
        stroke_path_open(ctx, &[wing(1.0), to, wing(-1.0)], color, width);
    }
}

fn mirror_glyph(
    ctx: &CanvasRenderingContext2d,
    view: &ViewTransform,
    size: ImageSize,
    region: &paste2prompt_core::Region,
    axis: MirrorAxis,
) -> Result<(), JsValue> {
    let b = region.bounds();
    let min = view.to_screen(size.to_pixels(b.min()));
    let max = view.to_screen(size.to_pixels(b.max()));
    let c = ((min.0 + max.0) / 2.0, (min.1 + max.1) / 2.0);
    let ext = 8.0;
    // The axis line, plus a small double arrow across it showing the flip.
    let (line, flip) = match axis {
        MirrorAxis::Horizontal => (
            [(c.0, min.1 - ext), (c.0, max.1 + ext)],
            [(c.0 - 12.0, c.1), (c.0 + 12.0, c.1)],
        ),
        MirrorAxis::Vertical => (
            [(min.0 - ext, c.1), (max.0 + ext, c.1)],
            [(c.0, c.1 - 12.0), (c.0, c.1 + 12.0)],
        ),
    };
    dashed(ctx, &[5.0, 3.0])?;
    stroke_path_open(ctx, &line, HALO, 4.0);
    stroke_path_open(ctx, &line, GHOST, 2.0);
    dashed(ctx, &[])?;
    arrow(ctx, c, flip[0]);
    arrow(ctx, c, flip[1]);
    Ok(())
}

fn badge(
    ctx: &CanvasRenderingContext2d,
    at: Px,
    number: usize,
    selected: bool,
) -> Result<(), JsValue> {
    ctx.begin_path();
    ctx.arc(at.0, at.1, BADGE_RADIUS, 0.0, std::f64::consts::TAU)?;
    ctx.set_fill_style_str(if selected { SELECTED } else { MARK });
    ctx.fill();
    ctx.set_stroke_style_str("#fff");
    ctx.set_line_width(2.0);
    ctx.stroke();
    ctx.set_fill_style_str(if selected { "#000" } else { "#fff" });
    ctx.set_font("bold 12px system-ui, sans-serif");
    ctx.set_text_align("center");
    ctx.set_text_baseline("middle");
    ctx.fill_text(&number.to_string(), at.0, at.1 + 0.5)
}
