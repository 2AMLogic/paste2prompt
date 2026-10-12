//! Export renderer tests, including golden images.
//!
//! Goldens live in `crates/core/tests/golden/`. To regenerate them after an
//! intended change to the export's look, run
//! `UPDATE_GOLDENS=1 cargo test -p paste2prompt-core render` and review the
//! new PNGs before committing them.

use std::borrow::Cow;
use std::path::PathBuf;

use super::*;
use crate::{Anchor, Caption, Editor, MirrorAxis, Op, Point, Region};

/// A deterministic test image: a diagonal gradient with a few flat
/// "UI" blocks, so marks are tested over light, dark and mid tones.
fn test_image(width: u32, height: u32) -> RgbaImage {
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            let (fx, fy) = (x as f32 / width as f32, y as f32 / height as f32);
            let mut px = [
                (40.0 + 200.0 * fx) as u8,
                (60.0 + 160.0 * fy) as u8,
                (220.0 - 120.0 * fx * fy) as u8,
                255,
            ];
            if (0.1..0.45).contains(&fx) && (0.15..0.5).contains(&fy) {
                px = [250, 250, 250, 255];
            }
            if (0.55..0.9).contains(&fx) && (0.6..0.85).contains(&fy) {
                px = [24, 26, 30, 255];
            }
            pixels.extend_from_slice(&px);
        }
    }
    RgbaImage {
        width,
        height,
        pixels,
    }
}

fn pt(x: f64, y: f64) -> Point {
    Point::new(x, y)
}

fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Region {
    Region::rect(pt(x0, y0), pt(x1, y1)).unwrap()
}

fn size_of(image: &RgbaImage) -> ImageSize {
    ImageSize::new(image.width, image.height).unwrap()
}

/// A narrow image (padded to the minimum width) with translate, rotate and a
/// lasso hint, as in the spec's example caption.
fn fixture_basic() -> (RgbaImage, Document) {
    let image = test_image(400, 260);
    let mut ed = Editor::new(size_of(&image));
    ed.add_edit(
        rect(0.12, 0.30, 0.40, 0.52),
        Op::Translate {
            dx: 0.15,
            dy: -0.04,
        },
        None,
    )
    .unwrap();
    ed.add_edit(
        rect(0.60, 0.10, 0.80, 0.25),
        Op::Rotate {
            degrees: 90.0,
            anchor: Anchor::Center,
        },
        None,
    )
    .unwrap();
    ed.add_edit(
        Region::polygon([
            pt(0.45, 0.62),
            pt(0.58, 0.66),
            pt(0.55, 0.80),
            pt(0.42, 0.76),
        ])
        .unwrap(),
        Op::Hint,
        Some("make this button red".into()),
    )
    .unwrap();
    ed.set_instructions("tighten the spacing between the cards");
    (image, ed.into_document())
}

/// A wider image with every op: scale, both mirrors, a translate whose ghost
/// leaves the image, a rotated lasso, a hint on a translate, and multi-line
/// instructions with non-ASCII text.
fn fixture_all_ops() -> (RgbaImage, Document) {
    let image = test_image(960, 600);
    let mut ed = Editor::new(size_of(&image));
    ed.add_edit(
        rect(0.10, 0.15, 0.30, 0.35),
        Op::Scale {
            sx: 1.5,
            sy: 1.5,
            anchor: Anchor::Center,
        },
        None,
    )
    .unwrap();
    ed.add_edit(
        rect(0.40, 0.10, 0.55, 0.40),
        Op::Mirror {
            axis: MirrorAxis::Horizontal,
        },
        None,
    )
    .unwrap();
    ed.add_edit(
        rect(0.62, 0.12, 0.92, 0.30),
        Op::Mirror {
            axis: MirrorAxis::Vertical,
        },
        None,
    )
    .unwrap();
    ed.add_edit(
        rect(0.70, 0.55, 0.95, 0.75),
        Op::Translate { dx: 0.20, dy: 0.10 },
        Some("and make it blue".into()),
    )
    .unwrap();
    ed.add_edit(
        Region::polygon([
            pt(0.15, 0.6),
            pt(0.35, 0.55),
            pt(0.40, 0.85),
            pt(0.20, 0.90),
        ])
        .unwrap(),
        Op::Rotate {
            degrees: -30.0,
            anchor: Anchor::Center,
        },
        None,
    )
    .unwrap();
    ed.set_instructions("Keep the header.\nRenommer « Größe » en Ελληνικά — Привет");
    (image, ed.into_document())
}

/// Named regions (#26): a label (a name and nothing else) across the top, a
/// named translate, a named lasso whose name has non-ASCII letters (the bold
/// badge face is ASCII-only, so they fall back to the regular face) and an
/// unnamed rotate. Badges read `1 nav bar`, `2 logo`, `3 Größe`, `4`, and the
/// caption gets the names header line.
fn fixture_named() -> (RgbaImage, Document) {
    let image = test_image(800, 500);
    let mut ed = Editor::new(size_of(&image));
    ed.add_named_edit(
        rect(0.02, 0.03, 0.98, 0.11),
        Op::Label,
        None,
        Some("nav bar".into()),
    )
    .unwrap();
    ed.add_named_edit(
        rect(0.12, 0.30, 0.40, 0.48),
        Op::Translate { dx: 0.15, dy: 0.25 },
        None,
        Some("logo".into()),
    )
    .unwrap();
    ed.add_named_edit(
        Region::polygon([
            pt(0.58, 0.62),
            pt(0.80, 0.60),
            pt(0.86, 0.82),
            pt(0.62, 0.86),
        ])
        .unwrap(),
        Op::Hint,
        Some("make it the brand green".into()),
        Some("Größe".into()),
    )
    .unwrap();
    ed.add_edit(
        rect(0.60, 0.22, 0.75, 0.40),
        Op::Rotate {
            degrees: 15.0,
            anchor: Anchor::Center,
        },
        None,
    )
    .unwrap();
    ed.set_instructions("put the logo under the nav bar, left of Größe");
    (image, ed.into_document())
}

/// No edits and no instructions: just the header.
fn fixture_empty() -> (RgbaImage, Document) {
    let image = test_image(720, 120);
    let doc = Document::new(size_of(&image));
    (image, doc)
}

/// A half-transparent image, which must be composited over white.
fn fixture_transparent() -> (RgbaImage, Document) {
    let mut image = test_image(160, 160);
    for (i, px) in image.pixels.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        px[3] = if (i / 160) < 80 { 0 } else { 128 };
    }
    let mut ed = Editor::new(size_of(&image));
    ed.add_edit(
        rect(0.0, 0.0, 1.0, 1.0),
        Op::Hint,
        Some("fill the transparent half".into()),
    )
    .unwrap();
    (image, ed.into_document())
}

/// Hint and instructions in scripts the Inter subset can't draw (CJK, emoji,
/// Arabic, Hebrew), mixed with Latin text it can.
const UNMAPPABLE_HINT: &str = "把按钮改成绿色 🙂 مرحبا שלום — ok";
const UNMAPPABLE_INSTRUCTIONS: &str = "全体: 余白を詰める 🚀";

fn fixture_unmappable() -> (RgbaImage, Document) {
    let image = test_image(800, 400);
    let mut ed = Editor::new(size_of(&image));
    ed.add_edit(
        rect(0.2, 0.2, 0.5, 0.5),
        Op::Hint,
        Some(UNMAPPABLE_HINT.into()),
    )
    .unwrap();
    ed.set_instructions(UNMAPPABLE_INSTRUCTIONS);
    (image, ed.into_document())
}

fn rgb_of(pixmap: &Pixmap) -> Vec<u8> {
    pixmap
        .data()
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|p| [p[0], p[1], p[2]])
        .collect()
}

fn golden_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(format!("{name}.png"))
}

fn decode_png(bytes: &[u8]) -> (u32, u32, png::ColorType, Vec<u8>) {
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let mut reader = decoder.read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buf).unwrap();
    buf.truncate(info.buffer_size());
    (info.width, info.height, info.color_type, buf)
}

/// Per-channel difference under which two pixels count as equal; absorbs
/// anti-aliasing differences between SIMD backends (CI is x86_64, dev machines
/// may be aarch64).
const CHANNEL_TOLERANCE: u8 = 24;
/// Share of pixels allowed to differ by more than the channel tolerance.
const PIXEL_TOLERANCE: f64 = 0.002;

fn check_golden(name: &str, (image, doc): (RgbaImage, Document)) {
    let png = render_export(&image, &doc).unwrap();
    let path = golden_path(name);
    if std::env::var_os("UPDATE_GOLDENS").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &png).unwrap();
        return;
    }
    let golden = std::fs::read(&path).unwrap_or_else(|e| {
        panic!(
            "missing golden {} ({e}); run with UPDATE_GOLDENS=1",
            path.display()
        )
    });
    let (gw, gh, _, want) = decode_png(&golden);
    let (w, h, _, got) = decode_png(&png);
    assert_eq!((w, h), (gw, gh), "{name}: export size changed");
    let differing = got
        .as_chunks::<3>()
        .0
        .iter()
        .zip(want.as_chunks::<3>().0.iter())
        .filter(|(a, b)| {
            a.iter()
                .zip(b.iter())
                .any(|(x, y)| x.abs_diff(*y) > CHANNEL_TOLERANCE)
        })
        .count();
    let share = differing as f64 / f64::from(w * h);
    assert!(
        share <= PIXEL_TOLERANCE,
        "{name}: {differing} pixels ({:.3}%) differ from {}; if intended, rerun with UPDATE_GOLDENS=1",
        share * 100.0,
        path.display()
    );
}

#[test]
fn golden_basic() {
    check_golden("basic", fixture_basic());
}

#[test]
fn golden_all_ops() {
    check_golden("all_ops", fixture_all_ops());
}

#[test]
fn golden_empty() {
    check_golden("empty", fixture_empty());
}

#[test]
fn golden_transparent() {
    check_golden("transparent", fixture_transparent());
}

#[test]
fn golden_unmappable() {
    check_golden("unmappable", fixture_unmappable());
}

#[test]
fn golden_named() {
    check_golden("named", fixture_named());
}

#[test]
fn a_named_badge_is_wider_and_shows_the_name() {
    let (image, doc) = fixture_named();
    let scale = MarkScale::for_image(size_of(&image));
    let mut text = Text::new();
    let plain = scale.badge_extent(&mut text, 2, None);
    let named = scale.badge_extent(&mut text, 2, Some("logo"));
    assert_eq!(plain.1, named.1, "same height");
    assert!(named.0 > plain.0 * 2.0, "{named:?} vs {plain:?}");
    assert_eq!(marks::badge_label(2, Some("logo")), "2 logo");
    assert_eq!(marks::badge_label(12, None), "12");
    // Unmappable letters in a name are drawn as U+FFFD, like any text.
    assert_eq!(text.drawable("logo 中文"), "logo \u{FFFD}");

    // The source badges are placed by their full width (`badge_positions`
    // keeps that whole capsule off its region; see placement's tests).
    let badges = badge_positions(&doc, |n, name| scale.badge_extent(&mut text, n, name));
    for (i, (edit, b)) in doc.edits().iter().zip(&badges).enumerate() {
        assert_eq!(b.size, scale.badge_extent(&mut text, i + 1, edit.name()));
    }
    assert!(badges[0].size.0 > badges[3].size.0 * 2.0);
    let caption = Caption::from_document(&doc).to_plain_text();
    assert!(caption.contains(crate::NAMES_HEADER), "{caption}");
    assert!(
        caption.contains(r#"#2  translate  "logo" (12%,30%)-(40%,48%)"#),
        "{caption}"
    );
}

#[test]
fn rendering_is_deterministic() {
    let (image, doc) = fixture_all_ops();
    assert_eq!(
        render_export(&image, &doc).unwrap(),
        render_export(&image, &doc).unwrap()
    );
}

/// Walks the PNG chunk list.
fn chunk_types(png: &[u8]) -> Vec<String> {
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    let mut types = Vec::new();
    let mut i = 8;
    while i + 8 <= png.len() {
        let len = u32::from_be_bytes(png[i..i + 4].try_into().unwrap()) as usize;
        types.push(String::from_utf8_lossy(&png[i + 4..i + 8]).into_owned());
        i += 12 + len;
    }
    types
}

#[test]
fn output_is_plain_opaque_rgb_png_without_text_chunks() {
    let (image, doc) = fixture_basic();
    let png = render_export(&image, &doc).unwrap();
    let types = chunk_types(&png);
    assert_eq!(types.first().map(String::as_str), Some("IHDR"));
    assert_eq!(types.last().map(String::as_str), Some("IEND"));
    assert!(
        types
            .iter()
            .all(|t| t == "IHDR" || t == "IDAT" || t == "IEND"),
        "unexpected chunks: {types:?}"
    );
    let (_, _, color, _) = decode_png(&png);
    assert_eq!(color, png::ColorType::Rgb);
}

#[test]
fn narrow_images_are_padded_and_wide_ones_keep_their_width() {
    let (image, doc) = fixture_basic();
    let p = render_pixmap(&image, &doc).unwrap();
    assert_eq!(p.width(), MIN_EXPORT_WIDTH);
    assert!(p.height() > image.height);
    // The image is centered: its first pixel sits after the left padding.
    let ox = (MIN_EXPORT_WIDTH - image.width) / 2;
    let px = p.pixel(ox, 0).unwrap();
    assert_eq!([px.red(), px.green(), px.blue()], [40, 60, 220]);
    let pad = p.pixel(0, 0).unwrap();
    assert_eq!([pad.red(), pad.green(), pad.blue()], PADDING_BG);

    let (image, doc) = fixture_all_ops();
    assert_eq!(render_pixmap(&image, &doc).unwrap().width(), image.width);
}

#[test]
fn original_pixels_survive_outside_the_marks() {
    let (image, doc) = fixture_all_ops();
    let p = render_pixmap(&image, &doc).unwrap();
    // Bottom-right corner of the image: no mark near it.
    let (x, y) = (image.width - 2, image.height - 2);
    let i = ((y * image.width + x) * 4) as usize;
    let px = p.pixel(x, y).unwrap();
    assert_eq!(
        [px.red(), px.green(), px.blue()],
        [image.pixels[i], image.pixels[i + 1], image.pixels[i + 2]]
    );
}

#[test]
fn outline_and_caption_badge_use_the_edit_color() {
    let (image, doc) = fixture_all_ops();
    let p = render_pixmap(&image, &doc).unwrap();
    let rgb = rgb_of(&p);
    let w = p.width() as usize;
    let count = |color: [u8; 3], rows: std::ops::Range<u32>| {
        rows.flat_map(|y| (0..p.width()).map(move |x| (x, y)))
            .filter(|&(x, y)| {
                let i = (y as usize * w + x as usize) * 3;
                rgb[i..i + 3] == color
            })
            .count()
    };
    for n in 1..=doc.edits().len() {
        let c = mark_color(n);
        assert!(
            count(c, 0..image.height) > 50,
            "edit #{n} has no outline in its color"
        );
        assert!(
            count(c, image.height..p.height()) > 50,
            "edit #{n} has no caption badge in its color"
        );
    }
    // Red first, then distinct colors.
    assert_eq!(mark_color(1), [222, 25, 35]);
    assert_ne!(mark_color(1), mark_color(2));
    assert_eq!(mark_color(1), mark_color(PALETTE.len() + 1));
    assert_eq!(mark_color(0), mark_color(1));
}

#[test]
fn transparent_pixels_become_white() {
    let (image, doc) = fixture_transparent();
    let p = render_pixmap(&image, &doc).unwrap();
    let ox = (p.width() - image.width) / 2;
    let px = p.pixel(ox + 80, 40).unwrap();
    assert_eq!(
        [px.red(), px.green(), px.blue(), px.alpha()],
        [255, 255, 255, 255]
    );
}

#[test]
fn rejects_bad_input() {
    let (image, doc) = fixture_basic();

    let mut short = image.clone();
    short.pixels.pop();
    assert!(matches!(
        render_export(&short, &doc),
        Err(RenderError::PixelBufferSize { .. })
    ));

    let other = test_image(401, 260);
    assert!(matches!(
        render_export(&other, &doc),
        Err(RenderError::SizeMismatch { .. })
    ));

    let empty = RgbaImage {
        width: 0,
        height: 10,
        pixels: Vec::new(),
    };
    assert_eq!(render_export(&empty, &doc), Err(RenderError::EmptyImage));

    // Rejected on its dimensions alone, before anything is allocated.
    let huge = RgbaImage {
        width: 10_000,
        height: 5_000,
        pixels: Vec::new(),
    };
    let huge_doc = Document::new(ImageSize::new(10_000, 5_000).unwrap());
    assert!(matches!(
        render_export(&huge, &huge_doc),
        Err(RenderError::TooLarge { .. })
    ));
    let long = RgbaImage {
        width: 1,
        height: MAX_EXPORT_SIDE + 1,
        pixels: Vec::new(),
    };
    let long_doc = Document::new(ImageSize::new(1, MAX_EXPORT_SIDE + 1).unwrap());
    assert!(matches!(
        render_export(&long, &long_doc),
        Err(RenderError::TooLarge { .. })
    ));
    assert!(!RenderError::EmptyImage.to_string().is_empty());
}

#[test]
fn extreme_but_valid_documents_render() {
    // A 1-pixel-wide, tall image; edits whose ghosts land far outside the
    // image; the largest scale; many edits with two-digit numbers; a long hint
    // word that has to wrap mid-word.
    let image = test_image(1, 3000);
    let mut ed = Editor::new(size_of(&image));
    let far = Op::Translate { dx: 1.0, dy: -1.0 };
    for i in 0..24 {
        let y = f64::from(i) / 25.0;
        let op = match i % 4 {
            0 => far,
            1 => Op::uniform_scale(crate::MAX_SCALE),
            2 => Op::Rotate {
                degrees: 7200.0,
                anchor: Anchor::Point(pt(0.0, 1.0)),
            },
            _ => Op::Mirror {
                axis: MirrorAxis::Vertical,
            },
        };
        ed.add_edit(rect(0.0, y, 1.0, y + 0.03), op, Some("x".repeat(300)))
            .unwrap();
    }
    ed.set_instructions("y".repeat(2000));
    let doc = ed.into_document();
    let png = render_export(&image, &doc).unwrap();
    let (w, h, _, _) = decode_png(&png);
    assert_eq!(w, MIN_EXPORT_WIDTH);
    assert!(h > 3000);
}

#[test]
fn caption_rows_stack_when_columns_do_not_fit() {
    // At the minimum width a lasso region string is too wide for columns;
    // at 3000 px it fits. Both must lay out without overlap.
    let (_, doc) = fixture_basic();
    let lines = caption_lines(&doc);
    let mut text = Text::new();
    let narrow = layout_band(&mut text, &lines, MIN_EXPORT_WIDTH as f32);
    let wide = layout_band(&mut text, &lines, 3000.0);
    assert!(narrow.height > 0.0 && wide.height > 0.0);
    for band in [&narrow, &wide] {
        let mut last_bottom = 0.0f32;
        for item in &band.items {
            if let Item::Text { block, y, x, .. } = item {
                assert!(*x >= 0.0);
                assert!(*y + 0.5 >= last_bottom.min(*y), "text goes backwards");
                last_bottom = last_bottom.max(*y + block.height);
            }
        }
        assert!(last_bottom <= band.height);
    }
}

#[test]
fn unmappable_characters_become_one_replacement_per_run() {
    let text = Text::new();
    assert_eq!(text.drawable("ok — Größe Привет"), "ok — Größe Привет");
    assert_eq!(text.drawable("把按钮改成绿色"), "\u{FFFD}");
    assert_eq!(
        text.drawable(UNMAPPABLE_HINT),
        "\u{FFFD} \u{FFFD} \u{FFFD} \u{FFFD} — ok"
    );
    // A run spanning scripts, emoji sequences included, is still one box.
    assert_eq!(text.drawable("a😀👍🏽中b"), "a\u{FFFD}b");
    assert_eq!(text.drawable("a\tb\n"), "a\tb\n");
}

#[test]
fn unmappable_text_is_visible_in_the_export() {
    let (image, doc) = fixture_unmappable();
    let lines = caption_lines(&doc);
    let mut text = Text::new();
    let band = layout_band(&mut text, &lines, image.width.max(MIN_EXPORT_WIDTH) as f32);
    let mut laid_out = String::new();
    for item in &band.items {
        if let Item::Text { block, .. } = item {
            let (s, all_found) = Text::laid_out(block);
            assert!(all_found, "{s:?} shaped to a missing glyph");
            laid_out.push_str(&s);
            laid_out.push('\n');
        }
    }
    assert!(laid_out.contains('\u{FFFD}'), "{laid_out}");
    assert!(laid_out.contains("— ok"), "{laid_out}");
    for c in ['把', '🙂', 'م', 'ש', '全', '🚀'] {
        assert!(!laid_out.contains(c), "{c} reached layout: {laid_out}");
    }

    // The export renders, with ink where the hint's replacement characters
    // are: the caption band is not just background.
    let png = render_export(&image, &doc).unwrap();
    assert!(!png.is_empty());
    let (w, h, _, rgb) = decode_png(&png);
    assert!(w >= image.width && h > image.height);
    let ink = rgb[(image.height * w * 3) as usize..]
        .as_chunks::<3>()
        .0
        .iter()
        .filter(|p| **p == INK)
        .count();
    assert!(ink > 0);

    // Copy caption as text keeps the original characters.
    let plain = Caption::from_document(&doc).to_plain_text();
    assert!(plain.contains(UNMAPPABLE_HINT), "{plain}");
    assert!(plain.contains(UNMAPPABLE_INSTRUCTIONS), "{plain}");
}

/// "Café, Größe, Ñandú, άΐ, йё", once precomposed (NFC) and once decomposed
/// (NFD, base letter plus combining marks, as some macOS sources paste it).
const ACCENTED_NFC: &str =
    "Caf\u{e9}, Gr\u{f6}\u{df}e, \u{d1}and\u{fa}, \u{3ac}\u{390}, \u{439}\u{451}";
const ACCENTED_NFD: &str = "Cafe\u{301}, Gro\u{308}\u{df}e, N\u{303}andu\u{301}, \u{3b1}\u{301}\u{3b9}\u{308}\u{301}, \u{438}\u{306}\u{435}\u{308}";

fn fixture_accented(hint: &str) -> (RgbaImage, Document) {
    let image = test_image(480, 300);
    let mut ed = Editor::new(size_of(&image));
    ed.add_edit(rect(0.2, 0.2, 0.6, 0.5), Op::Hint, Some(hint.into()))
        .unwrap();
    ed.set_instructions(hint);
    (image, ed.into_document())
}

#[test]
fn decomposed_accents_are_drawn_precomposed() {
    let text = Text::new();
    // Guard the fixture: the two spellings differ only in normalization.
    assert_ne!(ACCENTED_NFC, ACCENTED_NFD);
    assert_eq!(text.drawable(ACCENTED_NFD), ACCENTED_NFC);
    assert_eq!(text.drawable("e\u{301}"), "\u{e9}");
    // Several marks on one letter compose too (ι + diaeresis + acute = ΐ).
    assert_eq!(text.drawable("\u{3b9}\u{308}\u{301}"), "\u{390}");
}

#[test]
fn precomposed_text_is_unchanged() {
    let text = Text::new();
    for s in [ACCENTED_NFC, "ok — Größe Привет", "a\tb\n", ""] {
        assert!(
            matches!(text.drawable(s), Cow::Borrowed(b) if b == s),
            "{s:?} was rewritten"
        );
    }
}

#[test]
fn a_mark_with_no_drawable_composition_replaces_its_whole_letter() {
    let text = Text::new();
    // q + tilde has no precomposed form; Inter's subset has no combining
    // marks. The letter and its mark become one `�`, not `q�`.
    assert_eq!(text.drawable("q\u{303}"), "\u{FFFD}");
    assert_eq!(text.drawable("aq\u{303}b"), "a\u{FFFD}b");
    // Every mark on the letter goes with it.
    assert_eq!(text.drawable("xq\u{303}\u{301}\u{323}y"), "x\u{FFFD}y");
    // Composes to a character outside the subset (Vietnamese ệ, U+1EC7):
    // the same `�` as the precomposed spelling, whichever form was typed.
    assert_eq!(text.drawable("e\u{323}\u{302}"), "\u{FFFD}");
    assert_eq!(text.drawable("\u{1ec7}"), "\u{FFFD}");
    assert_eq!(
        text.drawable("Vi\u{1ec7}t"),
        text.drawable("Vie\u{323}\u{302}t")
    );
    // Partly composable: é composes, the extra mark has nowhere to go.
    assert_eq!(text.drawable("e\u{301}\u{330}z"), "\u{FFFD}z");
    // Joins an adjacent run of unmappable characters into one `�`.
    assert_eq!(text.drawable("中q\u{303}文"), "\u{FFFD}");
    // A mark with no letter (at the start, or after whitespace) is its own
    // `�`; the whitespace is kept.
    assert_eq!(text.drawable("\u{301}x"), "\u{FFFD}x");
    assert_eq!(text.drawable("a \u{301}b"), "a \u{FFFD}b");
    // Not a combining mark: a variation selector after a drawable arrow
    // keeps the existing per-character rule.
    assert_eq!(text.drawable("\u{2194}\u{fe0f}"), "\u{2194}\u{FFFD}");
}

#[test]
fn normalizing_never_loses_a_drawable_character() {
    // Every character the subset maps must still be drawable after NFC,
    // or normalizing would turn drawable text into `�`. The trailing `中`
    // keeps `drawable` off its fast path, so the text is normalized.
    let text = Text::new();
    for c in text.mapped_chars() {
        let input = format!("{c}\u{4e2d}");
        let drawn: Vec<char> = text.drawable(&input).chars().collect();
        assert!(
            drawn.len() == 2
                && (drawn[0] != '\u{FFFD}' || c == '\u{FFFD}')
                && drawn[1] == '\u{FFFD}',
            "U+{:04X} is not drawable after normalizing: {drawn:?}",
            u32::from(c)
        );
    }
}

#[test]
fn decomposed_text_exports_like_precomposed_text() {
    let nfd = fixture_accented(ACCENTED_NFD);
    let lines = caption_lines(&nfd.1);
    let mut text = Text::new();
    let band = layout_band(&mut text, &lines, nfd.0.width.max(MIN_EXPORT_WIDTH) as f32);
    let mut laid_out = String::new();
    for item in &band.items {
        if let Item::Text { block, .. } = item {
            let (s, all_found) = Text::laid_out(block);
            assert!(all_found, "{s:?} shaped to a missing glyph");
            laid_out.push_str(&s);
            laid_out.push('\n');
        }
    }
    assert!(!laid_out.contains('\u{FFFD}'), "{laid_out}");
    assert!(laid_out.contains(ACCENTED_NFC), "{laid_out}");

    // The two spellings export to the same image, pixel for pixel.
    let nfc = fixture_accented(ACCENTED_NFC);
    assert_eq!(
        render_export(&nfd.0, &nfd.1).unwrap(),
        render_export(&nfc.0, &nfc.1).unwrap()
    );

    // Copy caption as text keeps the characters as typed, unnormalized.
    let plain = Caption::from_document(&nfd.1).to_plain_text();
    assert!(plain.contains(ACCENTED_NFD), "{plain}");
    assert!(!plain.contains(ACCENTED_NFC), "{plain}");
}

#[test]
fn badges_on_tiny_images_stay_off_the_padding() {
    // A 16×16 image is smaller than a badge: the pill and its number must
    // both be clipped to the image, never drawn on the padding around it.
    let image = test_image(16, 16);
    let mut ed = Editor::new(size_of(&image));
    ed.add_edit(rect(0.0, 0.0, 1.0, 1.0), Op::Hint, Some("tiny".into()))
        .unwrap();
    ed.add_edit(
        rect(0.5, 0.5, 1.0, 1.0),
        Op::Translate { dx: -0.5, dy: 0.0 },
        None,
    )
    .unwrap();
    let doc = ed.into_document();
    let p = render_pixmap(&image, &doc).unwrap();
    let ox = (p.width() - image.width) / 2;
    for y in 0..image.height {
        for x in (0..p.width()).filter(|x| !(ox..ox + image.width).contains(x)) {
            let px = p.pixel(x, y).unwrap();
            assert_eq!(
                [px.red(), px.green(), px.blue()],
                PADDING_BG,
                "mark ink on the padding at ({x}, {y})"
            );
        }
    }
}

#[test]
fn badge_leaves_a_small_region_interior_untouched() {
    // #28: a region smaller than two badges. Its badge goes outside, so
    // every pixel inside the outline's halo is still the original image.
    let image = test_image(800, 600);
    let size = size_of(&image);
    let scale = MarkScale::for_image(size);
    let side = f64::from(scale.badge) * 1.5;
    let (x0, y0) = (300.0, 200.0);
    let mut ed = Editor::new(size);
    ed.add_edit(
        Region::rect_from_pixels(size, (x0, y0), (x0 + side, y0 + side)).unwrap(),
        Op::Hint,
        Some("small".into()),
    )
    .unwrap();
    let doc = ed.into_document();
    let p = render_pixmap(&image, &doc).unwrap();
    let ox = (p.width() - image.width) / 2;
    let inset = (scale.halo / 2.0).ceil() as u32 + 1;
    let (ix0, iy0) = (x0 as u32 + inset, y0 as u32 + inset);
    let (ix1, iy1) = ((x0 + side) as u32 - inset, (y0 + side) as u32 - inset);
    for y in iy0..iy1 {
        for x in ix0..ix1 {
            let i = ((y * image.width + x) * 4) as usize;
            let px = p.pixel(ox + x, y).unwrap();
            assert_eq!(
                [px.red(), px.green(), px.blue()],
                [image.pixels[i], image.pixels[i + 1], image.pixels[i + 2]],
                "mark ink inside the region at ({x}, {y})"
            );
        }
    }
    // And the badge is drawn: its color appears just outside the corner.
    let red = mark_color(1);
    let rgb = rgb_of(&p);
    let w = p.width() as usize;
    let near = (0..y0 as u32)
        .flat_map(|y| (0..x0 as u32).map(move |x| (x, y)))
        .filter(|&(x, y)| {
            let i = (y as usize * w + (ox + x) as usize) * 3;
            rgb[i..i + 3] == red
        })
        .count();
    assert!(near > 100, "no badge up and left of the region ({near} px)");
}
