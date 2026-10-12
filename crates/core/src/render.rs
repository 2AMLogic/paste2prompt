//! The export PNG (spec §Output → Image layout): the original image at full
//! resolution with numbered marks drawn over it, and the caption band below.
//!
//! ```text
//! +--------------------------------------+
//! |  padding | original image | padding  |   padding only when the image is
//! |          |  + marks       |          |   narrower than MIN_EXPORT_WIDTH
//! +======================================+   dark rule
//! |  caption band (header, one row per   |
//! |  edit, Overall: ...)                 |
//! +--------------------------------------+
//! ```
//!
//! Every mark is encoded redundantly, because models misread marks in
//! different ways (`docs/related-work.md`, "Visually Prompted Benchmarks Are
//! Surprisingly Fragile"): a high-contrast outline with a white halo, a large
//! numbered badge at the region's top-left, a per-edit color, and the same
//! number and color on the edit's caption row, whose text spells out the
//! coordinates and the change.
//!
//! - Region outlines are solid. Ghosts (where a translate, scale, rotate or
//!   mirror would put the region) are dashed in the same color, so they read as
//!   "not here yet". A translate also gets an arrow and a hollow badge at the
//!   destination; a mirror gets its axis and a ◀|▶ glyph.
//! - Edit colors cycle through [`mark_color`], starting with red, which
//!   models already read as "look here".
//! - Marks are clipped to the image, never drawn on the padding or band.
//!
//! Rendering is deterministic: the only fonts are the embedded Inter subsets
//! (no system fonts), and everything is drawn on the CPU with `tiny-skia`.
//! The output is a plain, opaque 8-bit RGB PNG with no text chunks (embedding
//! the edit list in `iTXt` is out of scope for v1).

mod marks;
mod text;

use std::fmt;

use tiny_skia::{FillRule, Mask, PathBuilder, Pixmap, Transform};

use crate::caption::{ARROW, CaptionLine, EditLine, OVERALL_PREFIX, caption_lines};
use crate::document::Document;
use crate::geometry::ImageSize;
use marks::{Badge, MarkScale, Marks, badge_width, draw_badge};
use text::{Block, Style, Text};

/// The narrowest export. A narrower image is centered on padding so the
/// caption stays legible (spec: "a minimum width so it stays legible on
/// narrow images").
pub const MIN_EXPORT_WIDTH: u32 = 720;

/// The largest input image accepted, in pixels (width × height): 40 MP,
/// e.g. 8000 × 5000. The export is drawn on the CPU into a buffer the size of
/// the image plus the caption band, so this bounds memory at a few hundred MB
/// even in a 32-bit wasm heap. (Browser canvases have their own, lower limits,
/// such as iOS Safari's ~16.7 MP, but the export doesn't use a canvas.)
pub const MAX_IMAGE_PIXELS: u64 = 40_000_000;

/// The largest export side, in pixels. Bounds very long, thin images and very
/// long captions.
pub const MAX_EXPORT_SIDE: u32 = 32_768;

/// The edit colors, in order: #1 is red, #2 blue, and so on, cycling after
/// seven. Each is dark enough for a white number on top and saturated enough
/// to stand out against a white halo.
const PALETTE: [[u8; 3]; 7] = [
    [222, 25, 35],  // red
    [0, 90, 225],   // blue
    [196, 0, 158],  // magenta
    [200, 82, 0],   // orange
    [0, 135, 62],   // green
    [112, 48, 200], // purple
    [0, 122, 138],  // teal
];

const BAND_BG: [u8; 3] = [242, 243, 245];
const RULE: [u8; 3] = [40, 44, 52];
const PADDING_BG: [u8; 3] = [200, 204, 210];
const INK: [u8; 3] = [17, 20, 24];
const MUTED_INK: [u8; 3] = [70, 76, 86];

/// The color of edit number `number` (1-based, as on its badge), as RGB. The
/// same color is used for the edit's outline, ghost, badge and caption row, so
/// the canvas view can match the export.
pub fn mark_color(number: usize) -> [u8; 3] {
    PALETTE[number.saturating_sub(1) % PALETTE.len()]
}

/// A decoded image: `width × height` pixels of straight (not premultiplied)
/// RGBA8, row-major from the top-left, as from a canvas `getImageData`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RgbaImage {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// `width * height * 4` bytes.
    pub pixels: Vec<u8>,
}

/// Why an export could not be rendered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenderError {
    /// The image has a zero width or height.
    EmptyImage,
    /// `pixels` is not `width * height * 4` bytes long.
    PixelBufferSize {
        /// The length `width * height * 4`.
        expected: u64,
        /// The actual length.
        actual: usize,
    },
    /// The document was made for an image of a different size.
    SizeMismatch {
        /// The image's `(width, height)`.
        image: (u32, u32),
        /// The document's [`Document::image_size`].
        document: ImageSize,
    },
    /// The image is over [`MAX_IMAGE_PIXELS`], or the export would have a
    /// side over [`MAX_EXPORT_SIDE`].
    TooLarge {
        /// The export's `(width, height)` (or the image's, if the image itself
        /// is too large).
        size: (u64, u64),
    },
    /// PNG encoding failed.
    Encode(String),
}

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyImage => f.write_str("the image is empty"),
            Self::PixelBufferSize { expected, actual } => write!(
                f,
                "image pixel buffer is {actual} bytes, expected {expected}"
            ),
            Self::SizeMismatch { image, document } => write!(
                f,
                "image is {}×{} but the edits were made on a {}×{} image",
                image.0,
                image.1,
                document.width(),
                document.height()
            ),
            Self::TooLarge { size } => write!(
                f,
                "a {}×{} export is too large (limit {} MP image, {} px side)",
                size.0,
                size.1,
                MAX_IMAGE_PIXELS / 1_000_000,
                MAX_EXPORT_SIDE
            ),
            Self::Encode(e) => write!(f, "PNG encoding failed: {e}"),
        }
    }
}

impl std::error::Error for RenderError {}

/// Checks, without any pixels, whether [`render_export`] would accept a
/// `width × height` image for `doc`: the image is non-empty, at most
/// [`MAX_IMAGE_PIXELS`] with sides at most [`MAX_EXPORT_SIDE`], matches the
/// document's size, and the export (the image padded to [`MIN_EXPORT_WIDTH`],
/// plus `doc`'s caption band) has no side over [`MAX_EXPORT_SIDE`].
///
/// Call it before copying an image's pixels out for an export: a rejected
/// image then costs nothing. It lays out the caption (a few milliseconds),
/// but allocates nothing image-sized. `render_export` runs the same checks,
/// so the two can't disagree; the only error it adds is
/// [`RenderError::PixelBufferSize`], which is about the buffer, not the size.
pub fn export_size_ok(width: u32, height: u32, doc: &Document) -> Result<(), RenderError> {
    plan(check_image_size(width, height)?, doc).map(|_| ())
}

/// Renders the export for `doc` over `image` and returns PNG bytes.
///
/// `image` must be the original image the document's edits were made on
/// (same size as [`Document::image_size`]). Transparent pixels are composited
/// over white. Never panics on valid input; see [`RenderError`] for what is
/// rejected, and [`export_size_ok`] to check the size before reading pixels.
pub fn render_export(image: &RgbaImage, doc: &Document) -> Result<Vec<u8>, RenderError> {
    let pixmap = render_pixmap(image, doc)?;
    encode_png(&pixmap)
}

/// Renders the export as an opaque pixmap (premultiplied, alpha 255).
fn render_pixmap(image: &RgbaImage, doc: &Document) -> Result<Pixmap, RenderError> {
    let size = check_image_size(image.width, image.height)?;
    let expected = u64::from(image.width) * u64::from(image.height) * 4;
    if image.pixels.len() as u64 != expected {
        return Err(RenderError::PixelBufferSize {
            expected,
            actual: image.pixels.len(),
        });
    }
    let Plan {
        width,
        height,
        mut text,
        mut band,
    } = plan(size, doc)?;
    let (iw, ih) = (image.width, image.height);
    let band_h = height - ih;
    let too_large = || RenderError::TooLarge {
        size: (u64::from(width), u64::from(height)),
    };
    let mut pixmap = Pixmap::new(width, height).ok_or_else(too_large)?;

    // Image area: padding, then the image centered on it.
    let ox = (width - iw) / 2;
    fill_rect(&mut pixmap, 0, 0, width, ih, PADDING_BG);
    blit_over_white(&mut pixmap, image, ox);

    // Marks, clipped to the image.
    let mut clip = Mask::new(width, height).ok_or_else(too_large)?;
    if let Some(rect) = tiny_skia::Rect::from_xywh(ox as f32, 0.0, iw as f32, ih as f32) {
        clip.fill_path(
            &PathBuilder::from_rect(rect),
            FillRule::Winding,
            false,
            Transform::identity(),
        );
    }
    let mut marks = Marks {
        pixmap: &mut pixmap,
        text: &mut text,
        clip: &clip,
        origin: (ox as f32, 0.0),
        size,
        scale: MarkScale::for_image(size),
    };
    for (i, edit) in doc.edits().iter().enumerate() {
        let number = i + 1;
        marks.draw_edit(number, edit, mark_color(number));
    }

    // Caption band.
    fill_rect(&mut pixmap, 0, ih, width, band_h, BAND_BG);
    fill_rect(&mut pixmap, 0, ih, width, band.rule, RULE);
    band.draw(&mut pixmap, &mut text, ih as f32);

    Ok(pixmap)
}

/// The image-only limits: non-empty, at most [`MAX_IMAGE_PIXELS`], and no
/// side over [`MAX_EXPORT_SIDE`]. Checked first, before any layout.
fn check_image_size(width: u32, height: u32) -> Result<ImageSize, RenderError> {
    let size = ImageSize::new(width, height).map_err(|_| RenderError::EmptyImage)?;
    let pixels = u64::from(width) * u64::from(height);
    if pixels > MAX_IMAGE_PIXELS || width > MAX_EXPORT_SIDE || height > MAX_EXPORT_SIDE {
        return Err(RenderError::TooLarge {
            size: (u64::from(width), u64::from(height)),
        });
    }
    Ok(size)
}

/// The export's size and its laid-out caption band, before any drawing.
struct Plan {
    /// Export width: the image's, or [`MIN_EXPORT_WIDTH`] if that's wider.
    width: u32,
    /// Export height: the image's plus the band's.
    height: u32,
    text: Text,
    band: Band,
}

/// Lays out the export of a `size` image for `doc` and checks the export's
/// size. Shared by [`export_size_ok`] and the renderer, so the prediction
/// and the render can't drift apart.
fn plan(size: ImageSize, doc: &Document) -> Result<Plan, RenderError> {
    if doc.image_size() != size {
        return Err(RenderError::SizeMismatch {
            image: (size.width(), size.height()),
            document: doc.image_size(),
        });
    }
    let width = size.width().max(MIN_EXPORT_WIDTH);
    let mut text = Text::new();
    let band = layout_band(&mut text, &caption_lines(doc), width as f32);
    let height = u64::from(size.height()) + u64::from(band.height.ceil() as u32);
    // The pixmap's byte length must also fit in `usize` (4 GiB on wasm32).
    let bytes = u64::from(width) * height * 4;
    if height > u64::from(MAX_EXPORT_SIDE)
        || width > MAX_EXPORT_SIDE
        || usize::try_from(bytes).is_err()
    {
        return Err(RenderError::TooLarge {
            size: (u64::from(width), height),
        });
    }
    Ok(Plan {
        width,
        height: height as u32,
        text,
        band,
    })
}

/// Fills a rectangle with an opaque color, clipped to the pixmap.
fn fill_rect(pixmap: &mut Pixmap, x: u32, y: u32, w: u32, h: u32, rgb: [u8; 3]) {
    let pw = pixmap.width();
    let ph = pixmap.height();
    let (x1, y1) = (x.saturating_add(w).min(pw), y.saturating_add(h).min(ph));
    let px = [rgb[0], rgb[1], rgb[2], 255];
    let data = pixmap.data_mut();
    for row in y.min(ph)..y1 {
        let start = (row as usize * pw as usize + x.min(pw) as usize) * 4;
        let end = (row as usize * pw as usize + x1 as usize) * 4;
        for chunk in data[start..end].as_chunks_mut::<4>().0.iter_mut() {
            chunk.copy_from_slice(&px);
        }
    }
}

/// Copies the straight-alpha image onto the canvas at (`ox`, 0), composited
/// over white so the export is opaque.
fn blit_over_white(pixmap: &mut Pixmap, image: &RgbaImage, ox: u32) {
    let pw = pixmap.width() as usize;
    let iw = image.width as usize;
    let data = pixmap.data_mut();
    for (row, src) in image.pixels.chunks_exact(iw * 4).enumerate() {
        let start = (row * pw + ox as usize) * 4;
        let dst = &mut data[start..start + iw * 4];
        for (d, s) in dst
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .zip(src.as_chunks::<4>().0.iter())
        {
            let a = u32::from(s[3]);
            for k in 0..3 {
                d[k] = ((u32::from(s[k]) * a + 255 * (255 - a) + 127) / 255) as u8;
            }
            d[3] = 255;
        }
    }
}

/// Encodes an opaque pixmap as an 8-bit RGB PNG with no ancillary chunks.
fn encode_png(pixmap: &Pixmap) -> Result<Vec<u8>, RenderError> {
    // Every pixel is opaque, so premultiplied equals straight RGB.
    let rgb: Vec<u8> = pixmap
        .data()
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|p| [p[0], p[1], p[2]])
        .collect();
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, pixmap.width(), pixmap.height());
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let enc = |e: png::EncodingError| RenderError::Encode(e.to_string());
    let mut writer = encoder.write_header().map_err(enc)?;
    writer.write_image_data(&rgb).map_err(enc)?;
    writer.finish().map_err(enc)?;
    Ok(out)
}

// ---------------------------------------------------------------------------
// Caption band layout

/// One thing to draw in the band, positioned relative to the band's top.
enum Item {
    Text {
        block: Block,
        x: f32,
        y: f32,
        rgb: [u8; 3],
    },
    Badge {
        number: usize,
        center: (f32, f32),
        height: f32,
    },
}

/// The laid-out caption band.
struct Band {
    items: Vec<Item>,
    height: f32,
    /// Height of the dark rule at the band's top.
    rule: u32,
}

impl Band {
    fn draw(&mut self, pixmap: &mut Pixmap, text: &mut Text, top: f32) {
        for item in &mut self.items {
            match item {
                Item::Text { block, x, y, rgb } => {
                    text.draw(block, pixmap, None, (*x, top + *y), *rgb)
                }
                Item::Badge {
                    number,
                    center,
                    height,
                } => draw_badge(
                    pixmap,
                    text,
                    None,
                    Badge {
                        number: *number,
                        center: (center.0, top + center.1),
                        height: *height,
                        ring: 0.0,
                        rgb: mark_color(*number),
                        hollow: false,
                    },
                ),
            }
        }
    }
}

/// Font size of the caption, scaled with the export width so the text stays
/// readable after a chat app downscales the image.
fn caption_font_size(width: f32) -> f32 {
    (width / 60.0).clamp(16.0, 96.0).round()
}

/// Lays out the caption lines for a band `width` pixels wide.
///
/// Edit rows use columns (`badge  op  region  →  description`) when the
/// fixed columns take at most 60% of the width, so the description column
/// gets room to wrap. Otherwise each row stacks: `op  region` on one line and
/// `→ description` under it, both wrapping in the column after the badge.
fn layout_band(text: &mut Text, lines: &[CaptionLine], width: f32) -> Band {
    let f = caption_font_size(width);
    let regular = Style {
        size: f,
        bold: false,
    };
    let bold = Style {
        size: f,
        bold: true,
    };
    let pad = f;
    let gap = (f * 0.8).round();
    let rule = (f * 0.25).round().max(3.0) as u32;
    let inner = width - 2.0 * pad;
    let badge_h = (f * 1.45).round();

    let edits: Vec<&EditLine> = lines
        .iter()
        .filter_map(|l| match l {
            CaptionLine::Edit(e) => Some(e),
            _ => None,
        })
        .collect();
    let max_number = edits.iter().map(|e| e.number).max().unwrap_or(1);
    let badge_col = badge_width(text, max_number, badge_h) + gap;
    let op_w = edits
        .iter()
        .map(|e| text.measure(e.op, regular))
        .fold(0.0, f32::max);
    let region_w = edits
        .iter()
        .map(|e| text.measure(&e.region, regular))
        .fold(0.0, f32::max);
    let arrow_w = text.measure(ARROW, regular);
    let fixed = badge_col + op_w + gap + region_w + gap + arrow_w + gap;
    let columns = fixed <= inner * 0.6;

    let overall_w = text.measure(OVERALL_PREFIX.trim_end(), bold);
    let overall_col = overall_w + text.measure(" ", regular);

    let mut items = Vec::new();
    let mut y = rule as f32 + pad;
    let mut prev_header = false;
    for line in lines {
        match line {
            CaptionLine::Header(h) => {
                // First header line in bold, the second muted.
                let (style, rgb) = if prev_header {
                    (regular, MUTED_INK)
                } else {
                    (bold, INK)
                };
                let block = text.layout(h, style, Some(inner));
                let height = block.height;
                items.push(Item::Text {
                    block,
                    x: pad,
                    y,
                    rgb,
                });
                y += height;
                if prev_header {
                    y += f * 0.5;
                }
                prev_header = true;
            }
            CaptionLine::Edit(e) => {
                prev_header = false;
                let x0 = pad + badge_col;
                let mut row = Vec::new();
                let row_h;
                if columns {
                    let desc_x = x0 + op_w + gap + region_w + gap + arrow_w + gap;
                    let desc = text.layout(&e.description, regular, Some(pad + inner - desc_x));
                    row_h = desc.height;
                    row.push((text.layout(e.op, regular, None), x0, 0.0));
                    row.push((text.layout(&e.region, regular, None), x0 + op_w + gap, 0.0));
                    let ax = x0 + op_w + gap + region_w + gap;
                    row.push((text.layout(ARROW, regular, None), ax, 0.0));
                    row.push((desc, desc_x, 0.0));
                } else {
                    let col = pad + inner - x0;
                    let head = format!("{}  {}", e.op, e.region);
                    let first = text.layout(&head, regular, Some(col));
                    let second =
                        text.layout(&format!("{ARROW} {}", e.description), regular, Some(col));
                    let h1 = first.height;
                    row_h = h1 + second.height;
                    row.push((first, x0, 0.0));
                    row.push((second, x0, h1));
                }
                // The badge sits on the row's first line.
                let mid = (f * text::LINE_HEIGHT).ceil() / 2.0;
                let center = (pad + (badge_col - gap) / 2.0, y + mid.max(badge_h / 2.0));
                let text_top = y + (badge_h / 2.0 - mid).max(0.0);
                items.push(Item::Badge {
                    number: e.number,
                    center,
                    height: badge_h,
                });
                for (block, x, dy) in row {
                    items.push(Item::Text {
                        block,
                        x,
                        y: text_top + dy,
                        rgb: INK,
                    });
                }
                y = (text_top + row_h).max(y + badge_h) + f * 0.4;
            }
            CaptionLine::Overall(t) | CaptionLine::OverallContinued(t) => {
                prev_header = false;
                if matches!(line, CaptionLine::Overall(_)) {
                    y += f * 0.2;
                    items.push(Item::Text {
                        block: text.layout(OVERALL_PREFIX.trim_end(), bold, None),
                        x: pad,
                        y,
                        rgb: INK,
                    });
                }
                let block = text.layout(t, regular, Some(inner - overall_col));
                let height = block.height;
                items.push(Item::Text {
                    block,
                    x: pad + overall_col,
                    y,
                    rgb: INK,
                });
                y += height;
            }
        }
    }
    Band {
        items,
        height: y + pad,
        rule,
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod size_tests;
