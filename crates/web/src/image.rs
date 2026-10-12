//! The session's decoded image: what every input path produces and what the
//! canvas view and export read (spec §Input).
//!
//! # For consumers (canvas view, export, history)
//!
//! ```ignore
//! let store = ImageStore::expect();            // provided once by `App`
//! Effect::new(move || {
//!     if let Some(img) = store.current().get() {
//!         ctx.draw_image_with_image_bitmap(img.bitmap(), 0.0, 0.0)?; // fast, GPU-backed
//!         let (w, h) = (img.width(), img.height());                  // full resolution
//!     }
//! });
//! let pixels = img.to_rgba()?; // full-resolution RGBA8, for the core's export
//! ```
//!
//! - The image is decoded **once**, off the main thread where the browser
//!   allows it, by `createImageBitmap`. It is kept at full resolution; scale
//!   when drawing, never when storing.
//! - The stored [`ImageBitmap`] is shared by every clone of [`LoadedImage`].
//!   Don't call `ImageBitmap::close()` on it: other readers still hold it.
//! - [`LoadedImage::to_rgba`] copies the pixels out through a scratch canvas.
//!   That is a synchronous, size-proportional copy (about 59 MB for a 5K
//!   screenshot), so call it when a pixel buffer is needed (export), not on
//!   every frame.
//! - A new value in [`ImageStore::current`] means a new session: the image
//!   was replaced (or cleared, `None`, by **Start over**), and any
//!   confirmation about discarding edits has already been given (see
//!   `input`).

use leptos::prelude::*;
use paste2prompt_core::RgbaImage;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{Blob, CanvasRenderingContext2d, HtmlCanvasElement, ImageBitmap};

/// A decoded image at full resolution.
#[derive(Clone, Debug)]
pub struct LoadedImage {
    bitmap: ImageBitmap,
    width: u32,
    height: u32,
    name: Option<String>,
}

impl LoadedImage {
    /// The decoded image, ready for `drawImage`.
    pub fn bitmap(&self) -> &ImageBitmap {
        &self.bitmap
    }

    /// Width in image pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Height in image pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// The file name, when the source had one (a pasted screenshot usually
    /// reports a generic name such as `image.png`).
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// Copy the pixels out as straight (non-premultiplied) RGBA8, row-major,
    /// top-left origin: the core's [`RgbaImage`], ready for `render_export`.
    pub fn to_rgba(&self) -> Result<RgbaImage, JsValue> {
        let document = window().document().ok_or("no document")?;
        let canvas: HtmlCanvasElement = document.create_element("canvas")?.unchecked_into();
        canvas.set_width(self.width);
        canvas.set_height(self.height);
        let ctx: CanvasRenderingContext2d = canvas
            .get_context("2d")?
            .ok_or("2d context unavailable")?
            .unchecked_into();
        ctx.draw_image_with_image_bitmap(&self.bitmap, 0.0, 0.0)?;
        let data = ctx.get_image_data(0.0, 0.0, f64::from(self.width), f64::from(self.height))?;
        let pixels = data.data().0;
        // Free the scratch canvas's backing store now rather than at GC.
        canvas.set_width(0);
        canvas.set_height(0);
        Ok(RgbaImage {
            width: self.width,
            height: self.height,
            pixels,
        })
    }
}

/// Why an image could not be loaded. `Display` is a user-facing message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadError {
    /// The browser could not decode the data (unsupported or corrupt format).
    Decode,
    /// The image decoded to zero width or height.
    Empty,
}

impl LoadError {
    /// A short message for the status line.
    pub fn message(&self) -> &'static str {
        match self {
            LoadError::Decode => "This browser can't decode that image.",
            LoadError::Empty => "That image is empty.",
        }
    }
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}

/// Decode a blob (a pasted, picked or dropped file) with `createImageBitmap`.
/// Reads only the local blob; no network access.
pub async fn decode(blob: &Blob, name: Option<String>) -> Result<LoadedImage, LoadError> {
    let promise = window()
        .create_image_bitmap_with_blob(blob)
        .map_err(|_| LoadError::Decode)?;
    let bitmap: ImageBitmap = JsFuture::from(promise)
        .await
        .map_err(|_| LoadError::Decode)?
        .unchecked_into();
    let (width, height) = (bitmap.width(), bitmap.height());
    if width == 0 || height == 0 {
        bitmap.close();
        return Err(LoadError::Empty);
    }
    Ok(LoadedImage {
        bitmap,
        width,
        height,
        name,
    })
}

/// The one image of the session, shared through Leptos context.
#[derive(Clone, Copy)]
pub struct ImageStore {
    current: RwSignal<Option<LoadedImage>, LocalStorage>,
}

impl ImageStore {
    /// Create the store and provide it to descendants. Call once, in `App`.
    pub fn provide() -> Self {
        let store = Self {
            current: RwSignal::new_local(None),
        };
        provide_context(store);
        store
    }

    /// The store provided by `App`. Panics outside the app tree.
    pub fn expect() -> Self {
        expect_context()
    }

    /// The current image, or `None` before the first load. Reactive.
    pub fn current(&self) -> ReadSignal<Option<LoadedImage>, LocalStorage> {
        self.current.read_only()
    }

    /// Replace the session's image. Only the input paths call this.
    pub(crate) fn set(&self, image: LoadedImage) {
        self.current.set(Some(image));
    }

    /// Drop the session's image (**Start over**), back to the empty state.
    /// Only the input paths call this, after any confirmation. The bitmap
    /// is not closed: other readers (an export in flight, a clone) may still
    /// hold it, and it is freed when the last one is dropped.
    pub(crate) fn clear(&self) {
        self.current.set(None);
    }
}
