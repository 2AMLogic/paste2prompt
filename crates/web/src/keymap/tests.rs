//! The keymap, driven natively: every binding in every context, no
//! duplicates, the sheet against the keymap, layout independence (#22) and
//! the controller side of each canvas and history action.

use std::collections::HashSet;

use paste2prompt_core::{EditId, ImageSize};

use super::rules::{
    Action, BINDINGS, Context, KeyInput, Platform, SheetStep, apply, event_letter, hint, lookup,
    non_latin_letter, sheet, sheet_step,
};
use crate::canvas::{Controller, OpKind, Pointer, Tool};

/// A modifier combination, for building events.
#[derive(Clone, Copy, Debug)]
enum M {
    None,
    Shift,
    Ctrl,
    Cmd,
    CtrlShift,
    CmdShift,
}

fn ev<'a>(key: &'a str, code: &'a str, m: M) -> KeyInput<'a> {
    let (ctrl, meta, shift) = match m {
        M::None => (false, false, false),
        M::Shift => (false, false, true),
        M::Ctrl => (true, false, false),
        M::Cmd => (false, true, false),
        M::CtrlShift => (true, false, true),
        M::CmdShift => (false, true, true),
    };
    KeyInput {
        key,
        code,
        ctrl,
        meta,
        shift,
        alt: false,
        alt_graph: false,
    }
}

fn action(input: KeyInput<'_>, ctx: Context) -> Option<Action> {
    lookup(&input, ctx).map(|b| b.action)
}

/// Every context with the sheet closed.
fn contexts() -> impl Iterator<Item = Context> {
    (0..8u8).map(|bits| Context {
        text_field: bits & 1 != 0,
        text_selected: bits & 2 != 0,
        dragging: bits & 4 != 0,
        sheet_open: false,
    })
}

/// One expected shortcut, written from the spec table (not from
/// `BINDINGS`): an event, its action, and the contexts it fires in.
struct Row {
    key: &'static str,
    code: &'static str,
    mods: M,
    action: Action,
    in_text_field: bool,
    with_text_selection: bool,
    during_drag: bool,
}

const fn row(
    key: &'static str,
    code: &'static str,
    mods: M,
    action: Action,
    [in_text_field, with_text_selection, during_drag]: [bool; 3],
) -> Row {
    Row {
        key,
        code,
        mods,
        action,
        in_text_field,
        with_text_selection,
        during_drag,
    }
}

/// Fires outside text fields, with or without a selection, not mid-drag.
const SINGLE: [bool; 3] = [false, true, false];
/// Like `SINGLE`, but also mid-drag (it is how a drag is cancelled).
const EDITOR: [bool; 3] = [false, true, true];
const ANYWHERE: [bool; 3] = [true, true, true];
/// `⌘C`: never in a text field or over a page text selection.
const COPY: [bool; 3] = [false, false, true];

/// The spec §Keyboard shortcuts table. `⌘` and `Ctrl` rows each stand for
/// both, checked separately below.
const SPEC: &[Row] = &[
    row("v", "KeyV", M::None, Action::Tool(Tool::Select), SINGLE),
    row("r", "KeyR", M::None, Action::Tool(Tool::Rect), SINGLE),
    row("l", "KeyL", M::None, Action::Tool(Tool::Lasso), SINGLE),
    row("t", "KeyT", M::None, Action::Op(OpKind::Translate), SINGLE),
    row("s", "KeyS", M::None, Action::Op(OpKind::Scale), SINGLE),
    row("m", "KeyM", M::None, Action::Op(OpKind::Mirror), SINGLE),
    row("o", "KeyO", M::None, Action::Op(OpKind::Rotate), SINGLE),
    row("h", "KeyH", M::None, Action::Op(OpKind::Hint), SINGLE),
    row("n", "KeyN", M::None, Action::Name, SINGLE),
    row("Escape", "Escape", M::None, Action::Cancel, EDITOR),
    row("Delete", "Delete", M::None, Action::DeleteSelected, SINGLE),
    row(
        "Backspace",
        "Backspace",
        M::None,
        Action::DeleteSelected,
        SINGLE,
    ),
    row("z", "KeyZ", M::Cmd, Action::Undo, EDITOR),
    row("z", "KeyZ", M::Ctrl, Action::Undo, EDITOR),
    row("z", "KeyZ", M::CmdShift, Action::Redo, EDITOR),
    row("Z", "KeyZ", M::CtrlShift, Action::Redo, EDITOR),
    row("y", "KeyY", M::Ctrl, Action::Redo, EDITOR),
    row("v", "KeyV", M::Cmd, Action::PasteImage, ANYWHERE),
    row("v", "KeyV", M::Ctrl, Action::PasteImage, ANYWHERE),
    row("o", "KeyO", M::Cmd, Action::OpenFile, ANYWHERE),
    row("o", "KeyO", M::Ctrl, Action::OpenFile, ANYWHERE),
    row("s", "KeyS", M::Cmd, Action::Download, ANYWHERE),
    row("s", "KeyS", M::Ctrl, Action::Download, ANYWHERE),
    row("c", "KeyC", M::Cmd, Action::CopyImage, COPY),
    row("c", "KeyC", M::Ctrl, Action::CopyImage, COPY),
    row("c", "KeyC", M::None, Action::CopyCaption, SINGLE),
    row("?", "Slash", M::Shift, Action::ShowSheet, SINGLE),
];

#[test]
fn every_spec_shortcut_fires_in_exactly_its_contexts() {
    for r in SPEC {
        for ctx in contexts() {
            let allowed = (r.in_text_field || !ctx.text_field)
                && (r.with_text_selection || !ctx.text_selected)
                && (r.during_drag || !ctx.dragging);
            let expected = allowed.then_some(r.action);
            assert_eq!(
                action(ev(r.key, r.code, r.mods), ctx),
                expected,
                "{} {:?} in {ctx:?}",
                r.key,
                r.mods
            );
        }
    }
}

#[test]
fn the_spec_table_covers_every_binding() {
    // Each binding is reached by some spec row, and each spec row by
    // exactly one binding: the keymap is the spec, no more and no less.
    for (i, b) in BINDINGS.iter().enumerate() {
        assert!(
            SPEC.iter()
                .any(|r| b.chord.matches(&ev(r.key, r.code, r.mods)) && r.action == b.action),
            "binding {i} ({b:?}) is not in the spec table"
        );
    }
}

#[test]
fn no_two_bindings_share_a_chord() {
    for (i, a) in BINDINGS.iter().enumerate() {
        for b in &BINDINGS[i + 1..] {
            assert_ne!(a.chord, b.chord, "duplicate chord");
        }
    }
    // ...and no event matches two bindings, whatever the modifiers.
    let keys = [
        ("v", "KeyV"),
        ("V", "KeyV"),
        ("c", "KeyC"),
        ("z", "KeyZ"),
        ("Z", "KeyZ"),
        ("y", "KeyY"),
        ("o", "KeyO"),
        ("s", "KeyS"),
        ("n", "KeyN"),
        ("щ", "KeyO"),
        ("?", "Slash"),
        ("Escape", "Escape"),
        ("Delete", "Delete"),
        ("Backspace", "Backspace"),
    ];
    for bits in 0..16u8 {
        for (key, code) in keys {
            let input = KeyInput {
                key,
                code,
                ctrl: bits & 1 != 0,
                meta: bits & 2 != 0,
                shift: bits & 4 != 0,
                alt: bits & 8 != 0,
                alt_graph: false,
            };
            let n = BINDINGS.iter().filter(|b| b.chord.matches(&input)).count();
            assert!(n <= 1, "{input:?} matches {n} bindings");
        }
    }
}

#[test]
fn single_key_shortcuts_never_fire_while_typing() {
    // The hint field and the instructions box are text fields.
    let typing = Context {
        text_field: true,
        ..Context::default()
    };
    for key in "vrltsmohncVRLTSMOHNC?".chars() {
        let key = key.to_string();
        let code = format!("Key{}", key.to_ascii_uppercase());
        for m in [M::None, M::Shift] {
            assert_eq!(action(ev(&key, &code, m), typing), None, "{key}");
        }
    }
    for key in ["Escape", "Delete", "Backspace"] {
        assert_eq!(action(ev(key, key, M::None), typing), None, "{key}");
    }
    // Native text undo in fields; ⌘C copies the field's text.
    assert_eq!(action(ev("z", "KeyZ", M::Cmd), typing), None);
    assert_eq!(action(ev("y", "KeyY", M::Ctrl), typing), None);
    assert_eq!(action(ev("c", "KeyC", M::Ctrl), typing), None);
}

#[test]
fn reserved_and_unbound_chords_are_left_to_the_browser() {
    let ctx = Context::default();
    let alt = |key, code, m| KeyInput {
        alt: true,
        ..ev(key, code, m)
    };
    // ⌘⇧C / Ctrl+Shift+C: inspector. ⌥⌘C / Ctrl+Alt+C: DevTools / AltGr.
    assert_eq!(action(ev("c", "KeyC", M::CmdShift), ctx), None);
    assert_eq!(action(ev("C", "KeyC", M::CtrlShift), ctx), None);
    assert_eq!(action(alt("c", "KeyC", M::Cmd), ctx), None);
    assert_eq!(action(alt("ç", "KeyC", M::Cmd), ctx), None);
    assert_eq!(action(alt("c", "KeyC", M::Ctrl), ctx), None);
    // ⌘⇧T reopens a tab; ⌘W / ⌘T / ⌘N are never ours.
    assert_eq!(action(ev("t", "KeyT", M::CmdShift), ctx), None);
    for (key, code) in [("w", "KeyW"), ("t", "KeyT"), ("n", "KeyN")] {
        assert_eq!(action(ev(key, code, M::Cmd), ctx), None, "{key}");
        assert_eq!(action(ev(key, code, M::Ctrl), ctx), None, "{key}");
    }
    // ⌘Y is history in macOS browsers; Ctrl+Shift+Y and ⌘⇧S are not ours.
    assert_eq!(action(ev("y", "KeyY", M::Cmd), ctx), None);
    assert_eq!(action(ev("Y", "KeyY", M::CtrlShift), ctx), None);
    assert_eq!(action(ev("s", "KeyS", M::CmdShift), ctx), None);
    // Shift+letter and Alt+letter are not the bare letter.
    assert_eq!(action(ev("C", "KeyC", M::Shift), ctx), None);
    assert_eq!(action(alt("o", "KeyO", M::None), ctx), None);
    // Caps Lock (uppercase key, no Shift) still is.
    assert_eq!(
        action(ev("C", "KeyC", M::None), ctx),
        Some(Action::CopyCaption)
    );
    assert_eq!(
        action(ev("O", "KeyO", M::Ctrl), ctx),
        Some(Action::OpenFile)
    );
    for key in ["x", "Enter", "", "cc", "Control", "Dead", "1"] {
        assert_eq!(action(ev(key, "KeyX", M::None), ctx), None, "{key}");
        assert_eq!(action(ev(key, "KeyX", M::Cmd), ctx), None, "{key}");
    }
}

/// One layout case: an event and the action it must trigger (`None`: the
/// key is left to the browser).
type LayoutCase = (&'static str, &'static str, M, Option<Action>, &'static str);

/// Non-Latin layouts (#22): the physical key decides.
const NON_LATIN: &[LayoutCase] = &[
    // Russian: the O key types "щ", Z "я", S "ы", C "с", Y "н", V "м".
    (
        "щ",
        "KeyO",
        M::Ctrl,
        Some(Action::OpenFile),
        "Russian Ctrl+O",
    ),
    ("щ", "KeyO", M::Cmd, Some(Action::OpenFile), "Russian ⌘O"),
    (
        "щ",
        "KeyO",
        M::None,
        Some(Action::Op(OpKind::Rotate)),
        "Russian O",
    ),
    ("я", "KeyZ", M::Ctrl, Some(Action::Undo), "Russian Ctrl+Z"),
    (
        "Я",
        "KeyZ",
        M::CtrlShift,
        Some(Action::Redo),
        "Russian Ctrl+Shift+Z",
    ),
    ("н", "KeyY", M::Ctrl, Some(Action::Redo), "Russian Ctrl+Y"),
    ("ы", "KeyS", M::Cmd, Some(Action::Download), "Russian ⌘S"),
    (
        "с",
        "KeyC",
        M::Ctrl,
        Some(Action::CopyImage),
        "Russian Ctrl+C",
    ),
    (
        "м",
        "KeyV",
        M::None,
        Some(Action::Tool(Tool::Select)),
        "Russian V",
    ),
    // Greek and Hebrew.
    ("ο", "KeyO", M::Ctrl, Some(Action::OpenFile), "Greek Ctrl+O"),
    (
        "ρ",
        "KeyR",
        M::None,
        Some(Action::Tool(Tool::Rect)),
        "Greek R",
    ),
    (
        "ם",
        "KeyO",
        M::Ctrl,
        Some(Action::OpenFile),
        "Hebrew Ctrl+O",
    ),
    (
        "ר",
        "KeyR",
        M::None,
        Some(Action::Tool(Tool::Rect)),
        "Hebrew R",
    ),
    // Arabic, Thai, Georgian, Korean 2-set, Japanese kana: the O key.
    (
        "خ",
        "KeyO",
        M::Ctrl,
        Some(Action::OpenFile),
        "Arabic Ctrl+O",
    ),
    ("น", "KeyO", M::Ctrl, Some(Action::OpenFile), "Thai Ctrl+O"),
    (
        "ო",
        "KeyO",
        M::Ctrl,
        Some(Action::OpenFile),
        "Georgian Ctrl+O",
    ),
    (
        "ㅐ",
        "KeyO",
        M::Ctrl,
        Some(Action::OpenFile),
        "Korean Ctrl+O",
    ),
    (
        "ら",
        "KeyO",
        M::Ctrl,
        Some(Action::OpenFile),
        "Japanese kana Ctrl+O",
    ),
    // Scripts added in #57, one row each (Ctrl+O unless the row says otherwise).
    (
        "ោ",
        "KeyO",
        M::Ctrl,
        Some(Action::OpenFile),
        "Khmer Ctrl+O (vowel sign)",
    ),
    (
        "រ",
        "KeyR",
        M::None,
        Some(Action::Tool(Tool::Rect)),
        "Khmer R",
    ),
    (
        "މ",
        "KeyM",
        M::None,
        Some(Action::Op(OpKind::Mirror)),
        "Thaana M",
    ),
    ("ཟ", "KeyZ", M::Ctrl, Some(Action::Undo), "Tibetan Ctrl+Z"),
    (
        "င",
        "KeyO",
        M::Ctrl,
        Some(Action::OpenFile),
        "Myanmar Ctrl+O",
    ),
    ("ܣ", "KeyS", M::Cmd, Some(Action::Download), "Syriac ⌘S"),
    (
        "ݐ",
        "KeyO",
        M::Ctrl,
        Some(Action::OpenFile),
        "Arabic Supplement Ctrl+O",
    ),
    ("ߞ", "KeyO", M::Ctrl, Some(Action::OpenFile), "NKo Ctrl+O"),
    (
        "ࢠ",
        "KeyO",
        M::Ctrl,
        Some(Action::OpenFile),
        "Arabic Extended-A Ctrl+O",
    ),
    (
        "ሀ",
        "KeyH",
        M::None,
        Some(Action::Op(OpKind::Hint)),
        "Ethiopic H",
    ),
    (
        "ᎀ",
        "KeyO",
        M::Ctrl,
        Some(Action::OpenFile),
        "Ethiopic Supplement Ctrl+O",
    ),
    (
        "Ꮎ",
        "KeyO",
        M::Ctrl,
        Some(Action::OpenFile),
        "Cherokee Ctrl+O",
    ),
    (
        "ꮎ",
        "KeyO",
        M::Ctrl,
        Some(Action::OpenFile),
        "Cherokee Supplement (lowercase) Ctrl+O",
    ),
    (
        "ᐅ",
        "KeyO",
        M::Ctrl,
        Some(Action::OpenFile),
        "Canadian Syllabics (Inuktitut) Ctrl+O",
    ),
    (
        "ᠣ",
        "KeyO",
        M::Ctrl,
        Some(Action::OpenFile),
        "Mongolian Ctrl+O",
    ),
    (
        "ᥐ",
        "KeyO",
        M::Ctrl,
        Some(Action::OpenFile),
        "Tai Le Ctrl+O",
    ),
    (
        "ᦀ",
        "KeyO",
        M::Ctrl,
        Some(Action::OpenFile),
        "New Tai Lue Ctrl+O",
    ),
    (
        "ᱚ",
        "KeyO",
        M::Ctrl,
        Some(Action::OpenFile),
        "Ol Chiki Ctrl+O",
    ),
    (
        "\u{1C96}",
        "KeyZ",
        M::CtrlShift,
        Some(Action::Redo),
        "Georgian Mtavruli Ctrl+Shift+Z",
    ),
    (
        "ⵓ",
        "KeyO",
        M::Ctrl,
        Some(Action::OpenFile),
        "Tifinagh Ctrl+O",
    ),
    (
        "ㄟ",
        "KeyO",
        M::Ctrl,
        Some(Action::OpenFile),
        "Bopomofo Ctrl+O",
    ),
    ("ꓳ", "KeyO", M::Ctrl, Some(Action::OpenFile), "Lisu Ctrl+O"),
    (
        "ꦎ",
        "KeyO",
        M::Ctrl,
        Some(Action::OpenFile),
        "Javanese Ctrl+O",
    ),
    (
        "\u{104C3}",
        "KeyO",
        M::Ctrl,
        Some(Action::OpenFile),
        "Osage Ctrl+O",
    ),
    (
        "\u{1E90C}",
        "KeyO",
        M::Ctrl,
        Some(Action::OpenFile),
        "Adlam Ctrl+O",
    ),
    // A key the browser can't name falls back to its position too.
    (
        "Unidentified",
        "KeyO",
        M::Ctrl,
        Some(Action::OpenFile),
        "Unidentified",
    ),
    // A non-letter position never becomes a letter.
    ("щ", "Digit1", M::None, None, "Cyrillic on a digit key"),
    ("ж", "Semicolon", M::None, None, "Cyrillic on Semicolon"),
    // Punctuation inside a non-Latin block is not a letter.
    (
        "\u{037E}",
        "KeyQ",
        M::None,
        None,
        "Greek question mark U+037E",
    ),
    ("،", "KeyO", M::Ctrl, None, "Arabic comma"),
    ("٣", "KeyO", M::Ctrl, None, "Arabic-Indic digit"),
    // Punctuation, digits and signs in the #57 blocks are not letters.
    ("༄", "KeyO", M::Ctrl, None, "Tibetan head mark"),
    ("၃", "KeyO", M::Ctrl, None, "Myanmar digit"),
    ("៛", "KeyO", M::Ctrl, None, "Khmer riel sign"),
    ("፡", "KeyO", M::Ctrl, None, "Ethiopic wordspace"),
    ("᠂", "KeyO", M::Ctrl, None, "Mongolian comma"),
];

/// Latin layouts: the letter printed on the key decides, and Latin letters
/// outside ASCII or symbols never fall back to the position (#53).
const LATIN: &[LayoutCase] = &[
    // AZERTY: M sits where QWERTY has `;`; QWERTY's M position types `,`.
    (
        "m",
        "Semicolon",
        M::None,
        Some(Action::Op(OpKind::Mirror)),
        "AZERTY M",
    ),
    (",", "KeyM", M::None, None, "AZERTY , on KeyM"),
    // AZERTY: Z is QWERTY's W position.
    ("z", "KeyW", M::Ctrl, Some(Action::Undo), "AZERTY Ctrl+Z"),
    ("w", "KeyZ", M::Ctrl, None, "AZERTY Ctrl+W on KeyZ"),
    // Dvorak: O is QWERTY's S position, S is QWERTY's semicolon.
    ("o", "KeyS", M::Cmd, Some(Action::OpenFile), "Dvorak ⌘O"),
    (
        "s",
        "Semicolon",
        M::Cmd,
        Some(Action::Download),
        "Dvorak ⌘S",
    ),
    // Turkish F: the R position types dotless ı (the layout's R is
    // elsewhere).
    ("ı", "KeyR", M::None, None, "Turkish F ı on KeyR"),
    ("ğ", "KeyO", M::Ctrl, None, "Turkish F ğ"),
    // BÉPO: è on QWERTY's T position, à on Z.
    ("è", "KeyT", M::None, None, "BÉPO è on KeyT"),
    ("à", "KeyZ", M::Ctrl, None, "BÉPO Ctrl+à on KeyZ"),
    // Other Latin-extended letters.
    ("ç", "KeyC", M::Ctrl, None, "ç"),
    ("ẽ", "KeyE", M::None, None, "Latin Extended Additional"),
    // Linux AltGr (German and similar), which Chrome reports with no altKey.
    ("µ", "KeyM", M::None, None, "AltGr µ on KeyM"),
    ("ł", "KeyL", M::None, None, "AltGr ł on KeyL"),
    ("ø", "KeyO", M::None, None, "AltGr ø on KeyO"),
    ("ſ", "KeyS", M::None, None, "AltGr ſ on KeyS"),
    ("¢", "KeyC", M::None, None, "AltGr ¢ on KeyC"),
    ("“", "KeyV", M::None, None, "AltGr “ on KeyV"),
    ("¶", "KeyR", M::None, None, "AltGr ¶ on KeyR"),
    ("€", "KeyE", M::None, None, "AltGr € on KeyE"),
    // Latin-extended letters outside Latin-1 (#57): every Latin block stays
    // out of the allowlist, including IPA letters that African layouts type.
    ("ƒ", "KeyO", M::Ctrl, None, "Latin Extended-B ƒ"),
    ("ɔ", "KeyO", M::Ctrl, None, "IPA ɔ (Ewe, Akan)"),
    ("ʋ", "KeyV", M::None, None, "IPA ʋ (Ewe)"),
    ("ʰ", "KeyH", M::None, None, "Spacing Modifier ʰ"),
    ("ⱥ", "KeyO", M::Ctrl, None, "Latin Extended-C"),
    ("ꝋ", "KeyO", M::Ctrl, None, "Latin Extended-D"),
    ("ꬰ", "KeyO", M::Ctrl, None, "Latin Extended-E"),
    ("\u{10781}", "KeyM", M::None, None, "Latin Extended-F"),
    ("\u{1DF00}", "KeyM", M::None, None, "Latin Extended-G"),
    ("ﬀ", "KeyO", M::Ctrl, None, "Latin ligature ﬀ"),
    ("ᴍ", "KeyM", M::None, None, "small capital ᴍ"),
    ("ᵐ", "KeyM", M::None, None, "modifier letter ᵐ"),
    ("ⁿ", "KeyO", M::Ctrl, None, "superscript ⁿ"),
    // Letterlike, enclosed and styled letters are alphabetic in Unicode but
    // are symbols on a keyboard: none falls back (#57).
    ("ℓ", "KeyL", M::None, None, "letterlike ℓ"),
    ("\u{212A}", "KeyC", M::Ctrl, None, "Kelvin sign"),
    ("ⓜ", "KeyM", M::None, None, "circled ⓜ"),
    ("⒨", "KeyM", M::None, None, "parenthesized ⒨"),
    ("🅜", "KeyM", M::None, None, "negative circled 🅜"),
    ("Ⅿ", "KeyM", M::None, None, "Roman numeral Ⅿ"),
    ("Ｍ", "KeyM", M::None, None, "fullwidth Ｍ"),
    ("𝐦", "KeyM", M::None, None, "mathematical bold 𝐦"),
    ("™", "KeyT", M::None, None, "™"),
    // `?` is matched on the character, wherever the layout puts it.
    ("?", "Comma", M::Shift, Some(Action::ShowSheet), "AZERTY ?"),
    (
        "?",
        "Minus",
        M::None,
        Some(Action::ShowSheet),
        "unshifted ?",
    ),
    ("/", "Slash", M::None, None, "/ is not ?"),
];

#[test]
fn letters_match_on_the_physical_key_for_non_latin_layouts() {
    for &(key, code, m, expected, note) in NON_LATIN {
        assert_eq!(
            action(ev(key, code, m), Context::default()),
            expected,
            "{note}"
        );
    }
}

#[test]
fn latin_layouts_keep_the_letter_printed_on_the_key() {
    for &(key, code, m, expected, note) in LATIN {
        assert_eq!(
            action(ev(key, code, m), Context::default()),
            expected,
            "{note}"
        );
    }
}

#[test]
fn only_non_latin_letters_fall_back_to_the_physical_key() {
    // (key, code, letter): the rule itself, below the bindings.
    let cases = [
        ("щ", "KeyO", Some('o')),
        ("Ω", "KeyV", Some('v')),
        ("Unidentified", "KeyQ", Some('q')),
        ("M", "KeyM", Some('m')),
        ("m", "Semicolon", Some('m')),
        ("ı", "KeyR", None),
        ("è", "KeyT", None),
        ("µ", "KeyM", None),
        ("ß", "KeyS", None),
        ("ª", "KeyA", None),
        ("щ", "Digit1", None),
        ("щщ", "KeyO", None),
        ("", "KeyO", None),
    ];
    for (key, code, letter) in cases {
        assert_eq!(event_letter(key, code), letter, "{key} {code}");
    }
    for c in [
        'щ',
        'Ж',
        'ο',
        'Ω',
        'ם',
        'خ',
        'น',
        'ო',
        'ㅐ',
        '한',
        'ら',
        'カ',
        '中',
        'ա',
        // #57
        'ក',
        'ោ',
        'ރ',
        'ཀ',
        'က',
        'ܐ',
        'ݐ',
        'ߊ',
        'ࢠ',
        'ሀ',
        'ᎀ',
        'Ꭰ',
        'ꭰ',
        'ᐁ',
        'ᠠ',
        'ᥐ',
        'ᦀ',
        'ᱚ',
        '\u{1C90}',
        'ⴰ',
        'ㄅ',
        'ꓐ',
        'ꦄ',
        '\u{104B0}',
        '\u{1E900}',
    ] {
        assert!(non_latin_letter(c), "{c}");
    }
    for c in [
        'a',
        'Z',
        'ı',
        'è',
        'à',
        'ç',
        'ğ',
        'ł',
        'ø',
        'ſ',
        'ß',
        'µ',
        'ẽ',
        '¢',
        '¶',
        '“',
        '\u{037E}',
        '،',
        '٣',
        '1',
        // #57: Latin-extended letters, letterlike and enclosed characters,
        // and the non-letters of the new blocks.
        'ƒ',
        'ɔ',
        'ʋ',
        'ʰ',
        'ⱥ',
        'ꝋ',
        'ꬰ',
        '\u{10781}',
        '\u{1DF00}',
        'ﬀ',
        'ᴍ',
        'ᵐ',
        'ⁿ',
        'ℓ',
        '\u{212A}',
        'ⓜ',
        '🅜',
        'Ⅿ',
        'Ｍ',
        '𝐦',
        '༄',
        '၃',
        '៛',
        '፡',
        '᠂',
    ] {
        assert!(!non_latin_letter(c), "{c}");
    }
    // These are alphabetic in Unicode, so `is_alphabetic` alone would let
    // them through: only the block allowlist keeps them out (#57).
    for c in [
        'ƒ',
        'ɔ',
        'ʰ',
        'ⱥ',
        'ꬰ',
        '\u{10781}',
        'ﬀ',
        'ᴍ',
        'ᵐ',
        'ⁿ',
        'ℓ',
        '\u{212A}',
        'ⓜ',
        '🅜',
        'Ⅿ',
        'Ｍ',
        '𝐦',
    ] {
        assert!(c.is_alphabetic() && !non_latin_letter(c), "{c}");
    }
}

#[test]
fn altgr_never_matches() {
    // Linux AltGr reports `AltGraph` but no `altKey`; Windows AltGr also
    // sets Ctrl and Alt.
    let ctx = Context::default();
    let altgr = |key, code, m| KeyInput {
        alt_graph: true,
        ..ev(key, code, m)
    };
    for (key, code, m) in [
        ("µ", "KeyM", M::None),
        ("m", "KeyM", M::None),
        ("щ", "KeyO", M::Ctrl),
        ("o", "KeyO", M::Ctrl),
        ("z", "KeyZ", M::Ctrl),
        ("?", "Minus", M::None),
        ("Escape", "Escape", M::None),
    ] {
        assert_eq!(action(altgr(key, code, m), ctx), None, "{key} {m:?}");
    }
}

#[test]
fn a_held_question_mark_never_toggles_the_sheet() {
    use SheetStep::{Hide, Keep, Show};
    // (action, sheet open, repeat, step)
    let cases = [
        (Action::ShowSheet, false, false, Some(Show)),
        (Action::ShowSheet, true, false, Some(Hide)),
        // The first auto-repeat of the `?` that opened it (#53).
        (Action::ShowSheet, true, true, Some(Keep)),
        (Action::ShowSheet, false, true, Some(Keep)),
        (Action::Cancel, true, false, Some(Hide)),
        (Action::Cancel, true, true, Some(Hide)),
        // A closed sheet's Esc is the canvas's.
        (Action::Cancel, false, false, None),
        (Action::Cancel, false, true, None),
        (Action::Undo, true, false, None),
        (Action::Download, false, false, None),
    ];
    for (action, open, repeat, step) in cases {
        assert_eq!(
            sheet_step(action, open, repeat),
            step,
            "{action:?} open={open} repeat={repeat}"
        );
    }
    // Press and hold `?`: one show, then the repeats keep it open.
    let mut open = false;
    for repeat in [false, true, true, true] {
        match sheet_step(Action::ShowSheet, open, repeat) {
            Some(Show) => open = true,
            Some(Hide) => open = false,
            _ => {}
        }
        assert!(open, "repeat={repeat}");
    }
}

#[test]
fn the_open_sheet_only_takes_its_own_keys_and_chords() {
    let open = Context {
        sheet_open: true,
        ..Context::default()
    };
    assert_eq!(
        action(ev("Escape", "Escape", M::None), open),
        Some(Action::Cancel)
    );
    assert_eq!(
        action(ev("?", "Slash", M::Shift), open),
        Some(Action::ShowSheet)
    );
    for (key, code) in [("r", "KeyR"), ("c", "KeyC"), ("Delete", "Delete")] {
        assert_eq!(action(ev(key, code, M::None), open), None, "{key}");
    }
    assert_eq!(
        action(ev("s", "KeyS", M::Cmd), open),
        Some(Action::Download)
    );
    assert_eq!(action(ev("z", "KeyZ", M::Ctrl), open), Some(Action::Undo));
}

#[test]
fn the_sheet_lists_exactly_the_keymap() {
    for platform in [Platform::Apple, Platform::Other] {
        let sections = sheet(platform);
        let rows: Vec<_> = sections.iter().flat_map(|s| &s.rows).collect();
        // One row per action, every action once.
        let actions: Vec<Action> = rows.iter().map(|r| r.action).collect();
        let unique: HashSet<Action> = actions.iter().copied().collect();
        assert_eq!(unique.len(), actions.len(), "an action is listed twice");
        let bound: HashSet<Action> = BINDINGS.iter().map(|b| b.action).collect();
        assert_eq!(unique, bound);
        // Every binding's label once, in its action's row, in keymap order.
        for row in &rows {
            let labels: Vec<String> = BINDINGS
                .iter()
                .filter(|b| b.action == row.action)
                .map(|b| b.chord.label(platform))
                .collect();
            assert_eq!(row.keys, labels, "{:?}", row.action);
            assert_eq!(row.description, row.action.description());
            assert!(!row.description.is_empty());
        }
        let total: usize = rows.iter().map(|r| r.keys.len()).sum();
        assert_eq!(total, BINDINGS.len());
        // Sections are not split: each title appears once.
        let titles: HashSet<_> = sections.iter().map(|s| s.title).collect();
        assert_eq!(titles.len(), sections.len());
    }
}

#[test]
fn labels_follow_the_platform() {
    use Platform::{Apple, Other};
    let cases = [
        (Action::Undo, Apple, "⌘Z"),
        (Action::Undo, Other, "Ctrl+Z"),
        (Action::Redo, Apple, "⌘⇧Z / ⌃Y"),
        (Action::Redo, Other, "Ctrl+Shift+Z / Ctrl+Y"),
        (Action::OpenFile, Apple, "⌘O"),
        (Action::OpenFile, Other, "Ctrl+O"),
        (Action::Download, Other, "Ctrl+S"),
        (Action::CopyImage, Apple, "⌘C"),
        (Action::CopyCaption, Apple, "C"),
        (Action::Tool(Tool::Rect), Other, "R"),
        (Action::Op(OpKind::Rotate), Apple, "O"),
        (Action::Name, Apple, "N"),
        (Action::Name, Other, "N"),
        (Action::Cancel, Apple, "Esc"),
        (Action::DeleteSelected, Other, "Delete / Backspace"),
        (Action::DeleteSelected, Apple, "⌦ / ⌫"),
        (Action::ShowSheet, Other, "?"),
    ];
    for (action, platform, label) in cases {
        assert_eq!(hint(action, platform), label, "{action:?} {platform:?}");
    }
    assert_eq!(super::rules::aria(Action::OpenFile), "Meta+O Control+O");
    assert_eq!(
        super::rules::aria(Action::Redo),
        "Meta+Shift+Z Control+Shift+Z Control+Y"
    );
    assert_eq!(super::rules::aria(Action::Tool(Tool::Lasso)), "L");
    assert_eq!(super::rules::aria(Action::Name), "N");
}

#[test]
fn platform_detection() {
    for p in ["MacIntel", "iPhone", "iPad", "Macintosh"] {
        assert_eq!(Platform::from_navigator(p), Platform::Apple, "{p}");
    }
    for p in ["Win32", "Linux x86_64", "Linux armv8l", ""] {
        assert_eq!(Platform::from_navigator(p), Platform::Other, "{p}");
    }
}

// --- The controller side of the canvas and history actions. ---

fn loaded() -> Controller {
    let mut c = Controller::new();
    c.load(ImageSize::new(200, 100).unwrap());
    c
}

fn rect_at(c: &mut Controller, x: f64) -> EditId {
    c.select(None);
    c.set_tool(Tool::Rect);
    c.pointer_down(Pointer::at(x, 10.0));
    c.pointer_move(Pointer::at(x + 15.0, 25.0));
    c.pointer_up(Pointer::at(x + 30.0, 40.0));
    c.selected().expect("new rect is selected")
}

fn order(c: &Controller) -> Vec<EditId> {
    c.editor()
        .unwrap()
        .document()
        .edits()
        .iter()
        .map(|e| e.id())
        .collect()
}

#[test]
fn tool_and_op_keys_drive_the_controller() {
    let mut c = loaded();
    for tool in Tool::ALL {
        assert_eq!(apply(Action::Tool(tool), &mut c), Some(true));
        assert_eq!(c.tool(), tool);
    }
    for kind in OpKind::ALL {
        assert_eq!(apply(Action::Op(kind), &mut c), Some(true));
        assert_eq!(c.op_kind(), kind);
    }
    // M again toggles the axis.
    let axis = c.mirror_axis();
    apply(Action::Op(OpKind::Mirror), &mut c);
    apply(Action::Op(OpKind::Mirror), &mut c);
    assert_eq!(c.mirror_axis(), axis.toggled());
}

#[test]
fn canvas_keys_do_nothing_without_an_image() {
    let mut c = Controller::new();
    assert_eq!(apply(Action::Tool(Tool::Lasso), &mut c), Some(false));
    assert_eq!(c.tool(), Tool::default());
    assert_eq!(apply(Action::Op(OpKind::Rotate), &mut c), Some(false));
    assert_eq!(apply(Action::Cancel, &mut c), Some(false));
    assert_eq!(apply(Action::DeleteSelected, &mut c), Some(false));
    assert_eq!(apply(Action::Undo, &mut c), Some(false));
    assert_eq!(apply(Action::Redo, &mut c), Some(false));
}

#[test]
fn non_controller_actions_are_not_applied() {
    let mut c = loaded();
    for a in [
        Action::PasteImage,
        Action::OpenFile,
        Action::Download,
        Action::CopyImage,
        Action::CopyCaption,
        Action::ShowSheet,
    ] {
        assert_eq!(apply(a, &mut c), None, "{a:?}");
    }
}

#[test]
fn delete_undo_and_redo_report_whether_they_ran() {
    let mut c = loaded();
    assert_eq!(apply(Action::DeleteSelected, &mut c), Some(false));
    assert_eq!(apply(Action::Undo, &mut c), Some(false));
    let a = rect_at(&mut c, 10.0);
    assert_eq!(apply(Action::DeleteSelected, &mut c), Some(true));
    assert!(order(&c).is_empty());
    assert_eq!(apply(Action::Undo, &mut c), Some(true));
    assert_eq!(order(&c), [a]);
    assert_eq!(apply(Action::Redo, &mut c), Some(true));
    assert!(order(&c).is_empty());
    assert_eq!(apply(Action::Redo, &mut c), Some(false));
}

#[test]
fn escape_cancels_a_drag_then_a_reorder_then_deselects() {
    // A canvas drag: Esc drops the half-drawn region, keeps the selection
    // rule for the next press.
    let mut c = loaded();
    let a = rect_at(&mut c, 10.0);
    c.select(None);
    c.pointer_down(Pointer::at(100.0, 10.0));
    c.pointer_move(Pointer::at(140.0, 50.0));
    assert!(c.dragging());
    assert_eq!(apply(Action::Cancel, &mut c), Some(true));
    assert!(!c.dragging());
    assert_eq!(order(&c), [a]);

    // A history reorder: Esc restores the order and keeps the redo stack.
    let b = rect_at(&mut c, 60.0);
    let d = rect_at(&mut c, 110.0);
    apply(Action::Undo, &mut c); // d's add is now redoable
    assert_eq!(order(&c), [a, b]);
    c.select(Some(a));
    assert!(c.begin_reorder());
    c.reorder_to(a, 1).unwrap();
    assert_eq!(order(&c), [b, a]);
    assert!(c.reordering());
    assert_eq!(apply(Action::Cancel, &mut c), Some(true));
    assert!(!c.reordering());
    assert_eq!(order(&c), [a, b]);
    // The selection survived that Esc: only the next one deselects.
    assert_eq!(c.selected(), Some(a));
    assert_eq!(apply(Action::Cancel, &mut c), Some(true));
    assert_eq!(c.selected(), None);
    // Nothing left to cancel.
    assert_eq!(apply(Action::Cancel, &mut c), Some(false));
    // The rest of that drag is ignored, and redo still has d.
    c.reorder_to(a, 1).unwrap();
    c.end_reorder();
    assert_eq!(order(&c), [a, b]);
    assert!(apply(Action::Redo, &mut c).unwrap());
    assert_eq!(order(&c), [a, b, d]);
}

#[test]
fn n_names_the_selection_or_arms_a_label() {
    // Without an image, N does nothing and keeps its ordinary meaning.
    let mut c = Controller::new();
    assert_eq!(apply(Action::Name, &mut c), Some(false));

    // With a selection: the name field is asked for; the op is untouched.
    let mut c = loaded();
    let a = rect_at(&mut c, 10.0);
    let kind = c.op_kind();
    assert_eq!(apply(Action::Name, &mut c), Some(true));
    assert!(c.take_name_focus());
    assert!(!c.take_name_focus(), "the request is taken once");
    assert_eq!(c.op_kind(), kind);
    assert_eq!(c.selected(), Some(a));

    // With nothing selected: the next region is a label, named `label N`,
    // and its name field is asked for.
    c.select(None);
    assert_eq!(apply(Action::Name, &mut c), Some(true));
    assert_eq!(c.op_kind(), OpKind::Label);
    assert!(!c.take_name_focus());
    let b = rect_at(&mut c, 80.0);
    assert!(c.take_name_focus());
    let doc = c.editor().unwrap().document();
    assert_eq!(*doc.edit(b).unwrap().op(), paste2prompt_core::Op::Label);
    assert_eq!(doc.edit(b).unwrap().name(), Some("label 1"));
    // Typing replaces it; one undo step for the typing.
    for typed in ["n", "na", "nav"] {
        c.set_name(typed).unwrap();
    }
    assert_eq!(c.selected_edit().unwrap().name(), Some("nav"));
    assert!(c.undo());
    assert_eq!(c.selected_edit().unwrap().name(), Some("label 1"));
}
