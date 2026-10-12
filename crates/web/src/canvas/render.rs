//! Canvas2D drawing of the image, marks, ghosts, handles and the region
//! being drawn. It follows the export's mark vocabulary (outline, numbered
//! badge, dashed ghost, translate arrow, mirror-axis glyph) but is not the
//! export: the core renders that (spec §Architecture). Regions and ghosts
//! are drawn from their flattened outlines (a lasso is its spline curve),
//! the same polylines the export strokes.
//!
//! Each edit's outline, badge, ghost, arrow and mirror glyph use the export's
//! color for its badge number ([`mark_css`]). Selection is a neutral
//! two-tone ring outside the white halo: dark ([`SELECTION_RING`]) inside
//! light ([`SELECTION_RING_OUTER`]), so it never depends on the edit's
//! color and shows on light and dark images alike.

use paste2prompt_core::{ImageSize, MirrorAxis, Op, badge_positions};
use wasm_bindgen::JsValue;
use web_sys::CanvasRenderingContext2d;

use super::colors::{SELECTION_RING, SELECTION_RING_OUTER, mark_css};
use super::controller::{Controller, Handle, Preview};
use super::geom::Px;
use super::source::SourceImage;
use super::view::ViewTransform;

const HALO: &str = "rgba(255, 255, 255, 0.95)";
/// Resize, rotate and lasso control-point handles: UI chrome, not an edit,
/// so not a palette color.
const HANDLE: &str = "#0a84ff";
const PREVIEW: &str = "#ff2d55";
const BADGE_RADIUS: f64 = 11.0;
/// Width of the badge's white ring, half of it outside `BADGE_RADIUS`.
const BADGE_RING: f64 = 2.0;
/// The selected badge's extra rings: white, then [`SELECTION_RING`], then
/// [`SELECTION_RING_OUTER`].
const SELECTED_BADGE_GAP: f64 = 2.5;
const SELECTED_BADGE_RING: f64 = 2.5;
const SELECTED_BADGE_OUTER_RING: f64 = 2.0;
const HANDLE_SIZE: f64 = 9.0;
const BADGE_FONT: &str = "bold 12px system-ui, sans-serif";

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
    for (i, edit) in doc.edits().iter().enumerate() {
        let Some(ghost) = edit.ghost(size) else {
            continue;
        };
        let color = mark_css(i + 1);
        let pts = screen_points(view, &ghost.outline_pixels(size));
        dashed(ctx, &[6.0, 4.0])?;
        stroke_path(ctx, &pts, HALO, 4.0);
        stroke_path(ctx, &pts, &color, 2.0);
        dashed(ctx, &[])?;
        if let Op::Translate { .. } = edit.op() {
            let from = view.to_screen(size.to_pixels(edit.region().center()));
            let to = view.to_screen(size.to_pixels(ghost.center()));
            arrow(ctx, from, to, &color);
        }
    }

    // The export's placement rule (core `badge_positions`): outside the
    // region where it fits, so the badge never hides what it labels. The
    // badge is a fixed size on screen, so its size in image pixels depends on
    // the zoom. A named region's badge (`1 logo`) is a wider capsule.
    let per_px = 1.0 / view.scale.max(f64::MIN_POSITIVE);
    let d = 2.0 * BADGE_RADIUS + BADGE_RING;
    let badges = badge_positions(doc, |number, name| {
        let w = badge_width(ctx, &badge_label(number, name)) + BADGE_RING;
        (w * per_px, d * per_px)
    });

    for (i, (edit, placed)) in doc.edits().iter().zip(&badges).enumerate() {
        let pts = screen_points(view, &edit.region().outline_pixels(size));
        let is_selected = selected == Some(edit.id());
        let number = i + 1;
        let color = mark_css(number);
        if is_selected {
            stroke_path(ctx, &pts, SELECTION_RING_OUTER, 12.0);
            stroke_path(ctx, &pts, SELECTION_RING, 9.0);
            stroke_path(ctx, &pts, HALO, 6.0);
            stroke_path(ctx, &pts, &color, 3.0);
        } else {
            stroke_path(ctx, &pts, HALO, 4.0);
            stroke_path(ctx, &pts, &color, 2.0);
        }
        if let Op::Mirror { axis } = edit.op() {
            mirror_glyph(ctx, view, size, edit.region(), *axis, &color)?;
        }
        badge(
            ctx,
            view.to_screen(placed.center),
            &badge_label(number, edit.name()),
            &color,
            is_selected,
        )?;
    }

    if let Some(edit) = ctrl.selected_edit() {
        for (handle, pos) in ctrl.handles() {
            let p = view.to_screen(pos);
            match handle {
                Handle::Corner(_) => {
                    let s = HANDLE_SIZE;
                    ctx.set_fill_style_str("#fff");
                    ctx.fill_rect(p.0 - s / 2.0, p.1 - s / 2.0, s, s);
                    ctx.set_stroke_style_str(HANDLE);
                    ctx.set_line_width(1.5);
                    ctx.stroke_rect(p.0 - s / 2.0, p.1 - s / 2.0, s, s);
                }
                Handle::Control(_) => {
                    // A lasso control point: a small ring on the curve, so
                    // it reads as "drag this point", not a resize corner.
                    ctx.begin_path();
                    ctx.arc(p.0, p.1, HANDLE_SIZE / 2.0, 0.0, std::f64::consts::TAU)?;
                    ctx.set_fill_style_str("#fff");
                    ctx.fill();
                    ctx.set_stroke_style_str(HANDLE);
                    ctx.set_line_width(2.0);
                    ctx.stroke();
                }
                Handle::Rotate => {
                    let anchor = match edit.op() {
                        Op::Rotate { anchor, .. } => anchor.resolve(edit.region()),
                        _ => edit.region().center(),
                    };
                    let a = view.to_screen(size.to_pixels(anchor));
                    dashed(ctx, &[3.0, 3.0])?;
                    stroke_path_open(ctx, &[a, p], HANDLE, 1.0);
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
                    ctx.set_stroke_style_str(HANDLE);
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

fn arrow(ctx: &CanvasRenderingContext2d, from: Px, to: Px, color: &str) {
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
    for (color, width) in [(HALO, 4.0), (color, 2.0)] {
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
    color: &str,
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
    stroke_path_open(ctx, &line, color, 2.0);
    dashed(ctx, &[])?;
    arrow(ctx, c, flip[0], color);
    arrow(ctx, c, flip[1], color);
    Ok(())
}

/// The badge text: the number, then the region's name if it has one
/// (`1 logo`), as on the export's badge.
fn badge_label(number: usize, name: Option<&str>) -> String {
    match name {
        Some(name) => format!("{number} {name}"),
        None => number.to_string(),
    }
}

/// The badge's width on screen (CSS pixels, its ring excluded): a circle
/// for a short number, a capsule around a longer label.
fn badge_width(ctx: &CanvasRenderingContext2d, label: &str) -> f64 {
    ctx.set_font(BADGE_FONT);
    let text = ctx.measure_text(label).map_or(0.0, |m| m.width());
    (text + BADGE_RADIUS * 1.2).max(2.0 * BADGE_RADIUS)
}

/// Traces a horizontal capsule of radius `r` around `at`, its two end
/// circles `half` either side of center (a circle when `half` is 0).
fn capsule(ctx: &CanvasRenderingContext2d, at: Px, half: f64, r: f64) -> Result<(), JsValue> {
    use std::f64::consts::FRAC_PI_2;
    ctx.begin_path();
    ctx.arc(at.0 + half, at.1, r, -FRAC_PI_2, FRAC_PI_2)?;
    ctx.arc(at.0 - half, at.1, r, FRAC_PI_2, 3.0 * FRAC_PI_2)?;
    ctx.close_path();
    Ok(())
}

fn badge(
    ctx: &CanvasRenderingContext2d,
    at: Px,
    label: &str,
    color: &str,
    selected: bool,
) -> Result<(), JsValue> {
    let half = (badge_width(ctx, label) / 2.0 - BADGE_RADIUS).max(0.0);
    if selected {
        // A light ring outside a dark ring outside a white gap, outside the
        // badge's own ring: one of the two rings contrasts with any image.
        let r = BADGE_RADIUS + BADGE_RING / 2.0;
        let dark = r + SELECTED_BADGE_GAP + SELECTED_BADGE_RING;
        for (radius, fill) in [
            (dark + SELECTED_BADGE_OUTER_RING, SELECTION_RING_OUTER),
            (dark, SELECTION_RING),
            (r + SELECTED_BADGE_GAP, "#fff"),
        ] {
            capsule(ctx, at, half, radius)?;
            ctx.set_fill_style_str(fill);
            ctx.fill();
        }
    }
    capsule(ctx, at, half, BADGE_RADIUS)?;
    ctx.set_fill_style_str(color);
    ctx.fill();
    ctx.set_stroke_style_str("#fff");
    ctx.set_line_width(BADGE_RING);
    ctx.stroke();
    ctx.set_fill_style_str("#fff");
    ctx.set_font(BADGE_FONT);
    ctx.set_text_align("center");
    ctx.set_text_baseline("middle");
    ctx.fill_text(label, at.0, at.1 + 0.5)
}
