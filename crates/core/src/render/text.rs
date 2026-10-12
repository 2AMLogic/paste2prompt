//! Text shaping, line wrapping and rasterization from the bundled Inter
//! subset only.
//!
//! Both faces are embedded and nothing else is ever loaded, so the export
//! looks the same on every machine. Text is shaped with harfrust (Inter's
//! kerning and its liga/calt/ccmp/mark features), wrapped greedily at
//! Unicode line-break opportunities, and drawn by turning the TrueType
//! outlines into tiny-skia paths, filled by the same rasterizer as the marks.
//!
//! The subset has no `.notdef` outline, so a character it can't map would
//! otherwise shape to nothing and silently vanish from the export. Before
//! layout, every character the regular face's cmap doesn't map (CJK, emoji,
//! Arabic, Hebrew, ...) is replaced with U+FFFD `�`, a glyph added to the
//! subset for this, and a run of such characters collapses into one `�`. A model
//! reading the export then sees that text was there but couldn't be drawn.
//! Whitespace is kept as is. A letter the font can't draw as typed is
//! NFC-normalized first, so decomposed accents draw as the precomposed glyph
//! (see [`Text::drawable`]). Only the
//! image is affected: the caption text (and so "Copy caption as text") keeps
//! the original characters, unnormalized.
//!
//! Because of that replacement, everything that reaches layout is in a
//! left-to-right script the subset covers (Latin, Greek, Cyrillic), so no
//! bidi reordering or script fallback is needed. The layout reproduces what
//! cosmic-text 0.19 (the previous text stack) did for such text, so line
//! breaks and glyph positions didn't change when it was replaced (#43):
//! each word and each whitespace character is shaped on its own, lines break
//! between words (inside a word only if it's wider than the line), and
//! trailing whitespace doesn't count toward a wrapped line's width.

use std::borrow::Cow;
use std::ops::Range;

use harfrust::{Direction, Script, ShapePlan, ShaperData, UnicodeBuffer};
use read_fonts::tables::cmap::CmapSubtable;
use read_fonts::tables::glyf::{Anchor, CompositeGlyphFlags, Glyf, Glyph as GlyfGlyph};
use read_fonts::tables::loca::Loca;
use read_fonts::tables::os2::SelectionFlags;
use read_fonts::types::GlyphId;
use read_fonts::{FontRef, TableProvider};
use tiny_skia::{FillRule, Mask, Paint, PathBuilder, Pixmap, Transform};
use unicode_normalization::char::{canonical_combining_class, compose, decompose_canonical};

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

/// Index of each face in [`Text::faces`].
const REGULAR: usize = 0;
const BOLD: usize = 1;

/// Line height as a multiple of the font size.
pub(super) const LINE_HEIGHT: f32 = 1.35;

/// Drawn in place of each run of characters the font can't map.
pub(super) const REPLACEMENT: char = '\u{FFFD}';

/// A tab advances to the next multiple of this many spaces.
const TAB_WIDTH: f32 = 8.0;

/// Deepest nesting of composite glyphs followed when drawing.
const MAX_COMPONENT_DEPTH: u8 = 8;

/// Font size and weight of a run of text.
#[derive(Debug, Clone, Copy)]
pub(super) struct Style {
    pub size: f32,
    pub bold: bool,
}

/// A shaped, possibly wrapped block of text.
pub(super) struct Block {
    lines: Vec<Line>,
    size: f32,
    line_height: f32,
    /// The text laid out, after replacement, one paragraph per line.
    #[cfg(test)]
    text: String,
    /// Widest line, in pixels.
    pub width: f32,
    /// Total height of all lines, in pixels.
    pub height: f32,
    /// Distance from the block's top to the first line's vertical middle.
    pub first_line_mid: f32,
}

/// One laid-out line of a [`Block`].
struct Line {
    glyphs: Vec<Placed>,
    /// Width, not counting whitespace at a wrap.
    w: f32,
    /// Largest ascent and descent of the line's glyphs, in pixels.
    ascent: f32,
    descent: f32,
}

/// A glyph at its pen position on a line.
struct Placed {
    face: usize,
    id: u16,
    /// Pen position from the line's start, in pixels.
    x: f32,
    /// Shaping offsets, in ems.
    x_offset: f32,
    y_offset: f32,
}

/// A shaped glyph, positioned in ems.
#[derive(Debug, Clone, Copy)]
struct Shaped {
    face: usize,
    id: u16,
    /// Byte range of the glyph's cluster in the paragraph.
    start: usize,
    end: usize,
    x_advance: f32,
    x_offset: f32,
    y_offset: f32,
}

impl Shaped {
    fn width(&self, size: f32) -> f32 {
        size * self.x_advance
    }
}

/// A word, or a single whitespace character (`blank`), shaped on its own.
struct Word {
    blank: bool,
    glyphs: Vec<Shaped>,
}

impl Word {
    fn width(&self, size: f32) -> f32 {
        let mut w = 0.0;
        for g in &self.glyphs {
            w += g.width(size);
        }
        w
    }
}

/// A position inside a paragraph's words: glyph `glyph` of word `word`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Pos {
    word: usize,
    glyph: usize,
}

impl Pos {
    const fn new(word: usize, glyph: usize) -> Self {
        Self { word, glyph }
    }
}

/// The glyphs of one wrapped line, before positioning.
#[derive(Default)]
struct Visual {
    ranges: Vec<(Pos, Pos)>,
    w: f32,
}

impl Visual {
    fn add(&mut self, start: Pos, end: Pos, width: f32) {
        if start != end {
            self.ranges.push((start, end));
            self.w += width;
        }
    }
}

/// One embedded face: its tables, shaping data and line metrics.
struct Face {
    font: FontRef<'static>,
    shaper: ShaperData,
    /// Shape plans by script, built on first use.
    plans: Vec<(Script, ShapePlan)>,
    cmap: Option<CmapSubtable<'static>>,
    loca: Option<Loca<'static>>,
    glyf: Option<Glyf<'static>>,
    units_per_em: f32,
    /// Line ascent and descent (both positive), in ems.
    ascent: f32,
    descent: f32,
}

impl Face {
    fn new(data: &'static [u8]) -> Self {
        // The embedded faces are compile-time constants, so failing to parse
        // one is a build defect, caught by every render test.
        let font = FontRef::new(data).expect("the embedded Inter subset parses");
        let units_per_em = font
            .head()
            .map_or(1000.0, |h| f32::from(h.units_per_em()))
            .max(1.0);
        // OS/2 typographic metrics if the font asks for them, else hhea's
        // (the same choice as skrifa's, which cosmic-text used).
        let (ascent, descent) = match (font.os2(), font.hhea()) {
            (Ok(os2), _)
                if os2
                    .fs_selection()
                    .contains(SelectionFlags::USE_TYPO_METRICS) =>
            {
                (os2.s_typo_ascender(), os2.s_typo_descender())
            }
            (_, Ok(hhea)) => (hhea.ascender().to_i16(), hhea.descender().to_i16()),
            _ => (0, 0),
        };
        Self {
            shaper: ShaperData::new(&font),
            plans: Vec::new(),
            cmap: font
                .cmap()
                .ok()
                .and_then(|c| c.best_subtable())
                .map(|(_, _, s)| s),
            loca: font.loca(None).ok(),
            glyf: font.glyf().ok(),
            units_per_em,
            ascent: f32::from(ascent) / units_per_em,
            descent: -f32::from(descent) / units_per_em,
            font,
        }
    }

    /// The face's glyph for `c`, 0 if it has none.
    fn glyph_for(&self, c: char) -> u32 {
        self.cmap
            .as_ref()
            .and_then(|m| m.map_codepoint(c))
            .map_or(0, GlyphId::to_u32)
    }

    /// Shapes `line[range]` on its own, appending its glyphs to `out`, and
    /// returns the start of each cluster the face has no glyph for.
    fn shape(
        &mut self,
        face: usize,
        line: &str,
        range: Range<usize>,
        out: &mut Vec<Shaped>,
    ) -> Vec<usize> {
        let run = &line[range.clone()];
        let mut buffer = UnicodeBuffer::new();
        buffer.set_direction(Direction::LeftToRight);
        if run.contains('\t') {
            // Tabs are shaped as spaces and widened afterwards.
            buffer.push_str(&run.replace('\t', " "));
        } else {
            buffer.push_str(run);
        }
        buffer.guess_segment_properties();
        let script = buffer.script();
        let shaper = self.shaper.shaper(&self.font).build();
        let plan = match self.plans.iter().position(|(s, _)| *s == script) {
            Some(i) => &self.plans[i].1,
            None => {
                let plan = ShapePlan::new(
                    &shaper,
                    buffer.direction(),
                    Some(script),
                    buffer.language().as_ref(),
                    &[],
                );
                self.plans.push((script, plan));
                &self.plans[self.plans.len() - 1].1
            }
        };
        let shaped = shaper.shape_with_plan(plan, buffer, &[]);
        let upem = self.units_per_em;
        let first = out.len();
        let mut missing = Vec::new();
        for (info, pos) in shaped.glyph_infos().iter().zip(shaped.glyph_positions()) {
            let start = range.start + info.cluster as usize;
            if info.glyph_id == 0 {
                missing.push(start);
            }
            out.push(Shaped {
                face,
                id: u16::try_from(info.glyph_id).unwrap_or(0),
                start,
                end: range.end,
                x_advance: pos.x_advance as f32 / upem,
                x_offset: pos.x_offset as f32 / upem,
                y_offset: pos.y_offset as f32 / upem,
            });
        }
        // Each glyph's cluster ends where the next cluster starts.
        for i in (first + 1..out.len()).rev() {
            let (next_start, next_end) = (out[i].start, out[i].end);
            let prev = &mut out[i - 1];
            prev.end = if prev.start == next_start {
                next_end
            } else {
                next_start
            };
        }
        missing
    }

    /// Appends the outline of glyph `id`, in font units with y up, to
    /// `points`, ending each contour at the index pushed to `ends`.
    fn outline(&self, id: GlyphId, depth: u8, points: &mut Vec<Point>, ends: &mut Vec<usize>) {
        let (Some(loca), Some(glyf)) = (&self.loca, &self.glyf) else {
            return;
        };
        let Ok(Some(glyph)) = loca.get_glyf(id, glyf) else {
            return;
        };
        match glyph {
            GlyfGlyph::Simple(simple) => {
                let base = points.len();
                points.extend(simple.points().map(|p| Point {
                    x: f32::from(p.x),
                    y: f32::from(p.y),
                    on: p.on_curve,
                }));
                let n = points.len();
                ends.extend(
                    simple
                        .end_pts_of_contours()
                        .iter()
                        .map(|e| (base + usize::from(e.get()) + 1).min(n)),
                );
            }
            GlyfGlyph::Composite(composite) if depth < MAX_COMPONENT_DEPTH => {
                let base = points.len();
                for c in composite.components() {
                    let start = points.len();
                    self.outline(c.glyph.into(), depth + 1, points, ends);
                    let t = c.transform;
                    let (xx, yx, xy, yy) =
                        (t.xx.to_f32(), t.yx.to_f32(), t.xy.to_f32(), t.yy.to_f32());
                    let apply = |x: f32, y: f32| (xx * x + xy * y, yx * x + yy * y);
                    for p in &mut points[start..] {
                        (p.x, p.y) = apply(p.x, p.y);
                    }
                    let (dx, dy) = match c.anchor {
                        Anchor::Offset { x, y } => {
                            let (x, y) = (f32::from(x), f32::from(y));
                            if c.flags
                                .contains(CompositeGlyphFlags::SCALED_COMPONENT_OFFSET)
                            {
                                apply(x, y)
                            } else {
                                (x, y)
                            }
                        }
                        // Line up a point of the glyph so far with one of
                        // this component.
                        Anchor::Point { base: b, component } => match (
                            points.get(base + usize::from(b)),
                            points.get(start + usize::from(component)),
                        ) {
                            (Some(b), Some(c)) => (b.x - c.x, b.y - c.y),
                            _ => (0.0, 0.0),
                        },
                    };
                    for p in &mut points[start..] {
                        p.x += dx;
                        p.y += dy;
                    }
                }
            }
            GlyfGlyph::Composite(_) => {}
        }
    }
}

/// An outline point in font units.
#[derive(Debug, Clone, Copy)]
struct Point {
    x: f32,
    y: f32,
    on: bool,
}

/// Owns the two faces for one render.
pub(super) struct Text {
    /// Regular, then bold. The regular face's cmap decides which characters
    /// can be drawn; bold covers only ASCII and falls back to regular.
    faces: [Face; 2],
}

impl Text {
    pub fn new() -> Self {
        Self {
            faces: [Face::new(INTER_REGULAR), Face::new(INTER_BOLD)],
        }
    }

    /// Whether the regular face has a glyph for `c`.
    fn can_draw(&self, c: char) -> bool {
        self.faces[REGULAR].glyph_for(c) != 0
    }

    /// `text` with each run of letters the font can't draw replaced by one
    /// [`REPLACEMENT`]. Whitespace is always kept.
    ///
    /// A letter is a character plus the combining marks after it (canonical
    /// combining class above 0). One the font can't draw as typed is
    /// NFC-normalized first, which composes decomposed accents (`e` +
    /// U+0301) into the precomposed character the subset has (`é`); the
    /// subset has no combining marks. Letters the font can draw are kept as
    /// typed, since NFC would map a few of them (e.g. U+0374) to characters
    /// outside the subset. If the normalized letter still can't be drawn, the
    /// whole letter is replaced: `q` + U+0303, which has no precomposed form,
    /// becomes `�`, not `q�`, the same as a precomposed character outside the
    /// subset. So the export doesn't depend on which form the text was typed
    /// in. A mark with no letter before it (at the start, or after
    /// whitespace) is replaced on its own.
    pub fn drawable<'t>(&self, text: &'t str) -> Cow<'t, str> {
        let ok = |c: char| c.is_whitespace() || self.can_draw(c);
        if text.chars().all(ok) {
            return Cow::Borrowed(text);
        }
        let mut out = String::with_capacity(text.len());
        let mut in_run = false;
        let mut rest = text;
        while let Some(first) = rest.chars().next() {
            let mut len = first.len_utf8();
            if !first.is_whitespace() {
                len += rest[len..]
                    .chars()
                    .take_while(|&c| canonical_combining_class(c) != 0)
                    .map(char::len_utf8)
                    .sum::<usize>();
            }
            let (letter, tail) = rest.split_at(len);
            rest = tail;
            if letter.chars().all(ok) {
                out.push_str(letter);
                in_run = false;
                continue;
            }
            let composed = nfc_letter(letter);
            if composed.iter().all(|&c| ok(c)) {
                out.extend(composed);
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
        let size = style.size;
        let line_height = (size * LINE_HEIGHT).ceil();
        let width = width.map(|w| w.max(1.0));
        let face = if style.bold { BOLD } else { REGULAR };
        let text = self.drawable(text);
        let mut lines = Vec::new();
        for paragraph in paragraphs(&text) {
            let words = self.words(face, paragraph);
            self.place(&words, size, width, &mut lines);
        }
        let w = lines.iter().map(|l| l.w).fold(0.0f32, f32::max);
        Block {
            height: lines.len().max(1) as f32 * line_height,
            lines,
            size,
            line_height,
            #[cfg(test)]
            text: paragraphs(&text).collect::<Vec<_>>().join("\n"),
            width: w.ceil(),
            first_line_mid: line_height / 2.0,
        }
    }

    /// Shapes `line[range]` in `face`, taking each cluster that face has no
    /// glyph for from the other face if it has one.
    fn shape_run(&mut self, face: usize, line: &str, range: Range<usize>) -> Vec<Shaped> {
        let mut glyphs = Vec::new();
        let mut missing = self.faces[face].shape(face, line, range.clone(), &mut glyphs);
        if missing.is_empty() {
            return glyphs;
        }
        let other = 1 - face;
        let mut fallback = Vec::new();
        let fb_missing = self.faces[other].shape(other, line, range, &mut fallback);
        let mut fb_i = 0;
        while fb_i < fallback.len() {
            let (start, end) = (fallback[fb_i].start, fallback[fb_i].end);
            if !missing.contains(&start) || fb_missing.contains(&start) {
                fb_i += 1;
                continue;
            }
            missing.retain(|&m| m < start || m >= end);
            let in_cluster = |g: &Shaped| g.start >= start && g.end <= end;
            let mut i = glyphs.iter().position(in_cluster).unwrap_or(glyphs.len());
            while i < glyphs.len() && in_cluster(&glyphs[i]) {
                glyphs.remove(i);
            }
            while fb_i < fallback.len() && in_cluster(&fallback[fb_i]) {
                glyphs.insert(i, fallback.remove(fb_i));
                i += 1;
            }
        }
        glyphs
    }

    /// Whether `face` shapes `a` and `b` together into something other than
    /// their two plain glyphs (a ligature such as `->` or a contextual
    /// alternate), so the line mustn't break between them.
    fn joins(&mut self, face: usize, a: char, b: char) -> bool {
        let pair: String = [a, b].into_iter().collect();
        let mut glyphs = Vec::new();
        self.faces[face].shape(face, &pair, 0..pair.len(), &mut glyphs);
        let f = &self.faces[face];
        glyphs.len() < 2
            || glyphs.len() == 2
                && (u32::from(glyphs[0].id) != f.glyph_for(a)
                    || u32::from(glyphs[1].id) != f.glyph_for(b))
    }

    /// Splits `line` into words at its line-break opportunities and shapes
    /// each word, and each whitespace character after it, on its own.
    fn words(&mut self, face: usize, line: &str) -> Vec<Word> {
        let mut words = Vec::new();
        let mut start_word = 0;
        for (end_lb, _) in unicode_linebreak::linebreaks(line) {
            if end_lb > 0 && end_lb < line.len() {
                let before = line[..end_lb].chars().next_back();
                let after = line[end_lb..].chars().next();
                if let (Some(a), Some(b)) = (before, after)
                    && a.is_ascii_punctuation()
                    && b.is_ascii_punctuation()
                    && self.joins(face, a, b)
                {
                    continue;
                }
            }
            let mut start_lb = end_lb;
            for (i, c) in line[start_word..end_lb].char_indices().rev() {
                if c.is_whitespace() {
                    start_lb = start_word + i;
                } else {
                    break;
                }
            }
            if start_word < start_lb {
                let glyphs = self.shape_run(face, line, start_word..start_lb);
                words.push(Word {
                    blank: false,
                    glyphs,
                });
            }
            for (i, c) in line[start_lb..end_lb].char_indices() {
                let at = start_lb + i;
                let glyphs = self.shape_run(face, line, at..at + c.len_utf8());
                words.push(Word {
                    blank: true,
                    glyphs,
                });
            }
            start_word = end_lb;
        }
        // A tab advances to the next tab stop.
        let mut x = 0.0;
        for word in &mut words {
            for g in &mut word.glyphs {
                if line.get(g.start..g.end) == Some("\t") {
                    let tab = TAB_WIDTH * g.x_advance;
                    let stop = ((x / tab).floor() + 1.0) * tab;
                    g.x_advance = stop - x;
                }
                x += g.x_advance;
            }
        }
        words
    }

    /// Wraps one paragraph's `words` to `width` and appends its lines (at
    /// least one, empty for an empty paragraph) to `out`.
    fn place(&self, words: &[Word], size: f32, width: Option<f32>, out: &mut Vec<Line>) {
        let max = width.unwrap_or(f32::INFINITY);
        let mut visual = Vec::new();
        let mut current = Visual::default();
        let mut range_w = 0.0;
        let mut w_before_blank = 0.0;
        let mut fitting_start = Pos::default();
        for (i, word) in words.iter().enumerate() {
            let word_w = word.width(size);
            // One blank past the limit still fits: it isn't counted in the
            // line's width.
            if current.w + (range_w + word_w) <= max || word.blank && current.w + range_w <= max {
                if word.blank {
                    w_before_blank = range_w;
                }
                range_w += word_w;
            } else if word_w > max {
                // Too wide for any line: start it on a new line and break it
                // between glyphs.
                if range_w > 0.0 {
                    current.add(fitting_start, Pos::new(i, 0), range_w);
                    visual.push(std::mem::take(&mut current));
                    range_w = 0.0;
                    fitting_start = Pos::new(i, 0);
                }
                for (gi, g) in word.glyphs.iter().enumerate() {
                    let gw = g.width(size);
                    if current.w + (range_w + gw) <= max {
                        range_w += gw;
                    } else {
                        current.add(fitting_start, Pos::new(i, gi), range_w);
                        visual.push(std::mem::take(&mut current));
                        range_w = gw;
                        fitting_start = Pos::new(i, gi);
                    }
                }
            } else {
                // Wrap before this word, dropping a blank just before it.
                if range_w > 0.0 {
                    if i > 0 && words[i - 1].blank {
                        current.add(fitting_start, Pos::new(i - 1, 0), w_before_blank);
                    } else {
                        current.add(fitting_start, Pos::new(i, 0), range_w);
                    }
                }
                if !current.ranges.is_empty() {
                    visual.push(std::mem::take(&mut current));
                }
                if word.blank {
                    range_w = 0.0;
                    fitting_start = Pos::new(i + 1, 0);
                } else {
                    range_w = word_w;
                    fitting_start = Pos::new(i, 0);
                }
            }
        }
        current.add(fitting_start, Pos::new(words.len(), 0), range_w);
        if !current.ranges.is_empty() {
            visual.push(current);
        }

        let first = out.len();
        for v in visual.iter().filter(|v| !v.ranges.is_empty()) {
            let mut line = Line {
                glyphs: Vec::new(),
                w: v.w,
                ascent: 0.0,
                descent: 0.0,
            };
            let mut x = 0.0;
            for &(start, end) in &v.ranges {
                for (wi, word) in words
                    .iter()
                    .enumerate()
                    .take(end.word + usize::from(end.glyph != 0))
                    .skip(start.word)
                {
                    let from = if wi == start.word { start.glyph } else { 0 };
                    let to = if wi == end.word {
                        end.glyph
                    } else {
                        word.glyphs.len()
                    };
                    for g in &word.glyphs[from..to] {
                        let face = &self.faces[g.face];
                        line.glyphs.push(Placed {
                            face: g.face,
                            id: g.id,
                            x,
                            x_offset: g.x_offset,
                            y_offset: g.y_offset,
                        });
                        x += g.width(size);
                        line.ascent = line.ascent.max(size * face.ascent);
                        line.descent = line.descent.max(size * face.descent);
                    }
                }
            }
            out.push(line);
        }
        if out.len() == first {
            out.push(Line {
                glyphs: Vec::new(),
                w: 0.0,
                ascent: 0.0,
                descent: 0.0,
            });
        }
    }

    /// The text actually laid out in `block`, one paragraph per line (after
    /// replacement), and whether every shaped glyph exists in the font (none
    /// is glyph 0, `.notdef`).
    #[cfg(test)]
    pub fn laid_out(block: &Block) -> (String, bool) {
        let all_found = block
            .lines
            .iter()
            .all(|l| l.glyphs.iter().all(|g| g.id != 0));
        (block.text.clone(), all_found)
    }

    /// Every character the regular face's cmap maps to a glyph.
    #[cfg(test)]
    pub fn mapped_chars(&self) -> Vec<char> {
        let Some(cmap) = &self.faces[REGULAR].cmap else {
            return Vec::new();
        };
        cmap.iter()
            .filter(|&(_, glyph)| glyph.to_u32() != 0)
            .filter_map(|(cp, _)| char::from_u32(cp))
            .collect()
    }

    /// Single-line width of `text`.
    pub fn measure(&mut self, text: &str, style: Style) -> f32 {
        self.layout(text, style, None).width
    }

    /// Draws `block` with its top-left at (`x`, `y`) in `rgb`, limited to
    /// `clip` if given (a mask the size of `pixmap`).
    ///
    /// Each line's baseline is centered in its line height and snapped down
    /// to a whole pixel; glyphs keep a quarter-pixel horizontal position.
    pub fn draw(
        &mut self,
        block: &mut Block,
        pixmap: &mut Pixmap,
        clip: Option<&Mask>,
        (x, y): (f32, f32),
        rgb: [u8; 3],
    ) {
        let (ox, oy) = (x.round(), y.round());
        let mut pb = PathBuilder::new();
        let mut points = Vec::new();
        let mut ends = Vec::new();
        let mut top = 0.0;
        for line in &block.lines {
            let centering = (block.line_height - (line.ascent + line.descent)) / 2.0;
            let baseline = top + centering + line.ascent;
            top += block.line_height;
            for g in &line.glyphs {
                let face = &self.faces[g.face];
                let gx = quarter_pixel(g.x + block.size * g.x_offset);
                let gy = (baseline - block.size * g.y_offset).trunc();
                points.clear();
                ends.clear();
                face.outline(GlyphId::new(u32::from(g.id)), 0, &mut points, &mut ends);
                let scale = block.size / face.units_per_em;
                let at = |p: Point| (ox + gx + p.x * scale, oy + gy - p.y * scale);
                let mut start = 0;
                for &end in &ends {
                    if end > start {
                        contour(&mut pb, &points[start..end], at);
                    }
                    start = start.max(end);
                }
            }
        }
        let Some(path) = pb.finish() else {
            return;
        };
        let mut paint = Paint::default();
        paint.set_color_rgba8(rgb[0], rgb[1], rgb[2], 255);
        paint.anti_alias = true;
        let clip = clip.filter(|m| m.width() == pixmap.width() && m.height() == pixmap.height());
        pixmap.fill_path(
            &path,
            &paint,
            FillRule::Winding,
            Transform::identity(),
            clip,
        );
    }
}

/// The paragraphs of `text`: split at each paragraph separator (`\r\n`
/// counts as one), without a trailing empty one. The caption never contains
/// line breaks (see `caption.rs`), so this is a single paragraph in practice.
fn paragraphs(text: &str) -> impl Iterator<Item = &str> {
    let mut rest = Some(text);
    std::iter::from_fn(move || {
        let s = rest?;
        match s.find(['\n', '\r', '\u{85}', '\u{2029}']) {
            Some(i) => {
                let len = if s[i..].starts_with("\r\n") {
                    2
                } else {
                    s[i..].chars().next().map_or(1, char::len_utf8)
                };
                let tail = &s[i + len..];
                rest = (!tail.is_empty()).then_some(tail);
                Some(&s[..i])
            }
            None => {
                rest = None;
                Some(s)
            }
        }
    })
}

/// `x` rounded to the nearest quarter pixel, the glyph positions
/// cosmic-text's glyph cache used.
fn quarter_pixel(x: f32) -> f32 {
    (x * 4.0).round() / 4.0
}

/// Adds one closed TrueType contour (on-curve points and quadratic control
/// points) to `pb`, mapping each point to pixels with `at`.
fn contour(pb: &mut PathBuilder, points: &[Point], at: impl Fn(Point) -> (f32, f32)) {
    let mid = |a: (f32, f32), b: (f32, f32)| ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
    let n = points.len();
    // Start on an on-curve point, or between the last and first points if
    // there's none.
    let (start, order): ((f32, f32), Vec<usize>) = match points.iter().position(|p| p.on) {
        Some(s) => (at(points[s]), (1..=n).map(|k| (s + k) % n).collect()),
        None => (mid(at(points[n - 1]), at(points[0])), (0..n).collect()),
    };
    pb.move_to(start.0, start.1);
    let mut control: Option<(f32, f32)> = None;
    for i in order {
        let p = at(points[i]);
        if points[i].on {
            match control.take() {
                Some(c) => pb.quad_to(c.0, c.1, p.0, p.1),
                None => pb.line_to(p.0, p.1),
            }
        } else {
            if let Some(c) = control {
                let m = mid(c, p);
                pb.quad_to(c.0, c.1, m.0, m.1);
            }
            control = Some(p);
        }
    }
    if let Some(c) = control {
        pb.quad_to(c.0, c.1, start.0, start.1);
    }
    pb.close();
}

/// `letter` (one character and the combining marks after it) in NFC:
/// canonically decomposed, its marks put in canonical order, then composed
/// (Unicode §3.11). This is `str::nfc()` for a single letter, built from the
/// crate's per-character functions so the compatibility-decomposition tables,
/// which NFC never uses, don't reach the wasm. Letters are composed one at a
/// time; the only compositions that would cross letters join two characters
/// with combining class 0 (Hangul jamo, a few Indic vowel signs), none of
/// which the subset covers either way.
fn nfc_letter(letter: &str) -> Vec<char> {
    let mut chars = Vec::with_capacity(letter.len());
    for c in letter.chars() {
        decompose_canonical(c, |d| chars.push(d));
    }
    // Canonical ordering: a stable sort of each run of marks by class.
    for run in chars.chunk_by_mut(|&a, &b| {
        canonical_combining_class(a) != 0 && canonical_combining_class(b) != 0
    }) {
        run.sort_by_key(|&c| canonical_combining_class(c));
    }
    // Canonical composition: each mark joins the last starter unless a
    // character between them has class 0 or a class at least its own.
    let mut out: Vec<char> = Vec::with_capacity(chars.len());
    let mut starter: Option<usize> = None;
    let mut last_class: Option<u8> = None;
    for c in chars {
        let class = canonical_combining_class(c);
        if let Some(s) = starter
            && last_class.is_none_or(|l| l < class)
            && let Some(composed) = compose(out[s], c)
        {
            out[s] = composed;
            continue;
        }
        if class == 0 {
            starter = Some(out.len());
            last_class = None;
        } else {
            last_class = Some(class);
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use unicode_normalization::UnicodeNormalization;

    use super::*;

    fn nfc(s: &str) -> String {
        s.nfc().collect()
    }

    fn ours(s: &str) -> String {
        nfc_letter(s).into_iter().collect()
    }

    /// Every character, alone and fully decomposed: `nfc_letter` agrees with
    /// the crate's own `str::nfc()` whenever the input is one letter.
    #[test]
    fn nfc_letter_matches_str_nfc() {
        let one_letter = |s: &str| s.chars().skip(1).all(|c| canonical_combining_class(c) != 0);
        let mut checked = 0;
        for c in (0..=0x10_FFFF).filter_map(char::from_u32) {
            let s = c.to_string();
            assert_eq!(ours(&s), nfc(&s), "U+{:04X}", u32::from(c));
            let d: String = s.nfd().collect();
            if one_letter(&d) {
                assert_eq!(ours(&d), nfc(&d), "NFD of U+{:04X}", u32::from(c));
                checked += 1;
            }
        }
        assert!(checked > 1000, "{checked}");
    }

    #[test]
    fn nfc_letter_orders_and_blocks_marks() {
        for s in [
            // Out of canonical order: dot below (220) after acute (230).
            "e\u{301}\u{323}",
            "e\u{323}\u{301}",
            // Precomposed letter plus another mark.
            "\u{e9}\u{323}",
            // Two marks of one class: the second is blocked from composing.
            "a\u{301}\u{300}",
            // A mark that never composes, between the letter and one that does.
            "a\u{31b}\u{301}",
            "u\u{31b}\u{301}",
            // A lone mark.
            "\u{301}",
            "\u{301}\u{323}",
        ] {
            assert_eq!(ours(s), nfc(s), "{s:?}");
        }
    }
}
