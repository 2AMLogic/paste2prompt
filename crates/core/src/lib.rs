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

#![warn(missing_docs)]

mod caption;
mod document;
mod geometry;
mod history;
mod op;

pub use caption::{
    ARROW, CAPTION_HEADER, Caption, CaptionLine, EditLine, OVERALL_PREFIX, caption_lines,
};
pub use document::{Document, DocumentError, Edit, EditId, FORMAT_VERSION};
pub use geometry::{GeometryError, ImageSize, Point, Polygon, Rect, Region};
pub use history::{EditError, Editor};
pub use op::{Anchor, Ghost, MAX_SCALE, MirrorAxis, Op, OpError};

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
