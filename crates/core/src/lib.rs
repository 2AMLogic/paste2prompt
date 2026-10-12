//! Pure-Rust core of paste2prompt.
//!
//! This crate holds the edit model, history, coordinate normalization,
//! caption generation and PNG export (see `docs/SPEC.md` §Architecture). It
//! must not depend on `web-sys` or anything else browser-specific, so it can
//! be tested natively and compiled for `wasm32-unknown-unknown` alike.
//!
//! # Edit model
//!
//! - [`Region`]: a [`Rect`] or lasso [`Polygon`] on the **original** image, in
//!   normalized coordinates (`0.0..=1.0` of width and height). [`ImageSize`]
//!   converts to and from pixels.
//! - [`Op`]: translate, scale, mirror, rotate or hint.
//! - [`Edit`]: a region, an op and an optional hint, with a stable [`EditId`].
//! - [`Document`]: image size, ordered edits and image-level instructions,
//!   with a stable JSON format ([`Document::to_json`]).
//! - [`Editor`]: the only way to change a document; owns undo/redo history
//!   with drag coalescing.
//!
//! Edits refer to the original image and don't compose. An edit's
//! transformed result is only ever a display-only [`Ghost`] (see
//! [`Edit::ghost`]), never a region, so no edit can be stacked on another's
//! result.
//!
//! # Caption
//!
//! [`caption_lines`] / [`Caption`] turn a [`Document`] into the caption block
//! printed under the exported image: a fixed header, one plain-English
//! [`CaptionLine`] per edit, then `Overall: …`. [`Caption::to_plain_text`] is
//! the same text for "Copy caption as text".
//!
//! # Export
//!
//! [`render_export`] draws the export PNG from an [`RgbaImage`] and a
//! [`Document`]: the original image with outlines, numbered badges and
//! ghosts, and the caption block in a band below it, laid out with the
//! bundled Inter font. [`mark_color`] gives each edit's color.

#![warn(missing_docs)]

mod caption;
mod document;
mod geometry;
mod history;
mod op;
mod render;

pub use caption::{CAPTION_HEADER, Caption, CaptionLine, EditLine, caption_lines};
pub use document::{Document, DocumentError, Edit, EditId, FORMAT_VERSION};
pub use geometry::{GeometryError, ImageSize, Point, Polygon, Rect, Region};
pub use history::{EditError, Editor};
pub use op::{Anchor, Ghost, MirrorAxis, Op, OpError};
pub use render::{
    MAX_EXPORT_SIDE, MAX_IMAGE_PIXELS, MIN_EXPORT_WIDTH, RenderError, RgbaImage, mark_color,
    render_export,
};

/// The product name, as shown in the UI and in the export caption header.
pub const APP_NAME: &str = "paste2prompt";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_name_is_lowercase_product_name() {
        assert_eq!(APP_NAME, "paste2prompt");
    }
}
