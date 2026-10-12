//! Sizing the stage to the image, and choosing where the history panel goes
//! (#69). Pure, so it is tested natively; `layout::measure_space` feeds it
//! from the DOM and the canvas view applies the result.
//!
//! The stage hugs the image: its size is the image at the chosen scale plus
//! [`Chrome::margin`] on every side (room for badges and handles at the
//! image edge), so there are no large letterbox bars. The scale is the
//! largest that fits the available space, capped by [`max_scale`] so small
//! images are never blown up into blur.
//!
//! Two arrangements are tried, and the one that shows the image larger
//! wins:
//!
//! - [`Arrangement::Side`]: the history panel beside the canvas column.
//!   The stage loses the panel's width, keeps the full height. Only offered
//!   when the canvas column keeps at least [`Chrome::min_side_main`] pixels,
//!   so phones and narrow windows always get the panel below.
//! - [`Arrangement::Below`]: the panel under the canvas column. The stage
//!   gets the full width but, when `Side` was on offer too, gives up
//!   [`Chrome::panel_peek`] of height, so the panel's header and first row
//!   stay in view instead of the panel hiding below the fold. When `Below`
//!   is the only choice (a phone), there is no peek: the panel is a scroll
//!   away either way, and the image gets the height.
//!
//! Near the crossover a small hysteresis ([`Chrome::hysteresis`]) keeps
//! the current arrangement, so resizing the window doesn't flip it back and
//! forth. Ties go to `Side`, which keeps the panel visible.

/// Where the history panel sits relative to the canvas column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Arrangement {
    /// Beside the canvas column.
    #[default]
    Side,
    /// Below the canvas column.
    Below,
}

impl Arrangement {
    /// The CSS class `.workspace` carries for this arrangement.
    pub fn class(self) -> &'static str {
        match self {
            Self::Side => "workspace side",
            Self::Below => "workspace below",
        }
    }
}

/// The space the workspace can use, measured from the DOM in CSS pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Space {
    /// The workspace's content width (the page width less its padding).
    pub width: f64,
    /// From the workspace's top to the bottom of the viewport, less the
    /// page's bottom padding: the height the canvas column may fill without
    /// scrolling.
    pub height: f64,
    /// What is stacked under the stage in its column (the gap and the
    /// instructions box), which also has to fit in `height`.
    pub under_stage: f64,
}

/// Fixed sizes around the stage, in CSS pixels. [`Chrome::DEFAULT`] matches
/// `style.css`; keep the two in step.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Chrome {
    /// Space between the image and the stage edge, on every side.
    pub margin: f64,
    /// The stage's border width (CSS `border: 1px`), on every side.
    pub border: f64,
    /// The history panel's width beside the canvas (`.workspace.side
    /// .history-panel { min-width }`).
    pub panel_side_w: f64,
    /// The gap between the canvas column and the panel (`.workspace { gap }`).
    pub gap: f64,
    /// Height kept free for the panel's header and first row, gap included,
    /// when the panel is below by choice (not on a phone).
    pub panel_peek: f64,
    /// The narrowest canvas column `Side` is offered with.
    pub min_side_main: f64,
    /// The stage's content area is never laid out shorter than this, even
    /// in a very short window (the page scrolls instead).
    pub min_stage_h: f64,
    /// How much larger (as a fraction) the other arrangement's scale must
    /// be before the current one is given up.
    pub hysteresis: f64,
}

impl Chrome {
    /// The sizes `style.css` uses.
    pub const DEFAULT: Self = Self {
        margin: 24.0,
        border: 1.0,
        panel_side_w: 300.0,
        gap: 16.0,
        panel_peek: 96.0,
        min_side_main: 360.0,
        min_stage_h: 200.0,
        hysteresis: 0.05,
    };
}

/// The chosen layout.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layout {
    /// Where the history panel goes.
    pub arrangement: Arrangement,
    /// The stage's content size in whole CSS pixels (its border is outside
    /// this).
    pub stage_w: u32,
    /// See `stage_w`.
    pub stage_h: u32,
    /// Screen pixels per image pixel at this size.
    pub scale: f64,
}

/// Images whose longer side is at most this many pixels may be shown at
/// 2x (see [`max_scale`]).
pub const TINY_IMAGE: u32 = 256;

/// The largest display scale for an image: 1 CSS pixel per image pixel, so
/// a screenshot is never upscaled into blur. Tiny images (longer side at
/// most [`TINY_IMAGE`]) may go to 2x: at 1x an icon-sized crop is too small
/// to draw regions on, and an integer 2x with smoothing off (the canvas
/// view turns it off at scale >= 1) stays pixel-crisp. The cap is a whole
/// number on purpose, so upscaled pixels are all the same size.
pub fn max_scale(image_w: u32, image_h: u32) -> f64 {
    if image_w.max(image_h) <= TINY_IMAGE {
        2.0
    } else {
        1.0
    }
}

/// Fits the image in a `box_w` x `box_h` content box: the scale, then the
/// stage size that hugs the image at that scale (rounded down, so the stage
/// never exceeds the box and the scale the view recomputes from it never
/// exceeds this one).
fn fit_in(image: (f64, f64), box_w: f64, box_h: f64, cap: f64, margin: f64) -> (f64, u32, u32) {
    let (iw, ih) = image;
    let scale = ((box_w - 2.0 * margin) / iw)
        .min((box_h - 2.0 * margin) / ih)
        .min(cap)
        .max(f64::MIN_POSITIVE);
    // Whole pixels, at least the margins plus one pixel of image.
    let side = |len: f64| ((len * scale + 2.0 * margin).floor().max(2.0 * margin + 1.0)) as u32;
    (scale, side(iw), side(ih))
}

/// Chooses the arrangement and stage size that show an `image_w` x
/// `image_h` image largest in `space`. `current` is the arrangement on
/// screen now, for the hysteresis; pass `None` for a new image.
pub fn choose_layout(
    image_w: u32,
    image_h: u32,
    space: Space,
    chrome: Chrome,
    current: Option<Arrangement>,
) -> Layout {
    let image = (f64::from(image_w.max(1)), f64::from(image_h.max(1)));
    let cap = max_scale(image_w, image_h);
    let border = 2.0 * chrome.border;
    let column_h = (space.height - space.under_stage).max(0.0);
    let fit = |arrangement, outer_w: f64, outer_h: f64| {
        let box_h = (outer_h - border).max(chrome.min_stage_h);
        let (scale, stage_w, stage_h) = fit_in(
            image,
            (outer_w - border).max(1.0),
            box_h,
            cap,
            chrome.margin,
        );
        Layout {
            arrangement,
            stage_w,
            stage_h,
            scale,
        }
    };

    let side_main = space.width - chrome.gap - chrome.panel_side_w;
    if side_main < chrome.min_side_main {
        return fit(Arrangement::Below, space.width, column_h);
    }
    let below = fit(
        Arrangement::Below,
        space.width,
        column_h - chrome.panel_peek,
    );
    let side = fit(Arrangement::Side, side_main, column_h);
    let keep = 1.0 + chrome.hysteresis;
    let side_wins = match current {
        Some(Arrangement::Below) => side.scale > below.scale * keep,
        Some(Arrangement::Side) => side.scale * keep >= below.scale,
        None => side.scale >= below.scale,
    };
    if side_wins { side } else { below }
}

#[cfg(test)]
mod tests {
    use super::*;

    const C: Chrome = Chrome::DEFAULT;

    /// A window of `vw` x `vh` CSS pixels, with chrome above the workspace
    /// and the instructions box under the stage like the real page.
    fn space(vw: f64, vh: f64) -> Space {
        let page_pad = 16.0;
        let above = 150.0;
        Space {
            width: vw - 2.0 * page_pad,
            height: vh - above - page_pad,
            under_stage: 90.0,
        }
    }

    fn pick(w: u32, h: u32, s: Space) -> Layout {
        choose_layout(w, h, s, C, None)
    }

    /// The stage fits the space it was given, border included.
    fn assert_fits(l: &Layout, s: Space) {
        let outer_w = f64::from(l.stage_w) + 2.0 * C.border;
        let outer_h = f64::from(l.stage_h) + 2.0 * C.border;
        let avail_w = match l.arrangement {
            Arrangement::Side => s.width - C.gap - C.panel_side_w,
            Arrangement::Below => s.width,
        };
        assert!(outer_w <= avail_w, "{l:?} too wide for {s:?}");
        let side_offered = s.width - C.gap - C.panel_side_w >= C.min_side_main;
        let peek = match l.arrangement {
            Arrangement::Below if side_offered => C.panel_peek,
            _ => 0.0,
        };
        let avail_h = (s.height - s.under_stage - peek).max(C.min_stage_h + 2.0 * C.border);
        assert!(outer_h <= avail_h, "{l:?} too tall for {s:?}");
    }

    /// The stage hugs the image: it is the image at `scale` plus the margins,
    /// to the pixel, on both axes.
    fn assert_hugs(l: &Layout, w: u32, h: u32) {
        for (len, stage) in [(w, l.stage_w), (h, l.stage_h)] {
            let want = f64::from(len) * l.scale + 2.0 * C.margin;
            assert!(
                (want - f64::from(stage)).abs() < 1.0 + 1e-9,
                "{l:?} doesn't hug a {w}x{h} image"
            );
        }
    }

    #[test]
    fn portrait_screenshot_goes_side_on_a_laptop() {
        let s = space(1440.0, 900.0);
        let l = pick(1170, 2532, s);
        assert_eq!(l.arrangement, Arrangement::Side);
        assert_fits(&l, s);
        assert_hugs(&l, 1170, 2532);
        // Height-limited: the stage is tall and narrow, not a full-width
        // box with gutters.
        assert!(l.stage_h - 48 > (l.stage_w - 48) * 2);
        assert!(f64::from(l.stage_h) + 2.0 * C.border >= s.height - s.under_stage - 1.0);
    }

    #[test]
    fn landscape_keeps_the_panel_beside_on_a_laptop() {
        let s = space(1440.0, 900.0);
        let l = pick(1920, 1080, s);
        assert_eq!(l.arrangement, Arrangement::Side);
        assert_fits(&l, s);
        assert_hugs(&l, 1920, 1080);
    }

    #[test]
    fn banner_puts_the_panel_below() {
        let s = space(1440.0, 900.0);
        let l = pick(3000, 400, s);
        assert_eq!(l.arrangement, Arrangement::Below);
        assert_fits(&l, s);
        assert_hugs(&l, 3000, 400);
        // Width-limited, using the full workspace width.
        assert_eq!(f64::from(l.stage_w) + 2.0 * C.border, s.width);
        // And it beats the side arrangement.
        let side_w = s.width - C.gap - C.panel_side_w - 2.0 * C.border - 2.0 * C.margin;
        assert!(l.scale > side_w / 3000.0);
    }

    #[test]
    fn very_tall_image_goes_side() {
        let s = space(1440.0, 900.0);
        let l = pick(400, 6000, s);
        assert_eq!(l.arrangement, Arrangement::Side);
        assert_fits(&l, s);
        assert_hugs(&l, 400, 6000);
    }

    #[test]
    fn square_image() {
        let s = space(1440.0, 900.0);
        let l = pick(2000, 2000, s);
        assert_eq!(l.arrangement, Arrangement::Side);
        assert_fits(&l, s);
        assert_hugs(&l, 2000, 2000);
        assert_eq!(l.stage_w, l.stage_h);
    }

    #[test]
    fn phone_always_puts_the_panel_below() {
        let s = space(390.0, 844.0);
        for (w, h) in [
            (1170, 2532),
            (1920, 1080),
            (3000, 400),
            (500, 500),
            (64, 64),
        ] {
            let l = pick(w, h, s);
            assert_eq!(l.arrangement, Arrangement::Below, "{w}x{h}");
            assert_fits(&l, s);
            assert_hugs(&l, w, h);
            // Even from Side, a phone switches to Below at once.
            let from_side = choose_layout(w, h, s, C, Some(Arrangement::Side));
            assert_eq!(from_side.arrangement, Arrangement::Below);
        }
        // A wide image takes the full width.
        let l = pick(1920, 1080, s);
        assert_eq!(f64::from(l.stage_w) + 2.0 * C.border, s.width);
        // A tall one takes the full column height: no peek kept for the
        // panel, which is a scroll away regardless.
        let l = pick(1170, 2532, s);
        assert!(f64::from(l.stage_h) + 2.0 * C.border >= s.height - s.under_stage - 1.0);
    }

    #[test]
    fn narrow_window_puts_the_panel_below() {
        // Just too narrow for a 360 px canvas column beside a 300 px panel.
        let s = Space {
            width: 360.0 + C.gap + C.panel_side_w - 1.0,
            height: 2000.0,
            under_stage: 90.0,
        };
        assert_eq!(pick(400, 6000, s).arrangement, Arrangement::Below);
        let wider = Space {
            width: s.width + 1.0,
            ..s
        };
        assert_eq!(pick(400, 6000, wider).arrangement, Arrangement::Side);
    }

    #[test]
    fn small_images_are_not_upscaled() {
        let s = space(1440.0, 900.0);
        let l = pick(800, 500, s);
        assert_eq!(l.scale, 1.0);
        assert_eq!((l.stage_w, l.stage_h), (800 + 48, 500 + 48));
        // Ties (both arrangements at the cap) keep the panel beside.
        assert_eq!(l.arrangement, Arrangement::Side);
    }

    #[test]
    fn tiny_images_go_to_2x_at_most() {
        assert_eq!(max_scale(256, 100), 2.0);
        assert_eq!(max_scale(257, 100), 1.0);
        let s = space(1440.0, 900.0);
        let l = pick(120, 80, s);
        assert_eq!(l.scale, 2.0);
        assert_eq!((l.stage_w, l.stage_h), (240 + 48, 160 + 48));
        // In a box too small for 2x, it still fits.
        let small = Space {
            width: 200.0,
            height: 2000.0,
            under_stage: 0.0,
        };
        let l = pick(256, 256, small);
        assert!(l.scale < 2.0);
        assert_fits(&l, small);
    }

    #[test]
    fn short_window_keeps_a_usable_stage() {
        let s = space(1440.0, 300.0);
        let l = pick(1920, 1080, s);
        assert!(f64::from(l.stage_h) >= C.min_stage_h);
        assert_hugs(&l, 1920, 1080);
    }

    #[test]
    fn hysteresis_holds_near_the_crossover() {
        // A wide image: Below wins in narrower windows, Side (height-limited
        // either way, and Below gives up the peek) in wider ones. Find the
        // first width where Side wins, then check each arrangement holds
        // against a marginal win on either side of it.
        let (w, h) = (2400, 700);
        let at = |vw: f64| space(vw, 900.0);
        let side_from = (800..4000)
            .map(f64::from)
            .find(|&vw| pick(w, h, at(vw)).arrangement == Arrangement::Side)
            .expect("Side wins somewhere");
        let just_side = at(side_from);
        let just_below = at(side_from - 1.0);
        assert_eq!(pick(w, h, just_below).arrangement, Arrangement::Below);
        assert_eq!(
            choose_layout(w, h, just_side, C, Some(Arrangement::Below)).arrangement,
            Arrangement::Below,
            "a marginal win doesn't flip Below"
        );
        assert_eq!(
            choose_layout(w, h, just_below, C, Some(Arrangement::Side)).arrangement,
            Arrangement::Side,
            "a marginal win doesn't flip Side"
        );
        // Well past the crossover, either way, it does switch.
        assert_eq!(
            choose_layout(w, h, at(side_from + 300.0), C, Some(Arrangement::Below)).arrangement,
            Arrangement::Side
        );
        assert_eq!(
            choose_layout(w, h, at(side_from - 300.0), C, Some(Arrangement::Side)).arrangement,
            Arrangement::Below
        );
        // A banner is a clear win for Below whatever is current.
        assert_eq!(
            choose_layout(3000, 400, at(1440.0), C, Some(Arrangement::Side)).arrangement,
            Arrangement::Below
        );
        // And a portrait image a clear win for Side.
        assert_eq!(
            choose_layout(1170, 2532, at(1440.0), C, Some(Arrangement::Below)).arrangement,
            Arrangement::Side
        );
    }

    #[test]
    fn degenerate_inputs_do_not_panic() {
        let zero = Space {
            width: 0.0,
            height: 0.0,
            under_stage: 500.0,
        };
        let l = pick(0, 0, zero);
        assert!(l.scale > 0.0 && l.scale.is_finite());
        assert!(l.stage_w >= 1 && l.stage_h >= 1);
    }
}
