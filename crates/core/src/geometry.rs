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

/// A closed lasso polygon in normalized coordinates. The closing segment from
/// the last point back to the first is implied.
///
/// Invariant: at least 3 points, all within `0.0..=1.0`, enclosing a
/// non-zero area.
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
    /// Builds a polygon from lasso points, each clamped into the image. Fails
    /// with fewer than 3 points or when the points enclose no area.
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

    /// The polygon's vertices, in drawing order.
    pub fn points(&self) -> &[Point] {
        &self.points
    }
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
    /// A lasso polygon, from the lasso tool (`L`).
    Polygon(Polygon),
}

impl Region {
    /// A rectangle region from two opposite corners in normalized coordinates.
    /// See [`Rect::from_corners`].
    pub fn rect(a: Point, b: Point) -> Result<Self, GeometryError> {
        Rect::from_corners(a, b).map(Self::Rect)
    }

    /// A polygon region from lasso points in normalized coordinates. See
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

    /// A polygon region from lasso points in pixel coordinates.
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

    /// The axis-aligned bounding rectangle.
    pub fn bounds(&self) -> Rect {
        match self {
            Self::Rect(r) => *r,
            Self::Polygon(p) => {
                let (mut min, mut max) = (p.points[0], p.points[0]);
                for q in &p.points[1..] {
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

    /// The outline as a closed polygon (a rectangle yields its four corners
    /// clockwise from top-left).
    pub fn outline(&self) -> Vec<Point> {
        match self {
            Self::Rect(r) => r.corners().to_vec(),
            Self::Polygon(p) => p.points.clone(),
        }
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
        let b = p.bounds();
        assert_eq!(b.min(), Point::new(0.2, 0.1));
        assert_eq!(b.max(), Point::new(0.6, 0.7));
        assert!((p.center().x - 0.4).abs() < EPS && (p.center().y - 0.4).abs() < EPS);
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
