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

    /// The badge's horizontal extent.
    fn x_span(&self) -> (f64, f64) {
        (
            self.center.0 - self.size.0 / 2.0,
            self.center.0 + self.size.0 / 2.0,
        )
    }

    /// Whether the badge would read as labelling something other than
    /// `region`: most of its width lies outside the region's horizontal
    /// extent, or it shares more of its width with another region's
    /// horizontal extent than with its own. Only regions vertically near
    /// the badge (within one badge height) count, so a region far above or
    /// below in the same column doesn't.
    fn misaligned(&self, region: &PixelBox, others: &[PixelBox]) -> bool {
        let (a0, a1) = self.x_span();
        let shared = |b: &PixelBox| (a1.min(b.x1) - a0.max(b.x0)).max(0.0);
        let own = shared(region);
        if own < (a1 - a0) / 2.0 - EPS {
            return true;
        }
        let (h, cy) = (self.size.1, self.center.1);
        let (t, b) = (cy - 1.5 * h, cy + 1.5 * h);
        others
            .iter()
            .filter(|o| o.y0 < b && o.y1 > t)
            .any(|o| shared(o) > own + EPS)
    }

    /// Whether the whole badge is on the image.
    pub(crate) fn inside(&self, size: ImageSize) -> bool {
        let (w, h) = self.size;
        let (cx, cy) = self.center;
        cx - w / 2.0 >= -EPS
            && cy - h / 2.0 >= -EPS
            && cx + w / 2.0 <= f64::from(size.width()) + EPS
            && cy + h / 2.0 <= f64::from(size.height()) + EPS
    }
}

/// Places one badge of `badge` = (width, height) image pixels for a region
/// whose bounding box is `region`. `named` says the badge carries the
/// region's name (`1 logo`), so it is a wide capsule rather than a round
/// number.
///
/// A round badge tries these spots, in order, each touching the region's
/// outline without covering it:
///
/// 1. diagonally up and left of the top-left corner, the capsule's right end
///    tangent to the corner;
/// 2. above the top edge, flush with the left edge;
/// 3. left of the left edge, flush with the top edge;
/// 4. right of the right edge, flush with the top edge;
/// 5. below the bottom edge, flush with the left edge.
///
/// A named badge is wide, and on the diagonal (or to the left) most of it
/// would hang over whatever is left of the region: on a row of cards,
/// `3 third card` sat over the middle card and read as its label. So a
/// named badge tries "above, flush left" first, then "above, flush right"
/// (its right end on the region's right edge, for a region near the
/// image's right edge), then just inside the top-left corner (only on a
/// region big enough that it covers a corner, not the region), then the
/// round badge's spots 1 and 3–5, then "below, flush right". And a spot
/// whose horizontal extent lies mostly outside the region's, or that
/// overlaps another nearby region's horizontal extent more than its own,
/// is *misaligned* and only used when no aligned spot fits.
///
/// A candidate must lie wholly on the image and must not overlap a badge in
/// `placed`. Among those, the first aligned one that also stays off every
/// box in `others` (the other regions) wins; then the first aligned one
/// that only overlaps other regions; then misaligned ones in the same
/// order. If no candidate fits (a region filling the image, or one in a
/// corner of the image), the badge falls back to the old position:
/// centered on the top-left corner, pushed inside the image.
pub fn badge_position(
    region: PixelBox,
    badge: (f64, f64),
    named: bool,
    image: ImageSize,
    others: &[PixelBox],
    placed: &[BadgePlacement],
) -> BadgePlacement {
    let (w, h) = badge;
    let r = h / 2.0;
    let half = (w / 2.0 - r).max(0.0);
    let d = r * std::f64::consts::FRAC_1_SQRT_2;
    let PixelBox { x0, y0, x1, y1 } = region;
    let diagonal = (x0 - half - d, y0 - d);
    let above = (x0 + w / 2.0, y0 - r);
    let left = (x0 - w / 2.0, y0 + r);
    let right = (x1 + w / 2.0, y0 + r);
    let below = (x0 + w / 2.0, y1 + r);
    // (center, outside the region?)
    let candidates: Vec<((f64, f64), bool)> = if named {
        // Flush right: the same spots ending at the region's right edge, for
        // a region near the image's right edge.
        let above_right = (x1 - w / 2.0, y0 - r);
        let below_right = (x1 - w / 2.0, y1 + r);
        let mut c = vec![(above, true), (above_right, true)];
        // Inside the top-left corner, only where the badge covers at most a
        // corner: no wider than the region, at most a third of its height.
        if w <= x1 - x0 + EPS && 3.0 * h <= y1 - y0 + EPS {
            c.push(((x0 + w / 2.0, y0 + r), false));
        }
        c.extend([diagonal, left, right, below, below_right].map(|p| (p, true)));
        c
    } else {
        [diagonal, above, left, right, below]
            .map(|p| (p, true))
            .to_vec()
    };
    // Lower is better: (misaligned, covers another region).
    let mut best: Option<((bool, bool), BadgePlacement)> = None;
    for (center, outside) in candidates {
        let p = BadgePlacement {
            center,
            size: badge,
            outside,
        };
        if !p.inside(image)
            || (outside && p.overlaps_box(&region))
            || placed.iter().any(|q| p.overlaps_badge(q))
        {
            continue;
        }
        let key = (
            named && p.misaligned(&region, others),
            others.iter().any(|b| p.overlaps_box(b)),
        );
        if key == (false, false) {
            return p;
        }
        if best.is_none_or(|(k, _)| key < k) {
            best = Some((key, p));
        }
    }
    best.map(|(_, p)| p).unwrap_or_else(|| {
        let (iw, ih) = (f64::from(image.width()), f64::from(image.height()));
        let fit = |v: f64, half: f64, max: f64| v.clamp(half, (max - half).max(half));
        BadgePlacement {
            center: (fit(x0, w / 2.0, iw), fit(y0, h / 2.0, ih)),
            size: badge,
            outside: false,
        }
    })
}

/// The text of edit `number`'s badge: the number, then the region's name
/// if it has one (`1 logo`), so the mark itself carries the name a caption
/// row or a reply refers to. [`fit_badge_label`] shortens it to fit.
pub fn badge_label(number: usize, name: Option<&str>) -> String {
    match name {
        Some(name) => format!("{number} {name}"),
        None => number.to_string(),
    }
}

/// The widest a named badge may be, ring included, on an image
/// `image_width` wide whose badges are `badge_height` tall (both in the
/// same units): 40% of the image width, but at least 6 and at most 12
/// badge heights, and never wider than the image.
///
/// A badge is a mark, not a caption: past this width its far end sits so
/// far from the region it labels that it reads as a banner over the
/// image. The caption row always carries the full name.
pub fn badge_max_width(image_width: f64, badge_height: f64) -> f64 {
    (0.4 * image_width)
        .clamp(6.0 * badge_height, 12.0 * badge_height)
        .min(image_width)
}

/// [`badge_label`], with the name cut short and ended with `…` so the
/// badge is at most `max_width` wide. `width(label)` is the full badge
/// width of a label, in the units of `max_width`; the caller measures with
/// its own font (the export's, or the canvas's). The number always stays;
/// a name that can't keep a single letter becomes `1 …`.
pub fn fit_badge_label(
    number: usize,
    name: Option<&str>,
    max_width: f64,
    mut width: impl FnMut(&str) -> f64,
) -> String {
    let full = badge_label(number, name);
    let Some(name) = name else { return full };
    if width(&full) <= max_width {
        return full;
    }
    let chars: Vec<char> = name.chars().collect();
    let cut = |k: usize| {
        let head: String = chars[..k].iter().collect();
        format!("{number} {}\u{2026}", head.trim_end())
    };
    // The longest prefix that fits: `cut(lo)` fits (or lo is 0), and
    // `cut(hi)` doesn't (the full name didn't).
    let (mut lo, mut hi) = (0, chars.len());
    while hi - lo > 1 {
        let mid = usize::midpoint(lo, hi);
        if width(&cut(mid)) <= max_width {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    cut(lo)
}

/// Places the badge of every edit in `doc`, in order, with
/// [`badge_position`]: each badge avoids the earlier edits' badges and,
/// where it can, every other edit's region.
///
/// `badge(number, name)` gives the full (width, height) in image pixels of
/// the badge for edit `number` (1-based, as printed on it) whose region is
/// named `name`: a named region's badge also shows its name (`1 logo`), so
/// it is wider, and the placement keeps the whole capsule clear. The result
/// has one placement per edit, in the same order.
pub fn badge_positions(
    doc: &Document,
    mut badge: impl FnMut(usize, Option<&str>) -> (f64, f64),
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
    for (i, (&region, edit)) in boxes.iter().zip(doc.edits()).enumerate() {
        others.clear();
        others.extend(
            boxes
                .iter()
                .enumerate()
                .filter(|&(j, _)| j != i)
                .map(|(_, b)| *b),
        );
        let name = edit.name();
        let p = badge_position(
            region,
            badge(i + 1, name),
            name.is_some(),
            size,
            &others,
            &placed,
        );
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
        let p = badge_position(region, BADGE, false, size(400, 300), &[], &[]);
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
        let p = badge_position(region, BADGE, false, size(400, 300), &[], &[]);
        assert!(p.outside);
        assert_eq!(p.center, (86.0, 14.0));
        assert!(clear_of(&p, &region));
    }

    #[test]
    fn region_at_the_left_edge_puts_the_badge_above() {
        let region = PixelBox::new(0.0, 100.0, 50.0, 140.0);
        let p = badge_position(region, BADGE, false, size(400, 300), &[], &[]);
        assert!(p.outside);
        assert_eq!(p.center, (14.0, 86.0));
    }

    #[test]
    fn region_in_the_image_corner_goes_right_of_it() {
        let region = PixelBox::new(0.0, 0.0, 50.0, 30.0);
        let p = badge_position(region, BADGE, false, size(400, 300), &[], &[]);
        assert!(p.outside);
        assert_eq!(p.center, (64.0, 14.0));
        assert!(clear_of(&p, &region));
    }

    #[test]
    fn wide_badges_keep_their_right_end_on_the_corner() {
        let region = PixelBox::new(100.0, 100.0, 120.0, 120.0);
        let p = badge_position(region, (44.0, 28.0), false, size(400, 300), &[], &[]);
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
        let pa = badge_position(a, BADGE, false, size(400, 300), &[b], &[]);
        let pb = badge_position(b, BADGE, false, size(400, 300), &[a], &[pa]);
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
        let p = badge_position(region, BADGE, false, s, &[], &[]);
        assert!(!p.outside);
        assert_eq!(p.center, (14.0, 14.0));
        assert!(p.inside(s));
    }

    #[test]
    fn badge_bigger_than_the_image_is_pinned_at_its_half_size() {
        let p = badge_position(
            PixelBox::new(0.0, 0.0, 16.0, 16.0),
            BADGE,
            false,
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
        let p = badge_position(small, BADGE, false, size(400, 300), &[big], &[]);
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
        let a = badge_positions(&doc, |_, _| BADGE);
        let b = badge_positions(&doc, |_, _| BADGE);
        assert_eq!(a, b);
        assert_eq!(a.len(), 3);
        assert!(a[0].outside && a[1].outside && !a[2].outside);
        assert!(!a[0].overlaps_badge(&a[1]));
    }

    #[test]
    fn a_named_badge_is_placed_by_its_full_width() {
        // A named region near the left edge: the wide `1 logo` capsule goes
        // above, flush left, and clears its own region and its neighbour's
        // badge.
        let s = size(400, 300);
        let mut ed = Editor::new(s);
        let rect = |x0, y0, x1, y1| Region::rect(Point::new(x0, y0), Point::new(x1, y1)).unwrap();
        ed.add_named_edit(
            rect(0.05, 0.3, 0.2, 0.4),
            Op::Label,
            None,
            Some("logo".into()),
        )
        .unwrap();
        ed.add_edit(rect(0.3, 0.3, 0.4, 0.4), Op::Hint, Some("b".into()))
            .unwrap();
        let doc = ed.into_document();
        let mut seen = Vec::new();
        let wide = (90.0, 28.0);
        let p = badge_positions(&doc, |n, name| {
            seen.push((n, name.map(str::to_owned)));
            if name.is_some() { wide } else { BADGE }
        });
        assert_eq!(seen, [(1, Some("logo".to_owned())), (2, None)]);
        assert_eq!(p[0].size, wide);
        assert!(p[0].outside && p[0].inside(s), "{:?}", p[0]);
        let boxes = [
            PixelBox::new(20.0, 90.0, 80.0, 120.0),
            PixelBox::new(120.0, 90.0, 160.0, 120.0),
        ];
        for b in &boxes {
            assert!(clear_of(&p[0], b), "{:?} covers {b:?}", p[0]);
        }
        assert!(!p[0].overlaps_badge(&p[1]));
    }

    /// The three cards of `docs/examples/export-named.png`: 1280×720, cards
    /// side by side 32 px apart, and a `3 third card` badge about half a
    /// card wide.
    fn cards() -> [PixelBox; 3] {
        [96.0, 468.0, 840.0].map(|x| PixelBox::new(x, 400.0, x + 340.0, 650.0))
    }

    #[test]
    fn a_wide_badge_stays_over_its_own_card() {
        // The diagonal spot put `3 third card` over the middle card, its
        // right tip on card 3's corner. Now it sits above card 3, flush
        // with its left edge.
        let s = size(1280, 720);
        let [a, b, c] = cards();
        let badge = (170.0, 43.0);
        let p = badge_position(c, badge, true, s, &[a, b], &[]);
        assert!(p.outside && clear_of(&p, &c), "{p:?}");
        let (l, r) = p.x_span();
        assert!((l - c.x0).abs() < 1e-9 && r <= c.x1, "{p:?}");
        assert!((p.center.1 - (c.y0 - 21.5)).abs() < 1e-9, "{p:?}");
        assert!(!p.misaligned(&c, &[a, b]));
        // The round badge for the same card keeps the diagonal.
        let q = badge_position(c, (43.0, 43.0), false, s, &[a, b], &[]);
        assert!(q.center.0 < c.x0 && q.center.1 < c.y0, "{q:?}");
    }

    #[test]
    fn a_wide_badge_with_no_room_above_goes_inside_or_below_not_sideways() {
        let s = size(1280, 720);
        let badge = (170.0, 43.0);
        // A big card at the top of the image: just inside its top-left
        // corner, still over its own span.
        let big = PixelBox::new(468.0, 0.0, 808.0, 250.0);
        let others = [PixelBox::new(96.0, 0.0, 436.0, 250.0)];
        let p = badge_position(big, badge, true, s, &others, &[]);
        assert!(!p.outside, "{p:?}");
        assert_eq!(p.center, (468.0 + 85.0, 21.5));
        // A thin strip at the top: too small to hold the badge, so below
        // it, flush left, rather than over the neighbour on its left.
        let strip = PixelBox::new(468.0, 0.0, 808.0, 60.0);
        let others = [PixelBox::new(96.0, 0.0, 436.0, 60.0)];
        let p = badge_position(strip, badge, true, s, &others, &[]);
        assert!(p.outside && clear_of(&p, &strip), "{p:?}");
        assert_eq!(p.center, (468.0 + 85.0, 60.0 + 21.5));
    }

    #[test]
    fn a_badge_more_than_twice_its_regions_width_starts_at_its_left_edge() {
        // A 60 px region with a 170 px badge: every spot is misaligned
        // (most of the badge is outside the region's span wherever it
        // goes), so the order decides and it goes above, flush left: it at
        // least starts where its region starts, not over the left
        // neighbour.
        let s = size(1280, 720);
        let narrow = PixelBox::new(400.0, 300.0, 460.0, 360.0);
        let left = PixelBox::new(100.0, 300.0, 390.0, 360.0);
        let p = badge_position(narrow, (170.0, 43.0), true, s, &[left], &[]);
        assert!(p.outside && clear_of(&p, &narrow) && clear_of(&p, &left));
        assert!(p.misaligned(&narrow, &[left]));
        assert_eq!(p.center, (400.0 + 85.0, 300.0 - 21.5));
    }

    #[test]
    fn misalignment_counts_only_regions_near_the_badge() {
        // The badge spans x 100..200; its region 150..400 holds half of it.
        let c = PixelBox::new(150.0, 300.0, 400.0, 400.0);
        let p = BadgePlacement {
            center: (150.0, 280.0),
            size: (100.0, 40.0),
            outside: true,
        };
        assert!(!p.misaligned(&c, &[]));
        // A region sharing 60 px of the badge's span, far below: ignored.
        let far = PixelBox::new(0.0, 600.0, 160.0, 700.0);
        assert!(!p.misaligned(&c, &[far]));
        // The same span right next to the badge: it shares more than the
        // badge's own region does, so the badge reads as that region's.
        let near = PixelBox::new(0.0, 200.0, 160.0, 260.0);
        assert!(p.misaligned(&c, &[near]));
        // A badge mostly left of its region is misaligned on its own.
        let off = BadgePlacement {
            center: (90.0, 280.0),
            ..p
        };
        assert!(off.misaligned(&c, &[]));
    }

    #[test]
    fn a_wide_badge_near_the_right_edge_goes_above_flush_right() {
        // Flush left would run off the image; flush right keeps the badge
        // over its own region instead of falling back to the diagonal over
        // whatever is left of it.
        let s = size(1200, 800);
        let region = PixelBox::new(750.0, 450.0, 1050.0, 700.0);
        let left = PixelBox::new(200.0, 250.0, 450.0, 450.0);
        let badge = (465.0, 50.0);
        let p = badge_position(region, badge, true, s, &[left], &[]);
        assert!(p.outside && p.inside(s) && clear_of(&p, &region), "{p:?}");
        assert!(!p.misaligned(&region, &[left]), "{p:?}");
        assert_eq!(p.center, (1050.0 - 232.5, 450.0 - 25.0));
    }
}
