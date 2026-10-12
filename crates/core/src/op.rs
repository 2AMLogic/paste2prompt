//! Operations attached to a region, and the display-only ghost they produce.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::geometry::{ImageSize, Point, Region};

/// The point a scale or rotate pivots around.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Anchor {
    /// The center of the region's bounding rectangle (the spec's default).
    #[default]
    Center,
    /// A fixed point in normalized image coordinates, within `0.0..=1.0` on
    /// both axes (checked by [`Op::validate`]).
    Point(Point),
}

impl Anchor {
    /// Resolves the anchor to a concrete point for `region`.
    pub fn resolve(&self, region: &Region) -> Point {
        match self {
            Self::Center => region.center(),
            Self::Point(p) => *p,
        }
    }
}

/// Which way a mirror flips the region. The flip is about the region's center.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MirrorAxis {
    /// Flip left and right (reflect across a vertical line through the
    /// region's center).
    Horizontal,
    /// Flip top and bottom (reflect across a horizontal line through the
    /// region's center).
    Vertical,
}

impl MirrorAxis {
    /// The other axis. The `M` key toggles between them.
    pub fn toggled(self) -> Self {
        match self {
            Self::Horizontal => Self::Vertical,
            Self::Vertical => Self::Horizontal,
        }
    }
}

/// The largest scale factor [`Op::validate`] accepts on either axis.
pub const MAX_SCALE: f64 = 100.0;

/// What the user asks the model to do with a region (spec §Operations).
///
/// All offsets are normalized like every other coordinate: `dx` is a fraction
/// of image width, `dy` of image height, with y pointing down. Every parameter
/// describes a change to the region **as it is in the original image**;
/// edits don't compose.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Op {
    /// Move the region by (`dx`, `dy`). Positive `dx` is right, positive `dy`
    /// is down.
    Translate {
        /// Horizontal offset as a fraction of image width.
        dx: f64,
        /// Vertical offset as a fraction of image height.
        dy: f64,
    },
    /// Scale the region by `sx` horizontally and `sy` vertically about
    /// `anchor`. Uniform when `sx == sy`. Both factors are positive.
    Scale {
        /// Horizontal factor (`> 0`).
        sx: f64,
        /// Vertical factor (`> 0`).
        sy: f64,
        /// Pivot point.
        #[serde(default)]
        anchor: Anchor,
    },
    /// Mirror the region about its center.
    Mirror {
        /// Which way to flip.
        axis: MirrorAxis,
    },
    /// Rotate the region by `degrees` about `anchor`. Positive is clockwise
    /// as seen on screen.
    Rotate {
        /// Angle in degrees, clockwise.
        degrees: f64,
        /// Pivot point.
        #[serde(default)]
        anchor: Anchor,
    },
    /// No geometric change; the edit's hint text carries the instruction.
    Hint,
    /// No change at all: the region is only named, so the instructions and
    /// the model's reply can refer to it by that name. A label edit always
    /// has a name ([`Edit::name`](crate::Edit::name)).
    Label,
}

impl Op {
    /// A uniform scale about the region's center.
    pub fn uniform_scale(factor: f64) -> Self {
        Self::Scale {
            sx: factor,
            sy: factor,
            anchor: Anchor::Center,
        }
    }

    /// The operation's lowercase name, as used in the caption (`"translate"`,
    /// `"scale"`, `"mirror"`, `"rotate"`, `"hint"`, `"label"`).
    pub fn name(&self) -> &'static str {
        match self {
            Self::Translate { .. } => "translate",
            Self::Scale { .. } => "scale",
            Self::Mirror { .. } => "mirror",
            Self::Rotate { .. } => "rotate",
            Self::Hint => "hint",
            Self::Label => "label",
        }
    }

    /// Checks the parameters:
    ///
    /// - every number is finite;
    /// - translate `dx` and `dy` are within `-1.0..=1.0` (a move past the full
    ///   image width or height is meaningless);
    /// - scale `sx` and `sy` are positive and at most [`MAX_SCALE`];
    /// - an [`Anchor::Point`] is inside the image (`0.0..=1.0` on both axes).
    ///
    /// The editor calls this on [`add_edit`](crate::Editor::add_edit) and
    /// [`set_op`](crate::Editor::set_op), and loading a document calls it on
    /// every edit, so a stored op always passes.
    pub fn validate(&self) -> Result<(), OpError> {
        let finite = |v: f64| {
            if v.is_finite() {
                Ok(())
            } else {
                Err(OpError::NotFinite)
            }
        };
        match *self {
            Self::Translate { dx, dy } => {
                finite(dx)?;
                finite(dy)?;
                if !(-1.0..=1.0).contains(&dx) || !(-1.0..=1.0).contains(&dy) {
                    return Err(OpError::OutOfRange);
                }
                Ok(())
            }
            Self::Scale { sx, sy, anchor } => {
                finite(sx)?;
                finite(sy)?;
                validate_anchor(anchor)?;
                if sx <= 0.0 || sy <= 0.0 {
                    return Err(OpError::NonPositiveScale);
                }
                if sx > MAX_SCALE || sy > MAX_SCALE {
                    return Err(OpError::OutOfRange);
                }
                Ok(())
            }
            Self::Rotate { degrees, anchor } => {
                finite(degrees)?;
                validate_anchor(anchor)
            }
            Self::Mirror { .. } | Self::Hint | Self::Label => Ok(()),
        }
    }

    /// The display-only outline of `region` after this op, or `None` for a
    /// hint or a label. See [`Ghost`].
    pub(crate) fn ghost(&self, region: &Region, size: ImageSize) -> Option<Ghost> {
        let outline = region.outline();
        let map = |f: &dyn Fn(Point) -> Point| outline.iter().copied().map(f).collect();
        let points = match *self {
            Self::Translate { dx, dy } => map(&|p| Point::new(p.x + dx, p.y + dy)),
            Self::Scale { sx, sy, anchor } => {
                let a = anchor.resolve(region);
                map(&|p| Point::new(a.x + (p.x - a.x) * sx, a.y + (p.y - a.y) * sy))
            }
            Self::Mirror { axis } => {
                let c = region.center();
                match axis {
                    MirrorAxis::Horizontal => map(&|p| Point::new(2.0 * c.x - p.x, p.y)),
                    MirrorAxis::Vertical => map(&|p| Point::new(p.x, 2.0 * c.y - p.y)),
                }
            }
            Self::Rotate { degrees, anchor } => {
                // Rotate in pixel space so a 90° turn stays square on a
                // non-square image, then normalize back.
                let (ax, ay) = size.to_pixels(anchor.resolve(region));
                let (sin, cos) = degrees.to_radians().sin_cos();
                map(&|p| {
                    let (px, py) = size.to_pixels(p);
                    let (rx, ry) = (px - ax, py - ay);
                    size.to_normalized(ax + rx * cos - ry * sin, ay + rx * sin + ry * cos)
                })
            }
            Self::Hint | Self::Label => return None,
        };
        Some(Ghost { outline: points })
    }
}

fn validate_anchor(anchor: Anchor) -> Result<(), OpError> {
    match anchor {
        Anchor::Center => Ok(()),
        Anchor::Point(p) if !(p.x.is_finite() && p.y.is_finite()) => Err(OpError::NotFinite),
        Anchor::Point(p) if (0.0..=1.0).contains(&p.x) && (0.0..=1.0).contains(&p.y) => Ok(()),
        Anchor::Point(_) => Err(OpError::AnchorOutOfRange),
    }
}

/// The display-only outline of a region after its edit's op: the destination
/// ghost of a translate, or the transformed outline of a scale, rotate or
/// mirror (spec §Output, §Operations).
///
/// A ghost is for drawing only. It may extend outside the image, it is not a
/// [`Region`], and nothing in this crate converts it into one: a moved region
/// can't be selected, and no edit can be stacked on another edit's result.
#[derive(Debug, Clone, PartialEq)]
pub struct Ghost {
    outline: Vec<Point>,
}

impl Ghost {
    /// The closed outline in normalized coordinates (may fall outside
    /// `0.0..=1.0`). It has the same vertex order as
    /// [`Region::outline`].
    pub fn outline(&self) -> &[Point] {
        &self.outline
    }

    /// The outline in pixel coordinates.
    pub fn outline_pixels(&self, size: ImageSize) -> Vec<(f64, f64)> {
        self.outline.iter().map(|&p| size.to_pixels(p)).collect()
    }

    /// The center of the outline's bounding box, e.g. the head of a
    /// translate's arrow (whose tail is the source region's center).
    pub fn center(&self) -> Point {
        let (mut min, mut max) = (self.outline[0], self.outline[0]);
        for q in &self.outline[1..] {
            min = Point::new(min.x.min(q.x), min.y.min(q.y));
            max = Point::new(max.x.max(q.x), max.y.max(q.y));
        }
        Point::new((min.x + max.x) / 2.0, (min.y + max.y) / 2.0)
    }
}

/// Why an op's parameters were rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpError {
    /// A parameter was `NaN` or infinite.
    NotFinite,
    /// A scale factor was zero or negative (use mirror to flip).
    NonPositiveScale,
    /// A translate offset was outside `-1.0..=1.0`, or a scale factor was
    /// above [`MAX_SCALE`].
    OutOfRange,
    /// An [`Anchor::Point`] was outside the image (`0.0..=1.0` on either
    /// axis). Clamp it (e.g. with [`Point::clamped`]) before passing it in.
    AnchorOutOfRange,
}

impl fmt::Display for OpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NotFinite => "operation parameter is not a finite number",
            Self::NonPositiveScale => "scale factors must be positive",
            Self::OutOfRange => "translate offset outside -1..1 or scale factor above 100",
            Self::AnchorOutOfRange => "anchor point outside the image (0..1)",
        })
    }
}

impl std::error::Error for OpError {}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f64 = 1e-9;

    fn close(a: Point, b: Point) -> bool {
        (a.x - b.x).abs() < EPS && (a.y - b.y).abs() < EPS
    }

    fn rect() -> Region {
        Region::rect(Point::new(0.2, 0.2), Point::new(0.4, 0.3)).unwrap()
    }

    fn square_image() -> ImageSize {
        ImageSize::new(100, 100).unwrap()
    }

    #[test]
    fn validate_rejects_bad_parameters() {
        assert_eq!(
            Op::Translate {
                dx: f64::NAN,
                dy: 0.0
            }
            .validate(),
            Err(OpError::NotFinite)
        );
        assert_eq!(
            Op::uniform_scale(0.0).validate(),
            Err(OpError::NonPositiveScale)
        );
        assert_eq!(
            Op::Rotate {
                degrees: f64::INFINITY,
                anchor: Anchor::Center
            }
            .validate(),
            Err(OpError::NotFinite)
        );
        assert!(Op::Hint.validate().is_ok());
        assert!(Op::uniform_scale(2.0).validate().is_ok());
    }

    #[test]
    fn validate_bounds_magnitudes() {
        let t = |dx, dy| Op::Translate { dx, dy }.validate();
        let s = |sx, sy| {
            Op::Scale {
                sx,
                sy,
                anchor: Anchor::Center,
            }
            .validate()
        };
        let r = |degrees| {
            Op::Rotate {
                degrees,
                anchor: Anchor::Center,
            }
            .validate()
        };
        // Translate: within the full image size either way.
        for (dx, dy) in [(1.0, -1.0), (-1.0, 1.0), (0.0, 0.0), (0.5, -0.25)] {
            assert!(t(dx, dy).is_ok());
        }
        for (dx, dy) in [(-1e18, 0.0), (0.0, 1e18), (1.0001, 0.0), (0.0, -1.5)] {
            assert_eq!(t(dx, dy), Err(OpError::OutOfRange));
        }
        assert_eq!(t(f64::NEG_INFINITY, 0.0), Err(OpError::NotFinite));
        // Scale: positive and at most MAX_SCALE.
        assert!(s(MAX_SCALE, 0.01).is_ok());
        assert!(s(f64::MIN_POSITIVE, 1.0).is_ok());
        assert_eq!(s(100.5, 1.0), Err(OpError::OutOfRange));
        assert_eq!(s(1.0, 1e300), Err(OpError::OutOfRange));
        assert_eq!(s(0.0, 1.0), Err(OpError::NonPositiveScale));
        assert_eq!(s(1.0, -0.0), Err(OpError::NonPositiveScale));
        assert_eq!(s(f64::NAN, 1.0), Err(OpError::NotFinite));
        // Rotate: any finite angle.
        assert!(r(-720.0).is_ok() && r(1e6).is_ok());
        assert_eq!(r(f64::NAN), Err(OpError::NotFinite));
    }

    #[test]
    fn validate_requires_anchor_point_inside_image() {
        let rotate = |x, y| Op::Rotate {
            degrees: 10.0,
            anchor: Anchor::Point(Point::new(x, y)),
        };
        let scale = |x, y| Op::Scale {
            sx: 2.0,
            sy: 2.0,
            anchor: Anchor::Point(Point::new(x, y)),
        };
        for (x, y) in [(-0.01, 0.5), (0.5, 1.01), (-50.0, 9e9)] {
            assert_eq!(rotate(x, y).validate(), Err(OpError::AnchorOutOfRange));
            assert_eq!(scale(x, y).validate(), Err(OpError::AnchorOutOfRange));
        }
        assert_eq!(rotate(f64::NAN, 0.5).validate(), Err(OpError::NotFinite));
        assert_eq!(
            scale(0.5, f64::INFINITY).validate(),
            Err(OpError::NotFinite)
        );
        // The edges are inside the image.
        for (x, y) in [(0.0, 0.0), (1.0, 1.0), (0.25, 0.75)] {
            assert!(rotate(x, y).validate().is_ok());
            assert!(scale(x, y).validate().is_ok());
        }
    }

    #[test]
    fn translate_ghost_is_offset_outline_and_may_leave_image() {
        let g = Op::Translate { dx: 0.7, dy: -0.1 }
            .ghost(&rect(), square_image())
            .unwrap();
        assert!(close(g.outline()[0], Point::new(0.9, 0.1)));
        assert!(close(g.outline()[2], Point::new(1.1, 0.2)));
        assert!(close(g.center(), Point::new(1.0, 0.15)));
    }

    #[test]
    fn scale_ghost_about_center() {
        let g = Op::uniform_scale(2.0)
            .ghost(&rect(), square_image())
            .unwrap();
        assert!(close(g.outline()[0], Point::new(0.1, 0.15)));
        assert!(close(g.outline()[2], Point::new(0.5, 0.35)));
    }

    #[test]
    fn mirror_ghost_reflects_polygon() {
        let tri = Region::polygon([
            Point::new(0.2, 0.2),
            Point::new(0.4, 0.2),
            Point::new(0.2, 0.4),
        ])
        .unwrap();
        let g = Op::Mirror {
            axis: MirrorAxis::Horizontal,
        }
        .ghost(&tri, square_image())
        .unwrap();
        // The ghost is the lasso's flattened curve, reflected across the
        // vertical line through its bounding-box center.
        let c = tri.center();
        let outline = tri.outline();
        assert_eq!(g.outline().len(), outline.len());
        for (got, p) in g.outline().iter().zip(&outline) {
            assert!(close(*got, Point::new(2.0 * c.x - p.x, p.y)));
        }
        // The first control point is the outline's first point.
        assert!(close(g.outline()[0], Point::new(2.0 * c.x - 0.2, 0.2)));
        assert_eq!(MirrorAxis::Horizontal.toggled(), MirrorAxis::Vertical);
    }

    #[test]
    fn rotate_ghost_is_clockwise_in_pixel_space() {
        // 200x100 image; a 90° clockwise turn of the top-left corner about the
        // center must stay a rigid rotation in pixels.
        let size = ImageSize::new(200, 100).unwrap();
        let g = Op::Rotate {
            degrees: 90.0,
            anchor: Anchor::Center,
        }
        .ghost(&rect(), size)
        .unwrap();
        // Rect in pixels: (40,20)-(80,30), center (60,25). Top-left (40,20)
        // is offset (-20,-5); clockwise 90° maps (x,y) -> (-y,x) = (5,-20),
        // so (65,5) in pixels.
        let (px, py) = size.to_pixels(g.outline()[0]);
        assert!((px - 65.0).abs() < 1e-6 && (py - 5.0).abs() < 1e-6);
    }

    #[test]
    fn hint_and_label_have_no_ghost() {
        assert!(Op::Hint.ghost(&rect(), square_image()).is_none());
        assert!(Op::Label.ghost(&rect(), square_image()).is_none());
        assert!(Op::Label.validate().is_ok());
    }

    #[test]
    fn op_names() {
        let names: Vec<_> = [
            Op::Translate { dx: 0.0, dy: 0.0 },
            Op::uniform_scale(1.0),
            Op::Mirror {
                axis: MirrorAxis::Vertical,
            },
            Op::Rotate {
                degrees: 0.0,
                anchor: Anchor::Center,
            },
            Op::Hint,
            Op::Label,
        ]
        .iter()
        .map(Op::name)
        .collect();
        assert_eq!(
            names,
            ["translate", "scale", "mirror", "rotate", "hint", "label"]
        );
    }
}
