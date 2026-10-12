//! Where each region's numbered badge goes.
//!
//! The badge used to be centered on the region's top-left corner, so on a
//! small region a quarter of it sat on the very pixels the edit is about
//! (#28). Now it goes **outside** the region whenever there is room, touching
//! the outline at the top-left corner, so it still reads as "this region's
//! number" (Set-of-Mark convention, spec §Output) without hiding what the
//! model has to look at.
//!
//! Both the export and the canvas call [`badge_positions`], so the two
//! always place badges by the same rule. Everything here is in image pixels
//! and pure: no drawing, no fonts, deterministic.

use crate::{Document, ImageSize};

/// An axis-aligned box in image pixels, `x0 <= x1`, `y0 <= y1`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PixelBox {
    /// Left edge.
    pub x0: f64,
    /// Top edge.
    pub y0: f64,
    /// Right edge.
    pub x1: f64,
    /// Bottom edge.
    pub y1: f64,
}

impl PixelBox {
    /// The box from (`x0`, `y0`) to (`x1`, `y1`).
    pub fn new(x0: f64, y0: f64, x1: f64, y1: f64) -> Self {
        Self { x0, y0, x1, y1 }
    }
}

/// Where a badge sits, in image pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BadgePlacement {
    /// The badge's center.
    pub center: (f64, f64),
    /// The badge's full width and height (a horizontal capsule, a circle when
    /// they are equal), including any ring around it.
    pub size: (f64, f64),
    /// `true` when the badge is outside its region, touching the outline;
    /// `false` for the fallback, centered on the region's top-left corner and
    /// pushed inside the image, which covers part of the region.
    pub outside: bool,
}

/// Slack for float error when testing touching shapes.
const EPS: f64 = 1e-6;

impl BadgePlacement {
    /// The capsule as a horizontal segment (its spine) and a radius.
    fn capsule(&self) -> ((f64, f64), f64, f64) {
        let (w, h) = self.size;
        let r = h / 2.0;
        let half = (w / 2.0 - r).max(0.0);
        (
            (self.center.0 - half, self.center.0 + half),
            self.center.1,
            r,
        )
    }

    /// Whether the badge overlaps the box's interior (touching is allowed).
    fn overlaps_box(&self, b: &PixelBox) -> bool {
        let ((a0, a1), cy, r) = self.capsule();
        let gx = (b.x0 - a1).max(a0 - b.x1).max(0.0);
        let gy = (b.y0 - cy).max(cy - b.y1).max(0.0);
        gx.hypot(gy) < r - EPS
    }

    /// Whether two badges overlap (touching is allowed).
    fn overlaps_badge(&self, other: &BadgePlacement) -> bool {
        let ((a0, a1), ay, ar) = self.capsule();
        let ((b0, b1), by, br) = other.capsule();
        let gx = (b0 - a1).max(a0 - b1).max(0.0);
        gx.hypot(ay - by) < ar + br - EPS
    }

    /// Whether the whole badge is on the image.
    fn inside(&self, size: ImageSize) -> bool {
        let (w, h) = self.size;
        let (cx, cy) = self.center;
        cx - w / 2.0 >= -EPS
            && cy - h / 2.0 >= -EPS
            && cx + w / 2.0 <= f64::from(size.width()) + EPS
            && cy + h / 2.0 <= f64::from(size.height()) + EPS
    }
}

/// Places one badge of `badge` = (width, height) image pixels for a region
/// whose bounding box is `region`.
///
/// Candidates, in order of preference, each touching the region's outline
/// without covering it:
///
/// 1. diagonally up and left of the top-left corner, the capsule's right end
///    tangent to the corner;
/// 2. above the top edge, flush with the left edge;
/// 3. left of the left edge, flush with the top edge;
/// 4. right of the right edge, flush with the top edge;
/// 5. below the bottom edge, flush with the left edge.
///
/// A candidate must lie wholly on the image and must not overlap a badge in
/// `placed`. Among those, the first that also stays off every box in
/// `others` (the other regions) wins; failing that, the first that only
/// overlaps other regions. If no candidate fits (a region filling the image,
/// or one in a corner of the image), the badge falls back to the old
/// position: centered on the top-left corner, pushed inside the image.
pub fn badge_position(
    region: PixelBox,
    badge: (f64, f64),
    image: ImageSize,
    others: &[PixelBox],
    placed: &[BadgePlacement],
) -> BadgePlacement {
    let (w, h) = badge;
    let r = h / 2.0;
    let half = (w / 2.0 - r).max(0.0);
    let d = r * std::f64::consts::FRAC_1_SQRT_2;
    let PixelBox { x0, y0, x1, y1 } = region;
    let candidates = [
        (x0 - half - d, y0 - d),
        (x0 + w / 2.0, y0 - r),
        (x0 - w / 2.0, y0 + r),
        (x1 + w / 2.0, y0 + r),
        (x0 + w / 2.0, y1 + r),
    ];
    let mut fallback_other = None;
    for center in candidates {
        let p = BadgePlacement {
            center,
            size: badge,
            outside: true,
        };
        if !p.inside(image) || p.overlaps_box(&region) || placed.iter().any(|q| p.overlaps_badge(q))
        {
            continue;
        }
        if !others.iter().any(|b| p.overlaps_box(b)) {
            return p;
        }
        fallback_other.get_or_insert(p);
    }
    fallback_other.unwrap_or_else(|| {
        let (iw, ih) = (f64::from(image.width()), f64::from(image.height()));
        let fit = |v: f64, half: f64, max: f64| v.clamp(half, (max - half).max(half));
        BadgePlacement {
            center: (fit(x0, w / 2.0, iw), fit(y0, h / 2.0, ih)),
            size: badge,
            outside: false,
        }
    })
}

/// Places the badge of every edit in `doc`, in order, with
/// [`badge_position`]: each badge avoids the earlier edits' badges and,
/// where it can, every other edit's region.
///
/// `badge(number)` gives the full (width, height) in image pixels of the
/// badge for edit `number` (1-based, as printed on it). The result has one
/// placement per edit, in the same order.
pub fn badge_positions(
    doc: &Document,
    mut badge: impl FnMut(usize) -> (f64, f64),
) -> Vec<BadgePlacement> {
    let size = doc.image_size();
    let boxes: Vec<PixelBox> = doc
        .edits()
        .iter()
        .map(|e| {
            let b = e.region().bounds();
            let (x0, y0) = size.to_pixels(b.min());
            let (x1, y1) = size.to_pixels(b.max());
            PixelBox::new(x0, y0, x1, y1)
        })
        .collect();
    let mut placed = Vec::with_capacity(boxes.len());
    let mut others = Vec::with_capacity(boxes.len());
    for (i, &region) in boxes.iter().enumerate() {
        others.clear();
        others.extend(
            boxes
                .iter()
                .enumerate()
                .filter(|&(j, _)| j != i)
                .map(|(_, b)| *b),
        );
        let p = badge_position(region, badge(i + 1), size, &others, &placed);
        placed.push(p);
    }
    placed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Editor, Op, Point, Region};

    const BADGE: (f64, f64) = (28.0, 28.0);

    fn size(w: u32, h: u32) -> ImageSize {
        ImageSize::new(w, h).unwrap()
    }

    /// The badge's disc and the box's interior share no pixel: every point
    /// of the capsule is at least `r` from the box, up to float slack.
    fn clear_of(p: &BadgePlacement, b: &PixelBox) -> bool {
        !p.overlaps_box(b)
    }

    #[test]
    fn small_region_near_a_corner_gets_the_badge_outside() {
        // A 32 px region (about one badge) 40 px in from the top-left.
        let region = PixelBox::new(40.0, 40.0, 72.0, 72.0);
        let p = badge_position(region, BADGE, size(400, 300), &[], &[]);
        assert!(p.outside);
        assert!(clear_of(&p, &region));
        assert!(p.inside(size(400, 300)));
        // Up and left of the corner, its disc touching the corner.
        assert!(p.center.0 < 40.0 && p.center.1 < 40.0, "{p:?}");
        let dist = (40.0 - p.center.0).hypot(40.0 - p.center.1);
        assert!((dist - 14.0).abs() < 1e-9, "disc should touch the corner");
    }

    #[test]
    fn region_at_the_top_edge_puts_the_badge_to_the_left() {
        let region = PixelBox::new(100.0, 0.0, 160.0, 40.0);
        let p = badge_position(region, BADGE, size(400, 300), &[], &[]);
        assert!(p.outside);
        assert_eq!(p.center, (86.0, 14.0));
        assert!(clear_of(&p, &region));
    }

    #[test]
    fn region_at_the_left_edge_puts_the_badge_above() {
        let region = PixelBox::new(0.0, 100.0, 50.0, 140.0);
        let p = badge_position(region, BADGE, size(400, 300), &[], &[]);
        assert!(p.outside);
        assert_eq!(p.center, (14.0, 86.0));
    }

    #[test]
    fn region_in_the_image_corner_goes_right_of_it() {
        let region = PixelBox::new(0.0, 0.0, 50.0, 30.0);
        let p = badge_position(region, BADGE, size(400, 300), &[], &[]);
        assert!(p.outside);
        assert_eq!(p.center, (64.0, 14.0));
        assert!(clear_of(&p, &region));
    }

    #[test]
    fn wide_badges_keep_their_right_end_on_the_corner() {
        let region = PixelBox::new(100.0, 100.0, 120.0, 120.0);
        let p = badge_position(region, (44.0, 28.0), size(400, 300), &[], &[]);
        assert!(p.outside && clear_of(&p, &region));
        // The right end's disc center is d = r/√2 from the corner on both axes.
        let d = 14.0 * std::f64::consts::FRAC_1_SQRT_2;
        assert!((p.center.0 + 8.0 - (100.0 - d)).abs() < 1e-9, "{p:?}");
        assert!((p.center.1 - (100.0 - d)).abs() < 1e-9, "{p:?}");
    }

    #[test]
    fn adjacent_regions_get_badges_that_do_not_overlap() {
        // Two small regions side by side, 4 px apart: the second one's
        // preferred spot would cover the first one's badge or region.
        let a = PixelBox::new(60.0, 60.0, 90.0, 90.0);
        let b = PixelBox::new(94.0, 60.0, 124.0, 90.0);
        let pa = badge_position(a, BADGE, size(400, 300), &[b], &[]);
        let pb = badge_position(b, BADGE, size(400, 300), &[a], &[pa]);
        assert!(pa.outside && pb.outside);
        assert!(!pa.overlaps_badge(&pb), "{pa:?} {pb:?}");
        for p in [&pa, &pb] {
            assert!(clear_of(p, &a) && clear_of(p, &b), "{p:?}");
        }
    }

    #[test]
    fn full_image_region_falls_back_inside() {
        let s = size(400, 300);
        let region = PixelBox::new(0.0, 0.0, 400.0, 300.0);
        let p = badge_position(region, BADGE, s, &[], &[]);
        assert!(!p.outside);
        assert_eq!(p.center, (14.0, 14.0));
        assert!(p.inside(s));
    }

    #[test]
    fn badge_bigger_than_the_image_is_pinned_at_its_half_size() {
        let p = badge_position(
            PixelBox::new(0.0, 0.0, 16.0, 16.0),
            BADGE,
            size(16, 16),
            &[],
            &[],
        );
        assert!(!p.outside);
        assert_eq!(p.center, (14.0, 14.0));
    }

    #[test]
    fn nested_region_prefers_outside_even_over_its_container() {
        // A small region inside a big one: every outside spot overlaps the
        // big region's box, so the first fitting one is used anyway rather
        // than covering the small region.
        let big = PixelBox::new(20.0, 20.0, 380.0, 280.0);
        let small = PixelBox::new(100.0, 100.0, 120.0, 120.0);
        let p = badge_position(small, BADGE, size(400, 300), &[big], &[]);
        assert!(p.outside && clear_of(&p, &small));
        assert!(p.center.0 < 100.0 && p.center.1 < 100.0);
    }

    #[test]
    fn positions_for_a_document_are_deterministic_and_one_per_edit() {
        let s = size(400, 300);
        let mut ed = Editor::new(s);
        let rect = |x0, y0, x1, y1| Region::rect(Point::new(x0, y0), Point::new(x1, y1)).unwrap();
        ed.add_edit(rect(0.1, 0.1, 0.15, 0.15), Op::Hint, Some("a".into()))
            .unwrap();
        ed.add_edit(rect(0.16, 0.1, 0.21, 0.15), Op::Hint, Some("b".into()))
            .unwrap();
        ed.add_edit(rect(0.0, 0.0, 1.0, 1.0), Op::Hint, Some("c".into()))
            .unwrap();
        let doc = ed.into_document();
        let a = badge_positions(&doc, |_| BADGE);
        let b = badge_positions(&doc, |_| BADGE);
        assert_eq!(a, b);
        assert_eq!(a.len(), 3);
        assert!(a[0].outside && a[1].outside && !a[2].outside);
        assert!(!a[0].overlaps_badge(&a[1]));
    }
}
