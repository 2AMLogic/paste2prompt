//! The marks drawn over the image: region outlines, numbered badges and
//! destination ghosts.
//!
//! Every mark is two strokes: a wider white halo underneath, then the edit's
//! color on top, so it reads on light, dark and busy backgrounds alike.

use tiny_skia::{
    Color, FillRule, LineCap, LineJoin, Mask, Paint, Path, PathBuilder, Pixmap, Stroke, StrokeDash,
    Transform,
};

use super::text::{Style, Text};
use crate::{Edit, ImageSize, MirrorAxis, Op, Region};

const WHITE: [u8; 3] = [255, 255, 255];

/// Ghost coordinates are clamped to this many pixels outside the image, so a
/// far-off-image ghost can't push the rasterizer into float trouble. Anything
/// that far out is clipped away anyway.
const FAR: f64 = 1.0e6;

/// Sizes of every mark, derived from the image so marks stay legible after a
/// chat app downscales the export, with floors for small images.
#[derive(Debug, Clone, Copy)]
pub(super) struct MarkScale {
    /// Width of the colored outline stroke.
    pub stroke: f32,
    /// Width of the white halo under it.
    pub halo: f32,
    /// Height (and minimum width) of a numbered badge.
    pub badge: f32,
}

impl MarkScale {
    pub fn for_image(size: ImageSize) -> Self {
        // The geometric mean of the sides, so a long thin image doesn't get
        // giant badges along its short side.
        let k = (f64::from(size.width()) * f64::from(size.height())).sqrt() as f32;
        let stroke = (k * 0.0035).max(3.0);
        let halo = stroke + 2.0 * (stroke * 0.6).max(1.5);
        let badge = (k * 0.045).max(28.0);
        Self {
            stroke,
            halo,
            badge,
        }
    }

    /// Width of the ring around a solid (source) badge.
    pub fn solid_ring(self) -> f32 {
        (self.halo - self.stroke) / 2.0 + 1.0
    }

    /// The full (width, height) of edit `number`'s solid badge (with its
    /// region's `name`, if any), ring included, in image pixels: what
    /// [`super::badge_positions`] places.
    pub fn badge_extent(self, text: &mut Text, number: usize, name: Option<&str>) -> (f64, f64) {
        let ring = 2.0 * self.solid_ring();
        let w = badge_width(text, &badge_label(number, name), self.badge) + ring;
        (f64::from(w), f64::from(self.badge + ring))
    }
}

/// Draws onto the image area of the export canvas.
pub(super) struct Marks<'a> {
    pub pixmap: &'a mut Pixmap,
    pub text: &'a mut Text,
    /// Clips every mark to the image, so ghosts never spill onto the
    /// padding or the caption band.
    pub clip: &'a Mask,
    /// Where the image's top-left sits on the canvas.
    pub origin: (f32, f32),
    pub size: ImageSize,
    pub scale: MarkScale,
}

impl Marks<'_> {
    /// Draws everything for one edit: ghost first (so the source outline and
    /// badge stay on top), then the region outline, then the badge centered
    /// on `badge` (image pixels, from [`super::badge_positions`]).
    pub fn draw_edit(&mut self, number: usize, edit: &Edit, rgb: [u8; 3], badge: (f64, f64)) {
        let outline = edit.region().outline_pixels(self.size);
        let source = self.path(&outline, true);

        if let Some(ghost) = edit.ghost(self.size) {
            let points = ghost.outline_pixels(self.size);
            // A mirrored rectangle lands exactly on itself; its dashed ghost
            // would only fray the solid outline, so the axis glyph says it.
            let on_itself =
                matches!(edit.op(), Op::Mirror { .. }) && matches!(edit.region(), Region::Rect(_));
            if !on_itself && let Some(path) = self.path(&points, true) {
                self.dashed(&path, rgb);
            }
            match *edit.op() {
                Op::Translate { .. } => {
                    let from = self.size.to_pixels(edit.region().center());
                    let to = self.size.to_pixels(ghost.center());
                    self.arrow(from, to, rgb);
                    // A hollow badge at the destination, unless it would be
                    // off the image, or the move is short (under two badges,
                    // where the ghost badge would sit on the source outline),
                    // or it would crowd the source badge, which may now sit
                    // outside its region (`badge_positions`). 1.5 badges
                    // between centers leaves a gap even for a two-digit
                    // capsule next to a circle.
                    let (gx, gy) = bounds_min(&points);
                    let (cx, cy) = self.size.to_pixels(edit.region().bounds().min());
                    let (sx, sy) = badge;
                    let (iw, ih) = (f64::from(self.size.width()), f64::from(self.size.height()));
                    let b = f64::from(self.scale.badge);
                    let clear =
                        (gx - cx).hypot(gy - cy) >= 2.0 * b && (gx - sx).hypot(gy - sy) >= 1.5 * b;
                    if clear && (0.0..iw).contains(&gx) && (0.0..ih).contains(&gy) {
                        self.badge(number, None, (gx, gy), rgb, true);
                    }
                }
                Op::Mirror { axis } => {
                    let b = edit.region().bounds();
                    let (x0, y0) = self.size.to_pixels(b.min());
                    let (x1, y1) = self.size.to_pixels(b.max());
                    self.mirror_glyph(axis, (x0, y0, x1, y1), rgb);
                }
                _ => {}
            }
        }

        if let Some(path) = source {
            self.solid(&path, rgb);
        }
        // The source badge carries the region's name (`1 logo`); the
        // hollow destination badge keeps just the number.
        self.badge(number, edit.name(), badge, rgb, false);
    }

    fn paint(rgb: [u8; 3]) -> Paint<'static> {
        let mut paint = Paint::default();
        paint.set_color(Color::from_rgba8(rgb[0], rgb[1], rgb[2], 255));
        paint.anti_alias = true;
        paint
    }

    fn stroke(width: f32, dash: Option<StrokeDash>) -> Stroke {
        Stroke {
            width,
            line_cap: LineCap::Round,
            line_join: LineJoin::Round,
            dash,
            ..Stroke::default()
        }
    }

    /// A path through image-pixel points, moved onto the canvas.
    fn path(&self, points: &[(f64, f64)], close: bool) -> Option<Path> {
        let (ox, oy) = self.origin;
        let mut pb = PathBuilder::new();
        for (i, &(x, y)) in points.iter().enumerate() {
            let x = x.clamp(-FAR, FAR) as f32 + ox;
            let y = y.clamp(-FAR, FAR) as f32 + oy;
            if i == 0 {
                pb.move_to(x, y);
            } else {
                pb.line_to(x, y);
            }
        }
        if close {
            pb.close();
        }
        pb.finish()
    }

    fn stroke_path(&mut self, path: &Path, rgb: [u8; 3], stroke: &Stroke) {
        self.pixmap.stroke_path(
            path,
            &Self::paint(rgb),
            stroke,
            Transform::identity(),
            Some(self.clip),
        );
    }

    fn fill_path(&mut self, path: &Path, rgb: [u8; 3]) {
        self.pixmap.fill_path(
            path,
            &Self::paint(rgb),
            FillRule::Winding,
            Transform::identity(),
            Some(self.clip),
        );
    }

    /// A solid outline with its halo.
    fn solid(&mut self, path: &Path, rgb: [u8; 3]) {
        let s = self.scale;
        self.stroke_path(path, WHITE, &Self::stroke(s.halo, None));
        self.stroke_path(path, rgb, &Self::stroke(s.stroke, None));
    }

    /// A dashed outline with a dashed halo: a ghost, visibly not a region.
    fn dashed(&mut self, path: &Path, rgb: [u8; 3]) {
        let s = self.scale;
        let dash = || StrokeDash::new(vec![s.stroke * 4.0, s.stroke * 3.0], 0.0);
        self.stroke_path(path, WHITE, &Self::stroke(s.halo, dash()));
        self.stroke_path(path, rgb, &Self::stroke(s.stroke, dash()));
    }

    /// An arrow from the source region's center to the ghost's center. Skipped
    /// when the move is shorter than a badge, where it would only be a blob.
    fn arrow(&mut self, from: (f64, f64), to: (f64, f64), rgb: [u8; 3]) {
        let s = self.scale;
        let (dx, dy) = (to.0 - from.0, to.1 - from.1);
        let len = dx.hypot(dy);
        if !len.is_finite() || len < f64::from(s.badge) {
            return;
        }
        let (ux, uy) = (dx / len, dy / len);
        let head = f64::from(s.stroke * 4.5).min(len / 2.0);
        // The shaft stops inside the head so its round cap doesn't poke out.
        let shaft_end = (to.0 - ux * head * 0.6, to.1 - uy * head * 0.6);
        if let Some(shaft) = self.path(&[from, shaft_end], false) {
            self.stroke_path(&shaft, WHITE, &Self::stroke(s.halo, None));
            self.stroke_path(&shaft, rgb, &Self::stroke(s.stroke, None));
        }
        let (bx, by) = (to.0 - ux * head, to.1 - uy * head);
        let (px, py) = (-uy * head * 0.6, ux * head * 0.6);
        let tri = [to, (bx + px, by + py), (bx - px, by - py)];
        if let Some(path) = self.path(&tri, true) {
            let halo = Self::stroke(s.halo - s.stroke, None);
            self.stroke_path(&path, WHITE, &halo);
            self.fill_path(&path, rgb);
        }
    }

    /// The mirror axis through the region's center, extended past its
    /// bounds, with a pair of opposed arrowheads (◀|▶ or ▲/▼) at the far end
    /// showing the flip direction.
    fn mirror_glyph(
        &mut self,
        axis: MirrorAxis,
        (x0, y0, x1, y1): (f64, f64, f64, f64),
        rgb: [u8; 3],
    ) {
        let s = self.scale;
        let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
        let ext = f64::from(s.badge) * 0.8;
        let tri = f64::from(s.badge) * 0.32;
        let gap = f64::from(s.stroke) * 1.2;
        // (axis line ends, glyph center, unit vector across the axis)
        let (a, b, glyph, across) = match axis {
            // Left-right flip: the axis is a vertical line.
            MirrorAxis::Horizontal => (
                (cx, y0 - ext * 0.5),
                (cx, y1 + ext),
                (cx, y1 + ext * 0.55),
                (1.0, 0.0),
            ),
            MirrorAxis::Vertical => (
                (x0 - ext * 0.5, cy),
                (x1 + ext, cy),
                (x1 + ext * 0.55, cy),
                (0.0, 1.0),
            ),
        };
        let dash = StrokeDash::new(vec![s.stroke * 2.0, s.stroke * 2.0], 0.0);
        if let Some(line) = self.path(&[a, b], false) {
            self.stroke_path(&line, WHITE, &Self::stroke(s.halo, dash.clone()));
            self.stroke_path(&line, rgb, &Self::stroke(s.stroke, dash));
        }
        // Two triangles pointing away from the axis on either side of it.
        let along = (across.1, across.0);
        for sign in [-1.0, 1.0] {
            let base = (
                glyph.0 + across.0 * gap * sign,
                glyph.1 + across.1 * gap * sign,
            );
            let tip = (
                base.0 + across.0 * tri * sign,
                base.1 + across.1 * tri * sign,
            );
            let w = tri * 0.75;
            let pts = [
                tip,
                (base.0 + along.0 * w, base.1 + along.1 * w),
                (base.0 - along.0 * w, base.1 - along.1 * w),
            ];
            if let Some(path) = self.path(&pts, true) {
                self.stroke_path(&path, WHITE, &Self::stroke(s.halo - s.stroke, None));
                self.fill_path(&path, rgb);
            }
        }
    }

    /// A numbered badge centered on `corner` (image pixels), nudged so it
    /// stays wholly inside the image (a no-op for a source badge, which
    /// [`super::badge_positions`] already placed inside). A ghost badge is hollow (white with a
    /// colored ring and number) so it can't be mistaken for the source.
    fn badge(
        &mut self,
        number: usize,
        name: Option<&str>,
        corner: (f64, f64),
        rgb: [u8; 3],
        hollow: bool,
    ) {
        let s = self.scale;
        let h = if hollow { s.badge * 0.8 } else { s.badge };
        let ring = if hollow { s.stroke } else { s.solid_ring() };
        let label = badge_label(number, name);
        let w = badge_width(self.text, &label, h);
        let (iw, ih) = (self.size.width() as f32, self.size.height() as f32);
        let fit = |v: f64, half: f32, max: f32| {
            let lo = half + ring;
            (v.clamp(-FAR, FAR) as f32).clamp(lo, (max - lo).max(lo))
        };
        let center = (
            fit(corner.0, w / 2.0, iw) + self.origin.0,
            fit(corner.1, h / 2.0, ih) + self.origin.1,
        );
        draw_badge(
            self.pixmap,
            self.text,
            Some(self.clip),
            Badge {
                label: &label,
                center,
                height: h,
                ring,
                rgb,
                hollow,
            },
        );
    }
}

/// A numbered badge: a capsule with a ring around it and the number (and
/// the region's name, if it has one) inside.
pub(super) struct Badge<'a> {
    /// The text inside: [`badge_label`].
    pub label: &'a str,
    /// Center, in canvas pixels.
    pub center: (f32, f32),
    pub height: f32,
    /// Width of the ring around the capsule.
    pub ring: f32,
    pub rgb: [u8; 3],
    /// Solid badges are filled with the edit color, a white ring and a white
    /// number; hollow ones are white with a colored ring and number.
    pub hollow: bool,
}

fn badge_style(height: f32) -> Style {
    Style {
        size: height * 0.6,
        bold: true,
    }
}

/// The text of edit `number`'s badge: the number, then the region's name
/// if it has one (`1 logo`), so the mark itself carries the name a caption
/// row or a reply refers to.
pub(super) fn badge_label(number: usize, name: Option<&str>) -> String {
    match name {
        Some(name) => format!("{number} {name}"),
        None => number.to_string(),
    }
}

/// Width of the capsule for `label` at `height` (a circle for one digit).
pub(super) fn badge_width(text: &mut Text, label: &str, height: f32) -> f32 {
    let label = text.measure(label, badge_style(height));
    (label + height * 0.5).max(height)
}

pub(super) fn draw_badge(pixmap: &mut Pixmap, text: &mut Text, clip: Option<&Mask>, b: Badge) {
    let mut label = text.layout(b.label, badge_style(b.height), None);
    let w = (label.width + b.height * 0.5).max(b.height);
    let (cx, cy) = b.center;
    let (outer, inner, ink) = if b.hollow {
        (b.rgb, WHITE, b.rgb)
    } else {
        (WHITE, b.rgb, WHITE)
    };
    for (path, rgb) in [
        (
            pill(cx, cy, w + 2.0 * b.ring, b.height + 2.0 * b.ring),
            outer,
        ),
        (pill(cx, cy, w, b.height), inner),
    ] {
        if let Some(path) = path {
            pixmap.fill_path(
                &path,
                &Marks::paint(rgb),
                FillRule::Winding,
                Transform::identity(),
                clip,
            );
        }
    }
    let x = cx - label.width / 2.0;
    let y = cy - label.first_line_mid;
    text.draw(&mut label, pixmap, clip, (x, y), ink);
}

/// A horizontal capsule (a circle when `w == h`) centered on (`cx`, `cy`).
pub(super) fn pill(cx: f32, cy: f32, w: f32, h: f32) -> Option<Path> {
    let r = h / 2.0;
    let half = (w / 2.0 - r).max(0.0);
    let k = 0.552_284_8 * r; // cubic approximation of a quarter circle
    let (l, rr, t, b) = (cx - half, cx + half, cy - r, cy + r);
    let mut pb = PathBuilder::new();
    pb.move_to(l, t);
    pb.line_to(rr, t);
    pb.cubic_to(rr + k, t, rr + r, cy - k, rr + r, cy);
    pb.cubic_to(rr + r, cy + k, rr + k, b, rr, b);
    pb.line_to(l, b);
    pb.cubic_to(l - k, b, l - r, cy + k, l - r, cy);
    pb.cubic_to(l - r, cy - k, l - k, t, l, t);
    pb.close();
    pb.finish()
}

fn bounds_min(points: &[(f64, f64)]) -> (f64, f64) {
    points
        .iter()
        .fold((f64::INFINITY, f64::INFINITY), |(x, y), &(px, py)| {
            (x.min(px), y.min(py))
        })
}
