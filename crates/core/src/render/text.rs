//! Text shaping and rasterization from the bundled Inter subset only.
//!
//! The font database is built from the two embedded files and nothing else:
//! no system font is ever loaded, so the export looks the same on every
//! machine.
//!
//! The subset has no `.notdef` outline, so a character it can't map would
//! otherwise shape to nothing and silently vanish from the export. Before
//! layout, every character the regular face's cmap doesn't map (CJK, emoji,
//! Arabic, Hebrew, ...) is replaced with U+FFFD `�`, a glyph added to the
//! subset for this, and a run of such characters collapses into one `�`. A model
//! reading the export then sees that text was there but couldn't be drawn.
//! Whitespace is kept as is. Only the image is affected: the caption text
//! (and so "Copy caption as text") keeps the original characters.

use std::borrow::Cow;
use std::sync::Arc;

use cosmic_text::{
    Attrs, Buffer, Color, Family, FontSystem, Metrics, Shaping, SwashCache, Weight, Wrap, fontdb,
};
use tiny_skia::{Mask, Pixmap};

/// Inter Regular (OFL-1.1, see `fonts/OFL.txt`), subset to Basic Latin,
/// Latin-1, Latin Extended-A, Greek, Cyrillic, general punctuation and
/// arrows. Used for all caption text. Reproduce with:
///
/// ```text
/// pyftsubset Inter-Regular.ttf --no-hinting --desubroutinize \
///   --layout-features='kern,liga,calt,ccmp,locl,mark,mkmk' \
///   --unicodes='U+0020-007E,U+00A0-017F,U+0370-03FF,U+0400-045F,U+2010-2027,U+2030-203A,U+2044,U+20AC,U+2122,U+2190-2195,U+21D2,U+2212,U+2022,U+FFFD'
/// ```
///
/// Inter has no U+FFFD glyph, so `pyftsubset` keeps none, and the subset's
/// `.notdef` has no outline. Then add one, a diamond with Inter's `?` cut out:
///
/// ```text
/// python3 fonts/add_replacement_glyph.py fonts/Inter-Regular-subset.ttf
/// ```
const INTER_REGULAR: &[u8] = include_bytes!("../../fonts/Inter-Regular-subset.ttf");

/// Inter Bold, subset to Basic Latin only (`--unicodes='U+0020-007E'
/// --layout-features='kern'`). Used for badge numbers and the caption
/// header; non-ASCII text in bold falls back to the regular face.
const INTER_BOLD: &[u8] = include_bytes!("../../fonts/Inter-Bold-subset.ttf");

const FAMILY: &str = "Inter";

/// Line height as a multiple of the font size.
pub(super) const LINE_HEIGHT: f32 = 1.35;

/// Drawn in place of each run of characters the font can't map.
pub(super) const REPLACEMENT: char = '\u{FFFD}';

/// Font size and weight of a run of text.
#[derive(Debug, Clone, Copy)]
pub(super) struct Style {
    pub size: f32,
    pub bold: bool,
}

/// A shaped, possibly wrapped block of text.
pub(super) struct Block {
    buffer: Buffer,
    /// Widest line, in pixels.
    pub width: f32,
    /// Total height of all lines, in pixels.
    pub height: f32,
    /// Distance from the block's top to the first line's vertical middle.
    pub first_line_mid: f32,
}

/// Owns the font system and glyph cache for one render.
pub(super) struct Text {
    fonts: FontSystem,
    cache: SwashCache,
    /// The regular face, whose cmap decides which characters can be drawn.
    /// (Bold covers only ASCII and falls back to it for everything else.)
    regular: Arc<cosmic_text::Font>,
}

impl Text {
    pub fn new() -> Self {
        let mut db = fontdb::Database::new();
        let regular_ids = db.load_font_source(fontdb::Source::Binary(Arc::new(INTER_REGULAR)));
        db.load_font_data(INTER_BOLD.to_vec());
        db.set_sans_serif_family(FAMILY);
        // A fixed locale, so nothing about the host machine reaches shaping.
        let mut fonts = FontSystem::new_with_locale_and_db("en-US".to_owned(), db);
        // The embedded face is a compile-time constant, so failing to load it
        // is a build defect, caught by every render test.
        let regular = regular_ids
            .first()
            .and_then(|&id| fonts.get_font(id, Weight::NORMAL))
            .expect("the embedded Inter Regular subset loads");
        Self {
            fonts,
            cache: SwashCache::new(),
            regular,
        }
    }

    /// Whether the regular face has a glyph for `c`.
    fn can_draw(&self, c: char) -> bool {
        self.regular.as_swash().charmap().map(c) != 0
    }

    /// `text` with each run of characters the font can't draw replaced by
    /// one [`REPLACEMENT`]. Whitespace is always kept.
    pub fn drawable<'t>(&self, text: &'t str) -> Cow<'t, str> {
        let ok = |c: char| c.is_whitespace() || self.can_draw(c);
        if text.chars().all(ok) {
            return Cow::Borrowed(text);
        }
        let mut out = String::with_capacity(text.len());
        let mut in_run = false;
        for c in text.chars() {
            if ok(c) {
                out.push(c);
                in_run = false;
            } else if !in_run {
                out.push(REPLACEMENT);
                in_run = true;
            }
        }
        Cow::Owned(out)
    }

    /// Shapes `text`, wrapping it to `width` if given (words first, then
    /// glyphs for a word longer than the line). Characters the font can't
    /// draw are replaced first (see [`Text::drawable`]).
    pub fn layout(&mut self, text: &str, style: Style, width: Option<f32>) -> Block {
        let line_height = (style.size * LINE_HEIGHT).ceil();
        let mut buffer = Buffer::new(&mut self.fonts, Metrics::new(style.size, line_height));
        buffer.set_wrap(Wrap::WordOrGlyph);
        buffer.set_size(width.map(|w| w.max(1.0)), None);
        let weight = if style.bold {
            Weight::BOLD
        } else {
            Weight::NORMAL
        };
        let attrs = Attrs::new().family(Family::Name(FAMILY)).weight(weight);
        let text = self.drawable(text);
        buffer.set_text(&text, &attrs, Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut self.fonts, false);

        let (mut w, mut lines) = (0.0f32, 0usize);
        for run in buffer.layout_runs() {
            w = w.max(run.line_w);
            lines += 1;
        }
        Block {
            buffer,
            width: w.ceil(),
            height: lines.max(1) as f32 * line_height,
            first_line_mid: line_height / 2.0,
        }
    }

    /// The text actually laid out in `block`, line by line (after
    /// replacement), and whether every shaped glyph exists in the font (none
    /// is glyph 0, `.notdef`).
    #[cfg(test)]
    pub fn laid_out(block: &Block) -> (String, bool) {
        let lines: Vec<&str> = block.buffer.lines.iter().map(|l| l.text()).collect();
        let all_found = block
            .buffer
            .layout_runs()
            .all(|run| run.glyphs.iter().all(|g| g.glyph_id != 0));
        (lines.join("\n"), all_found)
    }

    /// Single-line width of `text`.
    pub fn measure(&mut self, text: &str, style: Style) -> f32 {
        self.layout(text, style, None).width
    }

    /// Draws `block` with its top-left at (`x`, `y`) in `rgb`, limited to
    /// `clip` if given (a mask the size of `pixmap`).
    pub fn draw(
        &mut self,
        block: &mut Block,
        pixmap: &mut Pixmap,
        clip: Option<&Mask>,
        (x, y): (f32, f32),
        rgb: [u8; 3],
    ) {
        let (ox, oy) = (x.round() as i32, y.round() as i32);
        let color = Color::rgb(rgb[0], rgb[1], rgb[2]);
        block.buffer.draw(
            &mut self.fonts,
            &mut self.cache,
            color,
            |gx, gy, w, h, c| {
                blend_rect(pixmap, clip, ox + gx, oy + gy, w, h, c);
            },
        );
    }
}

/// Source-over blends a solid rectangle (usually one glyph pixel) onto an
/// opaque pixmap, clipped to its bounds and weighted by `clip` if given.
fn blend_rect(pixmap: &mut Pixmap, clip: Option<&Mask>, x: i32, y: i32, w: u32, h: u32, c: Color) {
    let a = u32::from(c.a());
    if a == 0 {
        return;
    }
    let (pw, ph) = (pixmap.width() as i32, pixmap.height() as i32);
    let x0 = x.max(0);
    let y0 = y.max(0);
    let x1 = x.saturating_add(w as i32).min(pw);
    let y1 = y.saturating_add(h as i32).min(ph);
    let src = [u32::from(c.r()), u32::from(c.g()), u32::from(c.b())];
    let mask = clip.filter(|m| m.width() == pixmap.width() && m.height() == pixmap.height());
    let data = pixmap.data_mut();
    for py in y0..y1 {
        for px in x0..x1 {
            let p = py as usize * pw as usize + px as usize;
            let a = match mask {
                Some(m) => (a * u32::from(m.data()[p]) + 127) / 255,
                None => a,
            };
            if a == 0 {
                continue;
            }
            let i = p * 4;
            for k in 0..3 {
                let d = u32::from(data[i + k]);
                data[i + k] = ((src[k] * a + d * (255 - a) + 127) / 255) as u8;
            }
            data[i + 3] = 255;
        }
    }
}
