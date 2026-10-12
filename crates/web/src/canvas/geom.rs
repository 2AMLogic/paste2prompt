//! Pure geometry for the canvas tools: hit-testing, lasso simplification and
//! handle math. Everything here works in **image pixels** (`(x, y)`, y down)
//! and has no browser dependency, so it is unit-tested natively.
//!
//! Hit-testing only ever looks at an edit's **source** region. Ghosts (the
//! display-only result of an op) are never consulted, so clicking one
//! selects nothing (spec §Operations, "a moved region can't be selected").

use paste2prompt_core::{Document, EditId, ImageSize, Region};

/// A point in image pixels.
pub type Px = (f64, f64);

/// Smallest scale factor a handle drag produces. Flipping is mirror's job,
/// so a drag past the anchor clamps here instead of going negative.
pub const MIN_SCALE: f64 = 0.05;
/// Largest scale factor a handle drag produces.
pub const MAX_SCALE: f64 = 20.0;

/// Euclidean distance.
pub fn dist(a: Px, b: Px) -> f64 {
    (a.0 - b.0).hypot(a.1 - b.1)
}

/// Even-odd point-in-polygon test. Points exactly on an edge may land either
/// way; that is fine for picking.
pub fn point_in_polygon(p: Px, poly: &[Px]) -> bool {
    let n = poly.len();
    if n < 3 {
        return false;
    }
    let mut inside = false;
    let mut j = n - 1;
    for i in 0..n {
        let (xi, yi) = poly[i];
        let (xj, yj) = poly[j];
        if (yi > p.1) != (yj > p.1) && p.0 < (xj - xi) * (p.1 - yi) / (yj - yi) + xi {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// Whether `p` lies inside `region`'s **source** outline.
pub fn region_contains(region: &Region, size: ImageSize, p: Px) -> bool {
    point_in_polygon(p, &region.outline_pixels(size))
}

/// The edit whose source region contains `p`, topmost (latest) first.
///
/// Only source regions are tested. Ghosts are display-only and are never
/// hit-testable, so a point that is only inside a ghost returns `None`.
pub fn hit_test(doc: &Document, p: Px) -> Option<EditId> {
    let size = doc.image_size();
    doc.edits()
        .iter()
        .rev()
        .find(|e| region_contains(e.region(), size, p))
        .map(|e| e.id())
}

/// Perpendicular distance from `p` to the segment `a`-`b`.
fn segment_distance(p: Px, a: Px, b: Px) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    if len2 == 0.0 {
        return dist(p, a);
    }
    let t = (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0);
    dist(p, (a.0 + t * dx, a.1 + t * dy))
}

/// Ramer-Douglas-Peucker on an open polyline; keeps both endpoints.
fn rdp(points: &[Px], epsilon: f64, out: &mut Vec<Px>) {
    let (first, last) = (points[0], points[points.len() - 1]);
    let (mut index, mut max) = (0, 0.0);
    for (i, &p) in points.iter().enumerate().take(points.len() - 1).skip(1) {
        let d = segment_distance(p, first, last);
        if d > max {
            (index, max) = (i, d);
        }
    }
    if max > epsilon {
        rdp(&points[..=index], epsilon, out);
        out.pop(); // shared vertex, re-added by the second half
        rdp(&points[index..], epsilon, out);
    } else {
        out.push(first);
        out.push(last);
    }
}

/// Closes and simplifies a freehand lasso path.
///
/// The path is treated as closed (the last point joins the first). Exact
/// duplicate points are dropped, a trailing point equal to the first is
/// removed, and Ramer-Douglas-Peucker removes vertices closer than
/// `epsilon` pixels to the simplified outline. The result has no repeated
/// closing vertex; it may have fewer than 3 points for a tiny or straight
/// scribble, which the caller treats as "no region".
pub fn simplify_closed(points: &[Px], epsilon: f64) -> Vec<Px> {
    let mut pts: Vec<Px> = Vec::with_capacity(points.len());
    for &p in points {
        if pts.last() != Some(&p) {
            pts.push(p);
        }
    }
    while pts.len() > 1 && pts.first() == pts.last() {
        pts.pop();
    }
    if pts.len() < 3 {
        return pts;
    }
    // Split the loop at the vertex farthest from the first one and simplify
    // both halves as open polylines; both split points are kept.
    let start = pts[0];
    let far = (1..pts.len())
        .max_by(|&a, &b| dist(start, pts[a]).total_cmp(&dist(start, pts[b])))
        .expect("at least 3 points");
    let mut first_half = Vec::new();
    rdp(&pts[..=far], epsilon, &mut first_half);
    let mut second: Vec<Px> = pts[far..].to_vec();
    second.push(start);
    let mut second_half = Vec::new();
    rdp(&second, epsilon, &mut second_half);
    // first_half ends with `far`, second_half starts with it and ends with
    // `start`; drop both duplicates.
    first_half.extend_from_slice(&second_half[1..second_half.len() - 1]);
    first_half
}

/// Normalizes an angle to `(-180, 180]` degrees.
pub fn normalize_degrees(deg: f64) -> f64 {
    let d = deg.rem_euclid(360.0);
    if d > 180.0 { d - 360.0 } else { d }
}

/// Rounds an angle to the nearest multiple of `step` degrees.
pub fn snap_degrees(deg: f64, step: f64) -> f64 {
    normalize_degrees((deg / step).round() * step)
}

/// The rotation, in degrees clockwise on screen, that takes "straight up
/// from `anchor`" to the direction of `pointer`. This is the angle a rotate
/// handle drag sets: the handle starts directly above the anchor.
pub fn rotation_degrees(anchor: Px, pointer: Px) -> f64 {
    let (dx, dy) = (pointer.0 - anchor.0, pointer.1 - anchor.1);
    if dx == 0.0 && dy == 0.0 {
        return 0.0;
    }
    // atan2 with y down is clockwise from +x; "up" is -90° there.
    normalize_degrees(dy.atan2(dx).to_degrees() + 90.0)
}

/// Rotates `p` about `anchor` by `degrees` clockwise (y down).
pub fn rotate_about(p: Px, anchor: Px, degrees: f64) -> Px {
    let (sin, cos) = degrees.to_radians().sin_cos();
    let (rx, ry) = (p.0 - anchor.0, p.1 - anchor.1);
    (
        anchor.0 + rx * cos - ry * sin,
        anchor.1 + rx * sin + ry * cos,
    )
}

/// Where the rotate handle sits: `offset` pixels above the top of the
/// region's bounding box, straight above `anchor`, then turned with the
/// current rotation so it rides along with the ghost.
pub fn rotate_handle(anchor: Px, bounds_top: f64, degrees: f64, offset: f64) -> Px {
    let reach = (anchor.1 - bounds_top).max(0.0) + offset;
    rotate_about((anchor.0, anchor.1 - reach), anchor, degrees)
}

fn clamp_scale(s: f64) -> f64 {
    if s.is_finite() {
        s.clamp(MIN_SCALE, MAX_SCALE)
    } else {
        1.0
    }
}

/// Scale factors for dragging the corner that started at `corner0` (in the
/// unscaled region) to `pointer`, pivoting on `anchor`.
///
/// Free drags scale x and y independently, so the handle stays under the
/// pointer. With `uniform` (Shift) both factors are the projection of the
/// pointer onto the anchor-corner diagonal, keeping the aspect ratio.
/// Factors are clamped to [`MIN_SCALE`]..=[`MAX_SCALE`]; an axis with no
/// extent (the anchor level with the corner) keeps factor 1.
pub fn scale_factors(anchor: Px, corner0: Px, pointer: Px, uniform: bool) -> (f64, f64) {
    let (cx, cy) = (corner0.0 - anchor.0, corner0.1 - anchor.1);
    let (px, py) = (pointer.0 - anchor.0, pointer.1 - anchor.1);
    if uniform {
        let len2 = cx * cx + cy * cy;
        if len2 == 0.0 {
            return (1.0, 1.0);
        }
        let s = clamp_scale((px * cx + py * cy) / len2);
        return (s, s);
    }
    let axis = |p: f64, c: f64| if c == 0.0 { 1.0 } else { clamp_scale(p / c) };
    (axis(px, cx), axis(py, cy))
}

/// Where a scaled corner lands: `corner0` scaled by `(sx, sy)` about `anchor`.
pub fn scaled_point(anchor: Px, corner0: Px, sx: f64, sy: f64) -> Px {
    (
        anchor.0 + (corner0.0 - anchor.0) * sx,
        anchor.1 + (corner0.1 - anchor.1) * sy,
    )
}

/// The four bounding-box corners of `region` in pixels, clockwise from
/// top-left.
pub fn bounds_corners(region: &Region, size: ImageSize) -> [Px; 4] {
    region.bounds().corners().map(|p| size.to_pixels(p))
}

#[cfg(test)]
mod tests {
    use super::*;
    use paste2prompt_core::{Editor, Op, Point};

    const EPS: f64 = 1e-9;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    fn square() -> Vec<Px> {
        vec![(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]
    }

    #[test]
    fn point_in_polygon_square() {
        let sq = square();
        assert!(point_in_polygon((5.0, 5.0), &sq));
        assert!(!point_in_polygon((15.0, 5.0), &sq));
        assert!(!point_in_polygon((5.0, -1.0), &sq));
        assert!(!point_in_polygon((5.0, 5.0), &sq[..2]));
    }

    #[test]
    fn point_in_polygon_concave() {
        // A "U": the notch between the arms is outside.
        let u = vec![
            (0.0, 0.0),
            (3.0, 0.0),
            (3.0, 7.0),
            (7.0, 7.0),
            (7.0, 0.0),
            (10.0, 0.0),
            (10.0, 10.0),
            (0.0, 10.0),
        ];
        assert!(point_in_polygon((1.0, 1.0), &u));
        assert!(!point_in_polygon((5.0, 3.0), &u));
        assert!(point_in_polygon((5.0, 9.0), &u));
    }

    fn editor() -> Editor {
        Editor::new(ImageSize::new(200, 100).unwrap())
    }

    #[test]
    fn hit_test_finds_topmost_source_region() {
        let mut ed = editor();
        let size = ed.document().image_size();
        let a = ed
            .add_edit(
                Region::rect_from_pixels(size, (10.0, 10.0), (100.0, 90.0)).unwrap(),
                Op::Hint,
                Some("a".into()),
            )
            .unwrap();
        let b = ed
            .add_edit(
                Region::rect_from_pixels(size, (50.0, 10.0), (150.0, 90.0)).unwrap(),
                Op::Hint,
                Some("b".into()),
            )
            .unwrap();
        assert_eq!(hit_test(ed.document(), (20.0, 50.0)), Some(a));
        assert_eq!(hit_test(ed.document(), (70.0, 50.0)), Some(b));
        assert_eq!(hit_test(ed.document(), (190.0, 50.0)), None);
    }

    #[test]
    fn hit_test_ignores_ghosts() {
        let mut ed = editor();
        let size = ed.document().image_size();
        let id = ed
            .add_edit(
                Region::rect_from_pixels(size, (10.0, 10.0), (50.0, 50.0)).unwrap(),
                Op::Translate { dx: 0.5, dy: 0.0 },
                None,
            )
            .unwrap();
        // The ghost sits at x 110..150; a point inside it hits nothing.
        let ghost = ed.document().edit(id).unwrap().ghost(size).unwrap();
        let ghost_px = ghost.outline_pixels(size);
        assert!(point_in_polygon((130.0, 30.0), &ghost_px));
        assert_eq!(hit_test(ed.document(), (130.0, 30.0)), None);
        assert_eq!(hit_test(ed.document(), (30.0, 30.0)), Some(id));
    }

    #[test]
    fn region_contains_lasso() {
        let size = ImageSize::new(100, 100).unwrap();
        let tri = Region::polygon([
            Point::new(0.1, 0.1),
            Point::new(0.9, 0.1),
            Point::new(0.5, 0.9),
        ])
        .unwrap();
        assert!(region_contains(&tri, size, (50.0, 30.0)));
        assert!(!region_contains(&tri, size, (10.0, 80.0)));
    }

    #[test]
    fn simplify_drops_collinear_points_and_closing_duplicate() {
        let mut path = Vec::new();
        for i in 0..=10 {
            path.push((f64::from(i), 0.0));
        }
        for i in 1..=10 {
            path.push((10.0, f64::from(i)));
        }
        for i in (0..10).rev() {
            path.push((f64::from(i), 10.0));
        }
        for i in (0..10).rev() {
            path.push((0.0, f64::from(i)));
        }
        assert_eq!(path.first(), path.last());
        let s = simplify_closed(&path, 0.5);
        assert_eq!(s.len(), 4, "{s:?}");
        for corner in square() {
            assert!(s.contains(&corner), "missing {corner:?} in {s:?}");
        }
    }

    #[test]
    fn simplify_keeps_detail_above_epsilon() {
        // A square with a 3 px bump on the top edge.
        let path = vec![
            (0.0, 0.0),
            (4.0, 0.0),
            (5.0, -3.0),
            (6.0, 0.0),
            (10.0, 0.0),
            (10.0, 10.0),
            (0.0, 10.0),
        ];
        assert_eq!(simplify_closed(&path, 1.0).len(), 7);
        assert_eq!(simplify_closed(&path, 4.0).len(), 4);
    }

    #[test]
    fn simplify_degenerate_inputs() {
        assert!(simplify_closed(&[], 1.0).is_empty());
        assert_eq!(simplify_closed(&[(1.0, 1.0), (1.0, 1.0)], 1.0).len(), 1);
        // A straight back-and-forth scribble collapses below 3 points.
        let line = [(0.0, 0.0), (5.0, 0.0), (10.0, 0.0), (5.0, 0.0)];
        assert!(simplify_closed(&line, 1.0).len() < 3);
    }

    #[test]
    fn simplified_lasso_is_a_valid_region() {
        // A noisy circle.
        let pts: Vec<Px> = (0..200)
            .map(|i| {
                let t = f64::from(i) / 200.0 * std::f64::consts::TAU;
                let r = 40.0 + if i % 2 == 0 { 0.3 } else { -0.3 };
                (50.0 + r * t.cos(), 50.0 + r * t.sin())
            })
            .collect();
        let s = simplify_closed(&pts, 1.5);
        assert!(s.len() >= 8 && s.len() < 60, "{}", s.len());
        let size = ImageSize::new(100, 100).unwrap();
        assert!(Region::polygon_from_pixels(size, s).is_ok());
    }

    #[test]
    fn degrees_normalize_and_snap() {
        assert!(close(normalize_degrees(190.0), -170.0));
        assert!(close(normalize_degrees(-180.0), 180.0));
        assert!(close(normalize_degrees(720.0), 0.0));
        assert!(close(snap_degrees(22.0, 15.0), 15.0));
        assert!(close(snap_degrees(23.0, 15.0), 30.0));
        assert!(close(snap_degrees(-97.0, 15.0), -90.0));
        assert!(close(snap_degrees(178.0, 15.0), 180.0));
    }

    #[test]
    fn rotation_degrees_is_clockwise_from_up() {
        let a = (50.0, 50.0);
        assert!(close(rotation_degrees(a, (50.0, 0.0)), 0.0));
        assert!(close(rotation_degrees(a, (100.0, 50.0)), 90.0));
        assert!(close(rotation_degrees(a, (50.0, 100.0)), 180.0));
        assert!(close(rotation_degrees(a, (0.0, 50.0)), -90.0));
        assert!(close(rotation_degrees(a, a), 0.0));
    }

    #[test]
    fn rotate_handle_round_trips_through_rotation_degrees() {
        let a = (50.0, 50.0);
        for deg in [0.0, 15.0, 90.0, -135.0, 180.0] {
            let h = rotate_handle(a, 30.0, deg, 10.0);
            assert!(close(dist(a, h), 30.0));
            assert!(close(rotation_degrees(a, h), deg), "{deg}");
        }
    }

    #[test]
    fn rotate_about_matches_core_ghost() {
        // Core rotates clockwise in pixel space; so must the handle math.
        let p = rotate_about((10.0, 0.0), (0.0, 0.0), 90.0);
        assert!(p.0.abs() < EPS && close(p.1, 10.0));
    }

    #[test]
    fn scale_free_follows_pointer() {
        let a = (50.0, 50.0);
        let c = (70.0, 60.0);
        let (sx, sy) = scale_factors(a, c, (90.0, 65.0), false);
        assert!(close(sx, 2.0) && close(sy, 1.5));
        let back = scaled_point(a, c, sx, sy);
        assert!(close(back.0, 90.0) && close(back.1, 65.0));
    }

    #[test]
    fn scale_uniform_keeps_aspect() {
        let a = (0.0, 0.0);
        let c = (10.0, 10.0);
        let (sx, sy) = scale_factors(a, c, (30.0, 10.0), true);
        assert!(close(sx, sy));
        assert!(close(sx, 2.0));
    }

    #[test]
    fn scale_clamps_past_anchor_and_degenerate_axes() {
        let a = (50.0, 50.0);
        let (sx, sy) = scale_factors(a, (60.0, 60.0), (40.0, 40.0), false);
        assert!(close(sx, MIN_SCALE) && close(sy, MIN_SCALE));
        let (sx, sy) = scale_factors(a, (60.0, 50.0), (80.0, 90.0), false);
        assert!(close(sx, 3.0) && close(sy, 1.0));
        assert_eq!(scale_factors(a, a, (80.0, 90.0), true), (1.0, 1.0));
        let (sx, _) = scale_factors(a, (51.0, 51.0), (1e9, 50.0), false);
        assert!(close(sx, MAX_SCALE));
    }
}
