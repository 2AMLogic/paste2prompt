//! The image the canvas draws, behind a small adapter over
//! [`crate::image::LoadedImage`] (#9).
//!
//! The canvas draws the shared, full-resolution `ImageBitmap` with
//! `drawImage`, scaled to the view. It never calls `close()` on it (other
//! readers hold it) and never calls `LoadedImage::to_rgba`, which is the
//! export's job.
//!
//! [`SourceImage::test_pattern`] is a generated test card for development
//! and smoke tests, shown only when the URL has `?test-pattern` and no image
//! is loaded (see [`test_pattern_requested`]). Nothing is fetched.

use paste2prompt_core::ImageSize;

use crate::image::LoadedImage;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement, ImageBitmap};

#[derive(Clone, Debug)]
enum Drawable {
    Bitmap(ImageBitmap),
    Canvas(HtmlCanvasElement),
}

/// A full-resolution image the canvas view can draw.
#[derive(Clone, Debug)]
pub struct SourceImage {
    width: u32,
    height: u32,
    drawable: Drawable,
}

impl SourceImage {
    /// Wraps the session's decoded image. Cloning the bitmap clones the JS
    /// reference, not the pixels.
    pub fn from_loaded(image: &LoadedImage) -> Self {
        Self {
            width: image.width(),
            height: image.height(),
            drawable: Drawable::Bitmap(image.bitmap().clone()),
        }
    }

    /// The image's size, or `None` for a zero-sized image.
    pub fn size(&self) -> Option<ImageSize> {
        ImageSize::new(self.width, self.height).ok()
    }

    /// Draws the whole image into the destination rectangle.
    pub fn draw(
        &self,
        ctx: &CanvasRenderingContext2d,
        x: f64,
        y: f64,
        w: f64,
        h: f64,
    ) -> Result<(), JsValue> {
        match &self.drawable {
            Drawable::Bitmap(b) => ctx.draw_image_with_image_bitmap_and_dw_and_dh(b, x, y, w, h),
            Drawable::Canvas(c) => {
                ctx.draw_image_with_html_canvas_element_and_dw_and_dh(c, x, y, w, h)
            }
        }
    }

    /// A generated test card (checkerboard, color blocks, a grid and a
    /// label), for development before image input exists. Nothing is
    /// fetched: it is drawn into an offscreen canvas.
    pub fn test_pattern(width: u32, height: u32) -> Result<Self, JsValue> {
        let doc = web_sys::window()
            .and_then(|w| w.document())
            .ok_or_else(|| JsValue::from_str("no document"))?;
        let canvas: HtmlCanvasElement = doc.create_element("canvas")?.dyn_into()?;
        canvas.set_width(width);
        canvas.set_height(height);
        let ctx: CanvasRenderingContext2d = canvas
            .get_context("2d")?
            .ok_or_else(|| JsValue::from_str("no 2d context"))?
            .dyn_into()?;
        let (w, h) = (f64::from(width), f64::from(height));
        let cell = 40.0;
        for row in 0..height.div_ceil(40) {
            for col in 0..width.div_ceil(40) {
                ctx.set_fill_style_str(if (row + col) % 2 == 0 {
                    "#e8e8ee"
                } else {
                    "#d0d0da"
                });
                ctx.fill_rect(f64::from(col) * cell, f64::from(row) * cell, cell, cell);
            }
        }
        let blocks = [
            ("#e4572e", 0.10, 0.15, 0.22, 0.25),
            ("#29335c", 0.45, 0.20, 0.18, 0.40),
            ("#f3a712", 0.70, 0.55, 0.20, 0.30),
            ("#669bbc", 0.15, 0.60, 0.30, 0.22),
        ];
        for (color, x, y, bw, bh) in blocks {
            ctx.set_fill_style_str(color);
            ctx.fill_rect(x * w, y * h, bw * w, bh * h);
        }
        ctx.set_fill_style_str("#111");
        ctx.set_font("bold 28px system-ui, sans-serif");
        ctx.fill_text(
            &format!("test pattern {width}\u{d7}{height}"),
            16.0,
            h - 20.0,
        )?;
        Ok(Self {
            width,
            height,
            drawable: Drawable::Canvas(canvas),
        })
    }
}

/// Whether to show the test pattern while no image is loaded: only when the
/// URL has a `test-pattern` query parameter (`?test-pattern`).
pub fn test_pattern_requested() -> bool {
    web_sys::window()
        .and_then(|w| w.location().search().ok())
        .is_some_and(|q| {
            q.trim_start_matches('?')
                .split('&')
                .any(|kv| kv.split('=').next() == Some("test-pattern"))
        })
}
