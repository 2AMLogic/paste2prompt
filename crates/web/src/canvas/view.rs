//! Mapping between image pixels and canvas (CSS) pixels.

use super::geom::Px;

/// Fits the image into the canvas: a uniform scale plus an offset that
/// centers it. Screen coordinates are CSS pixels relative to the canvas's
/// top-left corner.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewTransform {
    /// Screen pixels per image pixel.
    pub scale: f64,
    /// Screen position of the image's left edge.
    pub offset_x: f64,
    /// Screen position of the image's top edge.
    pub offset_y: f64,
}

impl ViewTransform {
    /// Fits an `image_w` x `image_h` image inside a `view_w` x `view_h`
    /// viewport with `margin` pixels on every side, centered, at no more
    /// than `max_scale`. The stage is sized to hug the image
    /// (`arrange::choose_layout`), so this normally fills it exactly; the
    /// cap only shows when the stage is larger than the image needs.
    pub fn fit(
        image_w: f64,
        image_h: f64,
        view_w: f64,
        view_h: f64,
        margin: f64,
        max_scale: f64,
    ) -> Self {
        let avail_w = (view_w - 2.0 * margin).max(1.0);
        let avail_h = (view_h - 2.0 * margin).max(1.0);
        let scale = (avail_w / image_w.max(1.0))
            .min(avail_h / image_h.max(1.0))
            .min(max_scale);
        Self {
            scale,
            offset_x: (view_w - image_w * scale) / 2.0,
            offset_y: (view_h - image_h * scale) / 2.0,
        }
    }

    /// Image pixels to screen pixels.
    pub fn to_screen(self, p: Px) -> Px {
        (
            self.offset_x + p.0 * self.scale,
            self.offset_y + p.1 * self.scale,
        )
    }

    /// Screen pixels to image pixels (not clamped to the image).
    pub fn to_image(self, s: Px) -> Px {
        (
            (s.0 - self.offset_x) / self.scale,
            (s.1 - self.offset_y) / self.scale,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_letterboxes_and_centers() {
        let v = ViewTransform::fit(200.0, 100.0, 400.0, 400.0, 0.0, 4.0);
        assert_eq!(v.scale, 2.0);
        assert_eq!((v.offset_x, v.offset_y), (0.0, 100.0));
        let v = ViewTransform::fit(100.0, 200.0, 400.0, 400.0, 0.0, 4.0);
        assert_eq!(v.scale, 2.0);
        assert_eq!((v.offset_x, v.offset_y), (100.0, 0.0));
    }

    #[test]
    fn fit_respects_margin_and_downscales() {
        let v = ViewTransform::fit(4000.0, 1000.0, 820.0, 600.0, 10.0, 1.0);
        assert_eq!(v.scale, 0.2);
        assert_eq!(v.offset_x, 10.0);
    }

    #[test]
    fn fit_caps_the_scale_and_still_centers() {
        let v = ViewTransform::fit(100.0, 50.0, 400.0, 400.0, 0.0, 1.0);
        assert_eq!(v.scale, 1.0);
        assert_eq!((v.offset_x, v.offset_y), (150.0, 175.0));
    }

    #[test]
    fn screen_image_round_trip() {
        let v = ViewTransform::fit(640.0, 480.0, 1000.0, 700.0, 16.0, 1.0);
        let p = (123.5, 456.25);
        let back = v.to_image(v.to_screen(p));
        assert!((back.0 - p.0).abs() < 1e-9 && (back.1 - p.1).abs() < 1e-9);
    }
}
