//! Snapshot of the text layout (line breaks and glyph positions) for a fixed
//! corpus (issue #51).
//!
//! #48 replaced cosmic-text with the layout in `text.rs` and was checked
//! against it on a large corpus, but that side-by-side test went with
//! cosmic-text. These snapshots take its place: they were taken from the
//! layout after #48, which matched cosmic-text 0.19 on everything here except
//! U+2029 (see [`paragraph_separators`]), so any change to where a line
//! breaks, which glyph is drawn, or where a glyph lands fails a test.
//!
//! Positions are printed as drawn: a glyph's x is snapped to the quarter
//! pixel [`Text::draw`] uses, and a line's baseline to the whole pixel it
//! uses. Widths are rounded to 1/100 px. Layout is plain `f32` arithmetic
//! (no fused multiply-add, no platform math functions), so it is the same on
//! x86_64 (CI) and aarch64; the rounding only keeps the fixture readable.
//!
//! A change to these snapshots is a change to the exported PNG, so review a
//! regeneration like a golden update. To regenerate after an intended change:
//!
//! ```text
//! INSTA_UPDATE=always cargo test -p paste2prompt-core layout_tests
//! ```
//!
//! (or `cargo insta test -p paste2prompt-core --review` with cargo-insta),
//! then read the diff in `src/render/text/snapshots/` before committing it.

use std::collections::HashMap;
use std::fmt::Write as _;

use super::*;

/// Font sizes laid out: the caption's smallest size and a large one
/// (`caption_font_size` clamps to 16..=96).
const SIZES: [f32; 2] = [16.0, 37.0];

/// Wrap widths laid out, in pixels; `None` is a single unwrapped line.
const WIDTHS: [Option<f32>; 5] = [None, Some(400.0), Some(200.0), Some(120.0), Some(60.0)];

const REGULAR_STYLE: bool = false;
const BOLD_STYLE: bool = true;

/// Rounds `x` to 1/100 for printing, without a negative zero.
fn r2(x: f32) -> f32 {
    let r = (x * 100.0).round() / 100.0;
    if r == 0.0 { 0.0 } else { r }
}

/// The character each face's cmap maps to each glyph (the lowest one if
/// several map to it), to print a line's text next to its glyph ids.
fn chars_by_glyph(text: &Text) -> HashMap<(usize, u16), char> {
    let mut map = HashMap::new();
    for (face, f) in text.faces.iter().enumerate() {
        let Some(cmap) = &f.cmap else { continue };
        for (cp, glyph) in cmap.iter() {
            let (Some(c), Ok(id @ 1..)) = (char::from_u32(cp), u16::try_from(glyph.to_u32()))
            else {
                continue;
            };
            map.entry((face, id))
                .and_modify(|old: &mut char| *old = (*old).min(c))
                .or_insert(c);
        }
    }
    map
}

/// One layout as text: the block's size, then per line its width, baseline
/// and text (a glyph no character maps to, such as a ligature or glyph 0,
/// shows as `<id>`), then each glyph as `id@x` (`b` before the id for the
/// bold face, and `+dx,dy` for a nonzero shaping offset, in pixels).
fn dump(text: &mut Text, s: &str, style: Style, width: Option<f32>) -> String {
    let chars = chars_by_glyph(text);
    let block = text.layout(s, style, width);
    let mut out = String::new();
    let width = width.map_or("none".to_owned(), |w| format!("{w}"));
    let weight = if style.bold { "bold" } else { "regular" };
    writeln!(
        out,
        "size={} {weight} width={width}: block {}x{}, {} line(s)",
        style.size,
        r2(block.width),
        r2(block.height),
        block.lines.len()
    )
    .unwrap();
    let mut top = 0.0;
    for (i, line) in block.lines.iter().enumerate() {
        // The same baseline as `Text::draw`.
        let centering = (block.line_height - (line.ascent + line.descent)) / 2.0;
        let baseline = (top + centering + line.ascent).trunc();
        top += block.line_height;
        let mut line_text = String::new();
        for g in &line.glyphs {
            match chars.get(&(g.face, g.id)) {
                Some(&c) => line_text.push(c),
                None => write!(line_text, "<{}>", g.id).unwrap(),
            }
        }
        write!(
            out,
            "  L{i} w={} base={baseline} {line_text:?}:",
            r2(line.w)
        )
        .unwrap();
        for g in &line.glyphs {
            let face = if g.face == BOLD { "b" } else { "" };
            let x = quarter_pixel(g.x + block.size * g.x_offset);
            write!(out, " {face}{}@{x}", g.id).unwrap();
            if g.x_offset != 0.0 || g.y_offset != 0.0 {
                write!(
                    out,
                    "+{},{}",
                    r2(block.size * g.x_offset),
                    r2(block.size * g.y_offset)
                )
                .unwrap();
            }
        }
        out.push('\n');
    }
    out
}

/// Every string in `corpus` at every size and width in `weights`.
fn snapshot(corpus: &[&str], weights: &[bool]) -> String {
    let mut text = Text::new();
    let mut out = String::new();
    for s in corpus {
        writeln!(out, "== {s:?}").unwrap();
        for &bold in weights {
            for size in SIZES {
                for width in WIDTHS {
                    out.push_str(&dump(&mut text, s, Style { size, bold }, width));
                }
            }
        }
        out.push('\n');
    }
    out
}

/// Ordinary prose wrapped at several widths, with the rules of `place()`:
/// whitespace past the limit still fits, and runs of spaces at a wrap are
/// dropped from the line's width.
#[test]
fn wrapping() {
    insta::assert_snapshot!(snapshot(
        &[
            "The quick brown fox jumps over the lazy dog.",
            "move right 15%, up 4%; rotate 90° clockwise about center (70%,18%)",
            "aaaa     bbbb     cccc     dddd",
            "trailing blanks     ",
            "   leading blanks",
            "tab\tstops\tand\u{a0}no-break\u{a0}spaces\u{3000}ideographic",
            "a-b c/d e—f (g) [h] \"i\" 'j'",
            "",
            " ",
        ],
        &[REGULAR_STYLE],
    ));
}

/// A word wider than the line breaks between glyphs, and the `->` / `=>`
/// ligatures are never split.
#[test]
fn long_words_and_ligatures() {
    insta::assert_snapshot!(snapshot(
        &[
            "Supercalifragilisticexpialidocious",
            "short Pneumonoultramicroscopicsilicovolcanoconiosis short",
            "WWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWW",
            "x->y a=>b c-->d -> => ->->->->->->->->->->",
            "https://example.com/a/very/long/path/without/any/spaces/at/all",
        ],
        &[REGULAR_STYLE],
    ));
}

/// Mandatory breaks: LF, CR, CRLF (one break), NEL, and blank paragraphs.
#[test]
fn mandatory_breaks() {
    insta::assert_snapshot!(snapshot(
        &[
            "line one\nline two\r\nline three\rfour\u{85}five",
            "a\n\nb",
            "\nleading and trailing\n",
            "wrapped first paragraph that is long\nsecond",
        ],
        &[REGULAR_STYLE],
    ));
}

/// U+2029 PARAGRAPH SEPARATOR breaks the line, like LF. cosmic-text 0.19
/// kept it inline as glyph 0, so this is the one case found in #48's review
/// where the layout differs from it. The export never reaches it, since
/// `caption.rs` splits caption text at U+2029 (and the other line
/// separators) before layout. U+2028 LINE SEPARATOR is not split by
/// `paragraphs()`: it stays on the line as glyph 0, which has an advance but
/// no outline (also unreachable from the export).
#[test]
fn paragraph_separators() {
    let mut text = Text::new();
    let style = Style {
        size: 16.0,
        bold: false,
    };
    let lines = |text: &mut Text, s: &str| text.layout(s, style, None).lines.len();
    assert_eq!(lines(&mut text, "a\u{2029}b"), 2);
    assert_eq!(lines(&mut text, "a\nb"), 2);
    assert_eq!(lines(&mut text, "a\u{2029}"), 1, "no trailing empty line");
    insta::assert_snapshot!(snapshot(
        &[
            "line one\nline two\r\nline three\u{2029}four",
            "a\u{2029}\u{2029}b",
            "a\u{2028}b",
        ],
        &[REGULAR_STYLE],
    ));
}

/// Accents typed precomposed and decomposed (NFD) lay out the same, since
/// decomposed letters are composed before layout; a letter with no
/// precomposed form becomes U+FFFD.
#[test]
fn accents() {
    let precomposed = "café naïve résumé Ångström façade über señor";
    let nfd = "cafe\u{301} nai\u{308}ve re\u{301}sume\u{301} A\u{30a}ngstro\u{308}m fac\u{327}ade u\u{308}ber sen\u{303}or";
    let mut text = Text::new();
    for size in SIZES {
        for width in WIDTHS {
            let style = Style { size, bold: false };
            assert_eq!(
                dump(&mut text, precomposed, style, width),
                dump(&mut text, nfd, style, width),
                "size {size}, width {width:?}"
            );
        }
    }
    insta::assert_snapshot!(snapshot(
        &[
            precomposed,
            nfd,
            "q\u{303} x\u{301}\u{323} \u{301}lone mark",
            "ĀāĂăĄąĆćĈĉĊċČčĎďĐđĒēĔĕĖėĘęĚěĜĝĞğ",
        ],
        &[REGULAR_STYLE],
    ));
}

/// Greek and Cyrillic, which the regular subset covers.
#[test]
fn greek_and_cyrillic() {
    insta::assert_snapshot!(snapshot(
        &[
            "Καλημέρα κόσμε, ΑΒΓΔ αβγδ ά έ ή ί ό ύ ώ ϊ ϋ",
            "Съешь же ещё этих мягких французских булок, да выпей чаю",
            "mixed Latin Ελληνικά Кириллица text",
        ],
        &[REGULAR_STYLE],
    ));
}

/// Characters outside the subset: each run becomes one U+FFFD, whitespace
/// between runs is kept, and a typed U+FFFD draws the same glyph.
#[test]
fn replacement_runs() {
    insta::assert_snapshot!(snapshot(
        &[
            "日本語のテキスト",
            "before 日本語 after",
            "emoji 😀😀😀 and 🎉 here",
            "عربي עברית 中文 한국어",
            "typed \u{fffd} and \u{fffd}\u{fffd}",
            "a日b本c語d",
        ],
        &[REGULAR_STYLE],
    ));
}

/// Regular next to bold, including bold's fallback to the regular face for
/// characters its ASCII-only subset lacks.
#[test]
fn regular_and_bold() {
    insta::assert_snapshot!(snapshot(
        &[
            "paste2prompt edit list (user-authored)",
            "#1 #2 #10 #99",
            "Bold café Ωμέγα Привет → arrows",
            "日本語 bold fallback",
        ],
        &[REGULAR_STYLE, BOLD_STYLE],
    ));
}
