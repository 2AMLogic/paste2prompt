//! Normalized coordinates, image size and regions.
//!
//! Every coordinate in the edit model is a fraction of the **original** image's
//! width (`x`) or height (`y`): `0.0` is the left/top edge, `1.0` the
//! right/bottom edge. The y axis points down, as on screen. Normalized
//! coordinates survive chat apps downscaling the exported image, and the
//! caption prints them as percentages (spec §Operations).
//!
//! [`ImageSize`] converts between normalized coordinates and pixels.

use std::fmt;

use serde::{Deserialize, Serialize};

/// A point in normalized image coordinates (`x` as a fraction of width, `y`
/// as a fraction of height, y pointing down).
///
/// A bare `Point` carries no range invariant, because display-only geometry
/// (a [`Ghost`](crate::Ghost) outline) can fall outside the image. Points
/// stored in a [`Region`] are always within `0.0..=1.0`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Point {
    /// Horizontal position as a fraction of image width.
    pub x: f64,
    /// Vertical position as a fraction of image height.
    pub y: f64,
}

impl Point {
    /// Creates a point. No clamping is applied.
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    /// Returns this point clamped into the image (`0.0..=1.0` on both axes).
    /// `NaN` becomes `0.0`.
    pub fn clamped(self) -> Self {
        Self {
            x: clamp_unit(self.x),
            y: clamp_unit(self.y),
        }
    }

    fn is_in_unit_square(self) -> bool {
        (0.0..=1.0).contains(&self.x) && (0.0..=1.0).contains(&self.y)
    }
}

fn clamp_unit(v: f64) -> f64 {
    if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) }
}

/// The original image's size in pixels. Both dimensions are non-zero.
///
/// Used to convert between normalized coordinates and pixels, and for the
/// one place where aspect ratio matters: rotation (see
/// [`Edit::ghost`](crate::Edit::ghost)).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "ImageSizeRepr")]
pub struct ImageSize {
    width: u32,
    height: u32,
}

#[derive(Deserialize)]
struct ImageSizeRepr {
    width: u32,
    height: u32,
}

impl TryFrom<ImageSizeRepr> for ImageSize {
    type Error = GeometryError;
    fn try_from(r: ImageSizeRepr) -> Result<Self, Self::Error> {
        Self::new(r.width, r.height)
    }
}

impl ImageSize {
    /// Creates an image size. Fails if either dimension is zero.
    pub fn new(width: u32, height: u32) -> Result<Self, GeometryError> {
        if width == 0 || height == 0 {
            return Err(GeometryError::EmptyImage);
        }
        Ok(Self { width, height })
    }

    /// Width in pixels.
    pub fn width(self) -> u32 {
        self.width
    }

    /// Height in pixels.
    pub fn height(self) -> u32 {
        self.height
    }

    /// Converts a normalized point to pixel coordinates (`(x, y)`, sub-pixel
    /// precision kept).
    pub fn to_pixels(self, p: Point) -> (f64, f64) {
        (p.x * f64::from(self.width), p.y * f64::from(self.height))
    }

    /// Converts pixel coordinates to a normalized point. No clamping: region
    /// constructors clamp.
    pub fn to_normalized(self, px: f64, py: f64) -> Point {
        Point::new(px / f64::from(self.width), py / f64::from(self.height))
    }

    /// Converts a normalized offset (fractions of width and height) to a pixel
    /// offset. Useful for a translate's `dx`/`dy`.
    pub fn delta_to_pixels(self, dx: f64, dy: f64) -> (f64, f64) {
        (dx * f64::from(self.width), dy * f64::from(self.height))
    }

    /// Converts a pixel offset to a normalized offset.
    pub fn delta_to_normalized(self, dpx: f64, dpy: f64) -> (f64, f64) {
        (dpx / f64::from(self.width), dpy / f64::from(self.height))
    }
}

/// An axis-aligned rectangle in normalized coordinates, with `min` at the
/// top-left and `max` at the bottom-right.
///
/// Invariant: `0 <= min < max <= 1` on both axes (non-zero area).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "RectRepr")]
pub struct Rect {
    min: Point,
    max: Point,
}

#[derive(Deserialize)]
struct RectRepr {
    min: Point,
    max: Point,
}

impl TryFrom<RectRepr> for Rect {
    type Error = GeometryError;
    fn try_from(r: RectRepr) -> Result<Self, Self::Error> {
        if !r.min.is_in_unit_square() || !r.max.is_in_unit_square() {
            return Err(GeometryError::OutOfRange);
        }
        if r.min.x >= r.max.x || r.min.y >= r.max.y {
            return Err(GeometryError::Degenerate);
        }
        Ok(Self {
            min: r.min,
            max: r.max,
        })
    }
}

impl Rect {
    /// Builds a rectangle from any two opposite corners (in any order),
    /// clamped into the image. Fails if the clamped rectangle has zero width
    /// or height, e.g. a click without a drag.
    pub fn from_corners(a: Point, b: Point) -> Result<Self, GeometryError> {
        let (a, b) = (a.clamped(), b.clamped());
        let min = Point::new(a.x.min(b.x), a.y.min(b.y));
        let max = Point::new(a.x.max(b.x), a.y.max(b.y));
        if min.x >= max.x || min.y >= max.y {
            return Err(GeometryError::Degenerate);
        }
        Ok(Self { min, max })
    }

    /// Top-left corner.
    pub fn min(&self) -> Point {
        self.min
    }

    /// Bottom-right corner.
    pub fn max(&self) -> Point {
        self.max
    }

    /// Width as a fraction of image width.
    pub fn width(&self) -> f64 {
        self.max.x - self.min.x
    }

    /// Height as a fraction of image height.
    pub fn height(&self) -> f64 {
        self.max.y - self.min.y
    }

    /// Center point.
    pub fn center(&self) -> Point {
        Point::new(
            (self.min.x + self.max.x) / 2.0,
            (self.min.y + self.max.y) / 2.0,
        )
    }

    /// The four corners, clockwise from top-left.
    pub fn corners(&self) -> [Point; 4] {
        [
            self.min,
            Point::new(self.max.x, self.min.y),
            self.max,
            Point::new(self.min.x, self.max.y),
        ]
    }
}

/// A closed lasso in normalized coordinates, stored as the **control points**
/// of a closed centripetal Catmull-Rom spline (spec §Operations, "Region").
///
/// The lasso's outline is the smooth closed curve through every control
/// point, not the polygon joining them: see [`Region::flatten`]. The lasso
/// tool stores 3 to 10 control points; a document may hold more, which still
/// draws as a curve through all of them.
///
/// Invariant: at least 3 points, all within `0.0..=1.0`, whose control
/// polygon encloses a non-zero area.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "PolygonRepr")]
pub struct Polygon {
    points: Vec<Point>,
}

#[derive(Deserialize)]
struct PolygonRepr {
    points: Vec<Point>,
}

impl TryFrom<PolygonRepr> for Polygon {
    type Error = GeometryError;
    fn try_from(r: PolygonRepr) -> Result<Self, Self::Error> {
        if r.points.iter().any(|p| !p.is_in_unit_square()) {
            return Err(GeometryError::OutOfRange);
        }
        Self::validated(r.points)
    }
}

impl Polygon {
    /// Builds a lasso from its control points, each clamped into the image.
    /// Fails with fewer than 3 points or when the points enclose no area.
    pub fn new(points: impl IntoIterator<Item = Point>) -> Result<Self, GeometryError> {
        Self::validated(points.into_iter().map(Point::clamped).collect())
    }

    fn validated(points: Vec<Point>) -> Result<Self, GeometryError> {
        if points.len() < 3 {
            return Err(GeometryError::TooFewPoints);
        }
        if shoelace_area(&points) == 0.0 {
            return Err(GeometryError::Degenerate);
        }
        Ok(Self { points })
    }

    /// The spline's control points, in drawing order. The outline passes
    /// through each of them; see [`Region::flatten`] for the outline itself.
    pub fn points(&self) -> &[Point] {
        &self.points
    }
}

/// Most segments [`catmull_rom_closed`] cuts one spline segment into, so a
/// tiny or non-finite tolerance can't blow up the outline.
const MAX_SEGMENT_STEPS: usize = 256;

/// Flattens the closed **centripetal** Catmull-Rom spline (alpha = 0.5)
/// through `points` into a polyline within `tolerance` of the curve.
///
/// The curve passes through every point: segment `i` runs from `points[i]`
/// to `points[i + 1]` (wrapping), shaped by the points either side of it.
/// Centripetal knot spacing (the square root of each chord's length) keeps
/// it from overshooting or forming cusps and loops when points bunch up,
/// which uniform Catmull-Rom does.
///
/// Each segment is a cubic, so its second derivative is linear and peaks at
/// an end. A chord over a parameter step `h` strays at most
/// `h² / 8 · max|p''|` from the curve, so cutting the segment into
/// `ceil(sqrt(max|p''| / (8 · tolerance)))` equal steps bounds the error by
/// `tolerance` (up to [`MAX_SEGMENT_STEPS`] steps).
///
/// The result starts at `points[0]`, contains every control point exactly,
/// and has no repeated closing vertex (the closing chord is implied).
fn catmull_rom_closed(points: &[Point], tolerance: f64) -> Vec<Point> {
    let n = points.len();
    if n < 3 {
        return points.to_vec();
    }
    let tolerance = if tolerance > 0.0 { tolerance } else { 1e-9 };
    let at = |i: usize| points[i % n];
    let mut out = Vec::with_capacity(n * 8);
    for i in 0..n {
        let (p0, p1, p2, p3) = (at(i + n - 1), at(i), at(i + 1), at(i + 2));
        let [a, b, c, d] = centripetal_segment(p0, p1, p2, p3);
        // p''(u) = 6a·u + 2b: largest at u = 0 or u = 1.
        let dd0 = 2.0 * b.0.hypot(b.1);
        let dd1 = (6.0 * a.0 + 2.0 * b.0).hypot(6.0 * a.1 + 2.0 * b.1);
        let steps = (dd0.max(dd1) / (8.0 * tolerance)).sqrt().ceil();
        let steps = if steps.is_finite() {
            (steps as usize).clamp(1, MAX_SEGMENT_STEPS)
        } else {
            MAX_SEGMENT_STEPS
        };
        out.push(p1);
        for k in 1..steps {
            let u = k as f64 / steps as f64;
            let x = ((a.0 * u + b.0) * u + c.0) * u + d.0;
            let y = ((a.1 * u + b.1) * u + c.1) * u + d.1;
            out.push(Point::new(x, y));
        }
    }
    out.dedup();
    while out.len() > 1 && out.first() == out.last() {
        out.pop();
    }
    out
}

/// The cubic `a·u³ + b·u² + c·u + d`, `u` in `0..=1`, of the centripetal
/// Catmull-Rom segment from `p1` to `p2` (the tangent form of the
/// Barry-Goldman recursion, reparameterized onto `0..=1`), as `(x, y)` pairs.
fn centripetal_segment(p0: Point, p1: Point, p2: Point, p3: Point) -> [(f64, f64); 4] {
    // Knot intervals: chord length to the power alpha = 0.5. A floor keeps
    // coincident control points from dividing by zero; their chords then
    // contribute nothing.
    let knot = |a: Point, b: Point| (b.x - a.x).hypot(b.y - a.y).sqrt().max(1e-12);
    let t = [knot(p0, p1), knot(p1, p2), knot(p2, p3)];
    let x = hermite_cubic([p0.x, p1.x, p2.x, p3.x], t);
    let y = hermite_cubic([p0.y, p1.y, p2.y, p3.y], t);
    [(x[0], y[0]), (x[1], y[1]), (x[2], y[2]), (x[3], y[3])]
}

/// One axis of [`centripetal_segment`]: the end tangents from the
/// non-uniform knot intervals `t`, scaled onto `0..=1`, then the Hermite
/// cubic's coefficients `[a, b, c, d]`.
fn hermite_cubic(v: [f64; 4], t: [f64; 3]) -> [f64; 4] {
    let [v0, v1, v2, v3] = v;
    let [t01, t12, t23] = t;
    let m1 = ((v1 - v0) / t01 - (v2 - v0) / (t01 + t12) + (v2 - v1) / t12) * t12;
    let m2 = ((v2 - v1) / t12 - (v3 - v1) / (t12 + t23) + (v3 - v2) / t23) * t12;
    [
        2.0 * v1 - 2.0 * v2 + m1 + m2,
        -3.0 * v1 + 3.0 * v2 - 2.0 * m1 - m2,
        m1,
        v1,
    ]
}

/// Unsigned polygon area via the shoelace formula.
fn shoelace_area(points: &[Point]) -> f64 {
    let n = points.len();
    let twice: f64 = (0..n)
        .map(|i| {
            let (a, b) = (points[i], points[(i + 1) % n]);
            a.x * b.y - b.x * a.y
        })
        .sum();
    (twice / 2.0).abs()
}

/// A region of the **original** image that an edit refers to.
///
/// A region is only ever built from original-image coordinates (from the
/// user's drag or lasso, or from moving an existing source region with
/// [`Region::shifted`]). There is deliberately no way to turn another edit's
/// transformed result into a region: edits don't compose (spec §Operations,
/// "Ordering semantics").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Region {
    /// A rectangle, from the rectangle tool (`R`).
    Rect(Rect),
    /// A lasso, from the lasso tool (`L`): a closed centripetal Catmull-Rom
    /// spline through the stored control points. Its outline is
    /// [`Region::flatten`]; the variant keeps the name `polygon` (and its JSON
    /// shape) from before lassos were curves.
    Polygon(Polygon),
}

impl Region {
    /// A rectangle region from two opposite corners in normalized coordinates.
    /// See [`Rect::from_corners`].
    pub fn rect(a: Point, b: Point) -> Result<Self, GeometryError> {
        Rect::from_corners(a, b).map(Self::Rect)
    }

    /// A lasso region from its control points in normalized coordinates. See
    /// [`Polygon::new`].
    pub fn polygon(points: impl IntoIterator<Item = Point>) -> Result<Self, GeometryError> {
        Polygon::new(points).map(Self::Polygon)
    }

    /// A rectangle region from two opposite corners in pixel coordinates.
    pub fn rect_from_pixels(
        size: ImageSize,
        a: (f64, f64),
        b: (f64, f64),
    ) -> Result<Self, GeometryError> {
        Self::rect(size.to_normalized(a.0, a.1), size.to_normalized(b.0, b.1))
    }

    /// A lasso region from its control points in pixel coordinates.
    pub fn polygon_from_pixels(
        size: ImageSize,
        points: impl IntoIterator<Item = (f64, f64)>,
    ) -> Result<Self, GeometryError> {
        Self::polygon(
            points
                .into_iter()
                .map(|(px, py)| size.to_normalized(px, py)),
        )
    }

    /// The axis-aligned bounding rectangle of the [`outline`](Self::outline)
    /// (for a lasso, of the curve, which can bulge past its control points).
    pub fn bounds(&self) -> Rect {
        match self {
            Self::Rect(r) => *r,
            Self::Polygon(_) => {
                let outline = self.outline();
                let (mut min, mut max) = (outline[0], outline[0]);
                for q in &outline[1..] {
                    min = Point::new(min.x.min(q.x), min.y.min(q.y));
                    max = Point::new(max.x.max(q.x), max.y.max(q.y));
                }
                Rect { min, max }
            }
        }
    }

    /// The center of the bounding rectangle. This is the default anchor for
    /// scale and rotate, and the axis position for mirror.
    pub fn center(&self) -> Point {
        self.bounds().center()
    }

    /// The outline as a closed polygon: [`flatten`](Self::flatten) at
    /// [`OUTLINE_TOLERANCE`](Self::OUTLINE_TOLERANCE). Everything that
    /// draws, hit-tests or measures a region uses this, so the canvas and the
    /// export trace the same curve.
    pub fn outline(&self) -> Vec<Point> {
        self.flatten(Self::OUTLINE_TOLERANCE)
    }

    /// How far (in normalized units) [`outline`](Self::outline) may stray
    /// from a lasso's curve: about a pixel on a 4000-pixel-wide image.
    pub const OUTLINE_TOLERANCE: f64 = 2.5e-4;

    /// The outline as a closed polyline (the closing edge from the last point
    /// to the first is implied).
    ///
    /// A rectangle yields its four corners clockwise from top-left. A lasso
    /// yields its closed centripetal Catmull-Rom curve, cut into chords that
    /// stay within `tolerance` (normalized units) of it, starting at the
    /// first control point and passing through every control point exactly.
    /// Where the curve would bulge past the image edge it is clamped onto the
    /// edge, so the outline is always within `0.0..=1.0`.
    pub fn flatten(&self, tolerance: f64) -> Vec<Point> {
        match self {
            Self::Rect(r) => r.corners().to_vec(),
            Self::Polygon(p) => catmull_rom_closed(&p.points, tolerance)
                .into_iter()
                .map(Point::clamped)
                .collect(),
        }
    }

    /// This lasso with control point `index` moved to `to` (clamped into the
    /// image): the `V`-tool control-point handle drag.
    ///
    /// `None` for a rectangle, an index out of range, or a move that would
    /// leave the control points enclosing no area (the drag then keeps the
    /// last valid shape).
    pub fn with_control_point(&self, index: usize, to: Point) -> Option<Self> {
        let Self::Polygon(p) = self else {
            return None;
        };
        let mut points = p.points.clone();
        *points.get_mut(index)? = to.clamped();
        Polygon::validated(points).ok().map(Self::Polygon)
    }

    /// The outline in pixel coordinates.
    pub fn outline_pixels(&self, size: ImageSize) -> Vec<(f64, f64)> {
        self.outline()
            .into_iter()
            .map(|p| size.to_pixels(p))
            .collect()
    }

    /// Moves this **source** region on the original image by a normalized
    /// offset, keeping its shape. The offset is limited so the region stays
    /// entirely inside the image.
    ///
    /// The result is always a valid region that loads back from JSON. If
    /// floating-point rounding would collapse it (only possible for a region
    /// a few ulps wide, which pointer input can't produce), the region is
    /// returned unmoved.
    ///
    /// This is the `V`-tool drag: the edit re-picks which original pixels it
    /// refers to. It is not how a translate's destination is expressed; that
    /// lives in [`Op::Translate`](crate::Op::Translate) and is never a region.
    pub fn shifted(&self, dx: f64, dy: f64) -> Self {
        let b = self.bounds();
        let dx = finite_or_zero(dx).clamp(-b.min.x, 1.0 - b.max.x);
        let dy = finite_or_zero(dy).clamp(-b.min.y, 1.0 - b.max.y);
        // Clamp again to absorb floating-point drift at the edges; the shift
        // already keeps every point in range mathematically.
        let mv = |p: Point| Point::new(p.x + dx, p.y + dy).clamped();
        // Re-run the deserialization checks so the result upholds exactly the
        // invariants `from_json` enforces.
        let moved = match self {
            Self::Rect(r) => Rect::try_from(RectRepr {
                min: mv(r.min),
                max: mv(r.max),
            })
            .map(Self::Rect),
            Self::Polygon(p) => Polygon::try_from(PolygonRepr {
                points: p.points.iter().copied().map(mv).collect(),
            })
            .map(Self::Polygon),
        };
        moved.unwrap_or_else(|_| self.clone())
    }
}

fn finite_or_zero(v: f64) -> f64 {
    if v.is_finite() { v } else { 0.0 }
}

/// Why a size or region could not be built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeometryError {
    /// An image dimension was zero.
    EmptyImage,
    /// A stored coordinate was outside `0.0..=1.0` (or not a number).
    OutOfRange,
    /// The region has zero width, height or area.
    Degenerate,
    /// A polygon had fewer than 3 points.
    TooFewPoints,
}

impl fmt::Display for GeometryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::EmptyImage => "image width and height must be non-zero",
            Self::OutOfRange => "normalized coordinate outside 0..1",
            Self::Degenerate => "region has zero area",
            Self::TooFewPoints => "polygon needs at least 3 points",
        })
    }
}

impl std::error::Error for GeometryError {}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f64 = 1e-9;

    fn size() -> ImageSize {
        ImageSize::new(1920, 1080).unwrap()
    }

    #[test]
    fn image_size_rejects_zero() {
        assert_eq!(ImageSize::new(0, 10), Err(GeometryError::EmptyImage));
        assert_eq!(ImageSize::new(10, 0), Err(GeometryError::EmptyImage));
    }

    #[test]
    fn pixel_normalized_round_trip() {
        let s = size();
        for &(px, py) in &[(0.0, 0.0), (1920.0, 1080.0), (123.5, 987.25), (1.0, 3.0)] {
            let n = s.to_normalized(px, py);
            let (bx, by) = s.to_pixels(n);
            assert!((bx - px).abs() < EPS && (by - py).abs() < EPS);
        }
        let (dx, dy) = s.delta_to_normalized(-192.0, 54.0);
        assert!((dx + 0.1).abs() < EPS && (dy - 0.05).abs() < EPS);
        let (bx, by) = s.delta_to_pixels(dx, dy);
        assert!((bx + 192.0).abs() < EPS && (by - 54.0).abs() < EPS);
    }

    #[test]
    fn rect_from_pixels_round_trips_outline() {
        let s = size();
        let r = Region::rect_from_pixels(s, (960.0, 540.0), (192.0, 108.0)).unwrap();
        let b = r.bounds();
        assert!((b.min().x - 0.1).abs() < EPS && (b.min().y - 0.1).abs() < EPS);
        assert!((b.max().x - 0.5).abs() < EPS && (b.max().y - 0.5).abs() < EPS);
        let px = r.outline_pixels(s);
        let want = [
            (192.0, 108.0),
            (960.0, 108.0),
            (960.0, 540.0),
            (192.0, 540.0),
        ];
        for (got, want) in px.iter().zip(want) {
            assert!((got.0 - want.0).abs() < EPS && (got.1 - want.1).abs() < EPS);
        }
    }

    #[test]
    fn rect_clamps_into_image_and_rejects_degenerate() {
        let r = Rect::from_corners(Point::new(-0.5, 0.2), Point::new(1.5, 0.4)).unwrap();
        assert_eq!(r.min(), Point::new(0.0, 0.2));
        assert_eq!(r.max(), Point::new(1.0, 0.4));
        assert_eq!(
            Rect::from_corners(Point::new(0.3, 0.3), Point::new(0.3, 0.6)),
            Err(GeometryError::Degenerate)
        );
        // Entirely off-image collapses to an edge: degenerate.
        assert_eq!(
            Rect::from_corners(Point::new(1.2, 0.1), Point::new(1.5, 0.4)),
            Err(GeometryError::Degenerate)
        );
        assert!(Rect::from_corners(Point::new(f64::NAN, 0.1), Point::new(0.5, 0.4)).is_ok());
    }

    #[test]
    fn polygon_validation() {
        assert_eq!(
            Polygon::new([Point::new(0.1, 0.1), Point::new(0.2, 0.2)]),
            Err(GeometryError::TooFewPoints)
        );
        assert_eq!(
            Polygon::new([
                Point::new(0.1, 0.1),
                Point::new(0.2, 0.2),
                Point::new(0.3, 0.3)
            ]),
            Err(GeometryError::Degenerate)
        );
        let p = Region::polygon([
            Point::new(0.2, 0.1),
            Point::new(0.6, 0.3),
            Point::new(0.3, 0.7),
        ])
        .unwrap();
        // The bounds are the curve's, which passes through (and here bulges
        // past) the control points.
        let b = p.bounds();
        assert!(b.min().x <= 0.2 && b.min().y <= 0.1);
        assert!(b.max().x >= 0.6 && b.max().y >= 0.7);
        assert_eq!(b, outline_bounds(&p.outline()));
    }

    fn outline_bounds(pts: &[Point]) -> Rect {
        let min = pts.iter().fold(Point::new(1.0, 1.0), |m, q| {
            Point::new(m.x.min(q.x), m.y.min(q.y))
        });
        let max = pts.iter().fold(Point::new(0.0, 0.0), |m, q| {
            Point::new(m.x.max(q.x), m.y.max(q.y))
        });
        Rect { min, max }
    }

    /// An irregular lasso: uneven spacing, two control points close together
    /// and a sharp turn, which is where uniform Catmull-Rom would loop.
    fn wobbly() -> Vec<Point> {
        vec![
            Point::new(0.20, 0.20),
            Point::new(0.55, 0.15),
            Point::new(0.57, 0.16),
            Point::new(0.80, 0.45),
            Point::new(0.60, 0.85),
            Point::new(0.35, 0.60),
            Point::new(0.15, 0.70),
        ]
    }

    /// The point at `u` on spline segment `i` of the closed curve through
    /// `pts`, evaluated directly (not via the flattener).
    fn curve_at(pts: &[Point], i: usize, u: f64) -> Point {
        let n = pts.len();
        let at = |k: usize| pts[k % n];
        let [a, b, c, d] = centripetal_segment(at(i + n - 1), at(i), at(i + 1), at(i + 2));
        Point::new(
            ((a.0 * u + b.0) * u + c.0) * u + d.0,
            ((a.1 * u + b.1) * u + c.1) * u + d.1,
        )
    }

    fn dist_to_segment(p: Point, a: Point, b: Point) -> f64 {
        let (dx, dy) = (b.x - a.x, b.y - a.y);
        let len2 = dx * dx + dy * dy;
        let t = if len2 == 0.0 {
            0.0
        } else {
            (((p.x - a.x) * dx + (p.y - a.y) * dy) / len2).clamp(0.0, 1.0)
        };
        (p.x - a.x - t * dx).hypot(p.y - a.y - t * dy)
    }

    #[test]
    fn catmull_rom_passes_through_its_control_points() {
        let pts = wobbly();
        let region = Region::polygon(pts.clone()).unwrap();
        for tol in [1e-2, 1e-3, Region::OUTLINE_TOLERANCE] {
            let flat = region.flatten(tol);
            assert_eq!(flat[0], pts[0]);
            for p in &pts {
                assert!(flat.contains(p), "{p:?} missing at tolerance {tol}");
            }
            // In order: each control point appears after the previous one.
            let idx: Vec<usize> = pts
                .iter()
                .map(|p| flat.iter().position(|q| q == p).unwrap())
                .collect();
            assert!(idx.windows(2).all(|w| w[0] < w[1]), "{idx:?}");
        }
        // And the analytic segment starts and ends on its control points.
        for i in 0..pts.len() {
            let (s, e) = (curve_at(&pts, i, 0.0), curve_at(&pts, i, 1.0));
            let next = pts[(i + 1) % pts.len()];
            assert!((s.x - pts[i].x).abs() < EPS && (s.y - pts[i].y).abs() < EPS);
            assert!((e.x - next.x).abs() < EPS && (e.y - next.y).abs() < EPS);
        }
    }

    #[test]
    fn flatten_is_closed_and_within_tolerance() {
        let pts = wobbly();
        let region = Region::polygon(pts.clone()).unwrap();
        for tol in [5e-3, 1e-3, Region::OUTLINE_TOLERANCE] {
            let flat = region.flatten(tol);
            let n = flat.len();
            // Closed by an implied edge: no repeated closing vertex, and no
            // zero-length chords.
            assert_ne!(flat[0], flat[n - 1]);
            assert!(flat.windows(2).all(|w| w[0] != w[1]));
            // Every point of the true curve is within `tol` of the polyline.
            let worst = (0..pts.len())
                .flat_map(|i| (0..=400).map(move |k| (i, f64::from(k) / 400.0)))
                .map(|(i, u)| {
                    let c = curve_at(&pts, i, u);
                    (0..n)
                        .map(|j| dist_to_segment(c, flat[j], flat[(j + 1) % n]))
                        .fold(f64::INFINITY, f64::min)
                })
                .fold(0.0, f64::max);
            assert!(worst <= tol + 1e-12, "tolerance {tol}: off by {worst}");
        }
        // A coarser tolerance needs fewer points.
        assert!(region.flatten(1e-2).len() < region.flatten(1e-4).len());
        // A rectangle flattens to its corners.
        let r = Region::rect(Point::new(0.1, 0.2), Point::new(0.3, 0.4)).unwrap();
        assert_eq!(r.flatten(1e-3), r.outline());
        assert_eq!(r.outline().len(), 4);
    }

    #[test]
    fn flatten_survives_coincident_points_and_bad_tolerances() {
        let mut pts = wobbly();
        pts.insert(2, pts[1]);
        let region = Region::polygon(pts).unwrap();
        for tol in [0.0, -1.0, f64::NAN, f64::INFINITY, 1e-300] {
            let flat = region.flatten(tol);
            assert!(flat.len() >= 3);
            assert!(flat.iter().all(|p| p.is_in_unit_square()), "{tol}");
        }
    }

    #[test]
    fn lasso_bounds_and_center_come_from_the_curve() {
        // Eight control points on a circle: the centripetal spline through
        // them is very nearly that circle, so its bounds are the circle's
        // box, wider than the control points' own.
        let (cx, cy, r) = (0.5, 0.4, 0.2);
        let ring: Vec<Point> = (0..8)
            .map(|k| {
                let a = f64::from(k) * std::f64::consts::TAU / 8.0 + 0.3;
                Point::new(cx + r * a.cos(), cy + r * a.sin())
            })
            .collect();
        let circle = Region::polygon(ring.clone()).unwrap();
        let b = circle.bounds();
        for (got, want) in [
            (b.min().x, cx - r),
            (b.min().y, cy - r),
            (b.max().x, cx + r),
            (b.max().y, cy + r),
        ] {
            assert!((got - want).abs() < 0.004, "{got} vs {want}");
        }
        // Rotated by 0.3 rad, no control point sits on the box, so the
        // control polygon's own bounds are strictly smaller.
        let ctrl = outline_bounds(&ring);
        assert!(b.min().x < ctrl.min().x && b.max().x > ctrl.max().x);
        assert!((circle.center().x - cx).abs() < 1e-3 && (circle.center().y - cy).abs() < 1e-3);
        assert_eq!(b, outline_bounds(&circle.outline()));

        // A square's four control points give a rounded square bulging
        // symmetrically past them; its center stays the square's center.
        let sq = Region::polygon([
            Point::new(0.3, 0.3),
            Point::new(0.7, 0.3),
            Point::new(0.7, 0.7),
            Point::new(0.3, 0.7),
        ])
        .unwrap();
        let b = sq.bounds();
        assert!(b.min().x < 0.3 && b.max().x > 0.7);
        assert!(((0.3 - b.min().x) - (b.max().x - 0.7)).abs() < 1e-9);
        assert!((sq.center().x - 0.5).abs() < EPS && (sq.center().y - 0.5).abs() < EPS);
    }

    #[test]
    fn lasso_curve_is_clamped_to_the_image() {
        // Control points on the image edges: the curve would bulge outside.
        let edge = Region::polygon([
            Point::new(0.0, 0.0),
            Point::new(1.0, 0.0),
            Point::new(1.0, 1.0),
            Point::new(0.0, 1.0),
        ])
        .unwrap();
        assert!(edge.outline().iter().all(|p| p.is_in_unit_square()));
        let b = edge.bounds();
        assert_eq!(
            (b.min(), b.max()),
            (Point::new(0.0, 0.0), Point::new(1.0, 1.0))
        );
        // A shift can't move it anywhere.
        assert_eq!(edge.shifted(0.2, -0.2), edge);
    }

    #[test]
    fn with_control_point_moves_one_point() {
        let tri = Region::polygon([
            Point::new(0.2, 0.2),
            Point::new(0.6, 0.2),
            Point::new(0.4, 0.6),
        ])
        .unwrap();
        let moved = tri.with_control_point(2, Point::new(0.4, 1.7)).unwrap();
        let Region::Polygon(p) = &moved else {
            panic!("still a lasso")
        };
        // Clamped into the image; the others untouched.
        assert_eq!(p.points()[2], Point::new(0.4, 1.0));
        assert_eq!(
            p.points()[..2],
            [Point::new(0.2, 0.2), Point::new(0.6, 0.2)]
        );
        assert!(reloads(&moved));
        // Collinear: no area, so no move.
        assert_eq!(tri.with_control_point(2, Point::new(0.9, 0.2)), None);
        assert_eq!(tri.with_control_point(3, Point::new(0.5, 0.5)), None);
        let r = Region::rect(Point::new(0.1, 0.1), Point::new(0.3, 0.2)).unwrap();
        assert_eq!(r.with_control_point(0, Point::new(0.0, 0.0)), None);
    }

    #[test]
    fn lasso_json_still_stores_control_points() {
        let tri = Region::polygon([
            Point::new(0.2, 0.2),
            Point::new(0.6, 0.2),
            Point::new(0.4, 0.6),
        ])
        .unwrap();
        assert_eq!(
            serde_json::to_string(&tri).unwrap(),
            r#"{"kind":"polygon","points":[{"x":0.2,"y":0.2},{"x":0.6,"y":0.2},{"x":0.4,"y":0.6}]}"#
        );
    }

    #[test]
    fn shifted_keeps_region_inside_image() {
        let r = Region::rect(Point::new(0.1, 0.1), Point::new(0.3, 0.2)).unwrap();
        let moved = r.shifted(0.2, 0.1).bounds();
        assert!((moved.min().x - 0.3).abs() < EPS && (moved.min().y - 0.2).abs() < EPS);
        let pinned = r.shifted(5.0, -5.0).bounds();
        assert_eq!(pinned.max().x, 1.0);
        assert_eq!(pinned.min().y, 0.0);
        assert!((pinned.width() - 0.2).abs() < EPS);
        assert_eq!(r.shifted(f64::NAN, f64::INFINITY), r);
    }

    /// The next representable `f64` above `v` (for positive `v`).
    fn next_up(v: f64) -> f64 {
        f64::from_bits(v.to_bits() + 1)
    }

    fn reloads(r: &Region) -> bool {
        serde_json::from_str::<Region>(&serde_json::to_string(r).unwrap())
            .ok()
            .as_ref()
            == Some(r)
    }

    #[test]
    fn shifted_never_collapses_a_region() {
        // A 1-ulp-wide rect near 0.1: shifting it into [0.5, 1) rounds both
        // edges to the same value, which used to give a zero-width rect that
        // `from_json` then rejected.
        let x = 0.1;
        let thin = Region::rect(Point::new(x, 0.1), Point::new(next_up(x), 0.3)).unwrap();
        let moved = thin.shifted(0.5, 0.1);
        assert!(moved.bounds().width() > 0.0);
        assert!(reloads(&moved));
        // It stays where it was rather than collapsing.
        assert_eq!(moved, thin);

        // Same for a polygon whose area rounds away.
        let sliver = Region::polygon([
            Point::new(x, 0.1),
            Point::new(next_up(x), 0.1),
            Point::new(x, 0.3),
        ])
        .unwrap();
        let moved = sliver.shifted(0.5, 0.0);
        assert!(reloads(&moved));
        assert_eq!(moved, sliver);

        // An ordinary shift still moves and still reloads.
        let r = Region::rect(Point::new(0.1, 0.1), Point::new(0.3, 0.2)).unwrap();
        let moved = r.shifted(0.5, 0.5);
        assert_ne!(moved, r);
        assert!(reloads(&moved));
    }

    #[test]
    fn deserialize_rejects_invalid_regions() {
        let bad = r#"{"kind":"rect","min":{"x":0.5,"y":0.1},"max":{"x":0.2,"y":0.4}}"#;
        assert!(serde_json::from_str::<Region>(bad).is_err());
        let out = r#"{"kind":"polygon","points":[{"x":0,"y":0},{"x":1.5,"y":0},{"x":0,"y":1}]}"#;
        assert!(serde_json::from_str::<Region>(out).is_err());
        assert!(serde_json::from_str::<ImageSize>(r#"{"width":0,"height":5}"#).is_err());
    }
}
