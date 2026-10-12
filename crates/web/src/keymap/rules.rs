//! The keymap itself: every keyboard shortcut (spec §Keyboard shortcuts) as
//! data, how a key event matches one, the context it may fire in, and the
//! labels the shortcut sheet and the tooltips show. No browser types here, so
//! `cargo test` covers it natively.
//!
//! # Layout independence (#22)
//!
//! Letters are matched on the character the user **sees**, falling back to
//! the physical key only when that character is from a non-Latin script:
//!
//! - If `KeyboardEvent.key` is a single ASCII letter, that letter decides.
//!   On AZERTY, Dvorak or Colemak the user presses the key labelled `M` to
//!   mirror, wherever it sits, which is what the sheet and the tooltips
//!   show. Matching on `code` alone would bind AZERTY's `,` key (where
//!   QWERTY has `M`) and leave its labelled `M` key dead.
//! - If `key` is a single letter from a non-Latin script (Cyrillic `щ`,
//!   Greek `ο`, Hebrew `ם`, Arabic, Khmer, kana, Hangul…: an allowlist of
//!   scripts, see [`non_latin_letter`] and `NON_LATIN_BLOCKS` for why) or
//!   `"Unidentified"`, the letter comes from `KeyboardEvent.code`
//!   (`KeyO` → `o`), so `Ctrl+O`, `⌘Z`, `O` and the rest still work, on the
//!   key that has the Latin letter printed on it on those keyboards.
//! - Anything else never falls back (#53): a Latin letter outside ASCII
//!   (Turkish F's `ı` on `KeyR`, BÉPO's `è` on `KeyT`) is the letter on that
//!   key, not R or T; a symbol (Linux AltGr's `µ` on `KeyM`, `¶` on `KeyR`,
//!   AZERTY's `,` on `KeyM`) is not a letter at all, and neither is a
//!   letterlike or circled character (`ℓ`, `ⓜ`). Digits, punctuation and
//!   named keys never match a letter binding.
//! - An event with AltGr held (`getModifierState("AltGraph")`) matches
//!   nothing, like Alt: on Linux AltGr doesn't set `altKey`.
//!
//! `?` is matched on `key` only: it is a shifted character whose position
//! differs between layouts, and `key` is exactly what the user typed.

use crate::canvas::{Controller, OpKind, Tool};

/// What a shortcut does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Action {
    /// Pick a region tool.
    Tool(Tool),
    /// Pick an operation (`M` again toggles the mirror axis).
    Op(OpKind),
    /// `N`: focus the selected edit's name field, or, with nothing
    /// selected, make the next region drawn a label (a name only).
    Name,
    /// `Esc`: cancel a canvas drag, else a history reorder, else deselect.
    /// Closes the shortcut sheet when it is open.
    Cancel,
    /// Delete the selected edit.
    DeleteSelected,
    Undo,
    Redo,
    /// `⌘V`: handled by the window `paste` listener (`crate::input`). The
    /// keymap lists it for the sheet and never prevents its default.
    PasteImage,
    /// Open the file picker.
    OpenFile,
    /// Download the export PNG.
    Download,
    /// Copy the export PNG.
    CopyImage,
    /// Copy the caption as text.
    CopyCaption,
    /// Show or hide the shortcut sheet.
    ShowSheet,
}

impl Action {
    /// The sheet's description of the action.
    pub fn description(self) -> &'static str {
        match self {
            Self::Tool(Tool::Select) => "Select an edit; dragging moves its source region",
            Self::Tool(Tool::Rect) => "Rectangle region tool",
            Self::Tool(Tool::Lasso) => "Lasso region tool",
            Self::Op(OpKind::Translate) => "Operation: translate",
            Self::Op(OpKind::Scale) => "Operation: scale",
            Self::Op(OpKind::Mirror) => "Operation: mirror (press again to switch the axis)",
            Self::Op(OpKind::Rotate) => "Operation: rotate",
            Self::Op(OpKind::Hint) => "Operation: hint (focuses the hint field)",
            Self::Op(OpKind::Label) => "Operation: label (a name only)",
            Self::Name => {
                "Name the selected region, or label the next region drawn (focuses the name field)"
            }
            Self::Cancel => "Cancel the current drag or reorder, or deselect",
            Self::DeleteSelected => "Delete the selected edit",
            Self::Undo => "Undo",
            Self::Redo => "Redo",
            Self::PasteImage => "Paste an image",
            Self::OpenFile => "Open an image file",
            Self::Download => "Download PNG",
            Self::CopyImage => "Copy image (outside text fields, with no page text selected)",
            Self::CopyCaption => "Copy caption as text",
            Self::ShowSheet => "Show or hide this shortcut sheet",
        }
    }

    /// The sheet section the action is listed under.
    pub fn section(self) -> &'static str {
        match self {
            Self::Tool(_) => "Tools",
            Self::Op(_) | Self::Name => "Operations",
            Self::Cancel | Self::DeleteSelected | Self::Undo | Self::Redo => "Editing",
            Self::PasteImage | Self::OpenFile => "Image",
            Self::Download | Self::CopyImage | Self::CopyCaption => "Export",
            Self::ShowSheet => "Help",
        }
    }
}

/// The key part of a chord.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    /// A Latin letter (lowercase), matched layout-independently (module
    /// docs).
    Letter(char),
    /// A named key, matched on `KeyboardEvent.key` (`"Escape"`, `"Delete"`,
    /// `"Backspace"`).
    Named(&'static str),
    /// A printed character matched on `KeyboardEvent.key` exactly, with
    /// Shift ignored (it is how the character is typed): `?`.
    Char(char),
}

/// The modifiers a chord needs. Alt and AltGr never match: `⌥⌘C` opens
/// DevTools on macOS, and Ctrl+Alt is AltGr on many European layouts
/// (spec).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mods {
    /// No Ctrl, ⌘ or Alt (and no Shift, except for [`Key::Char`]).
    None,
    /// `⌘` or `Ctrl`, without Shift. Either works on every platform.
    Primary,
    /// `⌘⇧` or `Ctrl+Shift`.
    PrimaryShift,
    /// `Ctrl` only, never `⌘` (`Ctrl+Y`; `⌘Y` is history in macOS browsers).
    Ctrl,
}

/// A key and its modifiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chord {
    pub key: Key,
    pub mods: Mods,
}

/// Where a binding may fire. A context the binding doesn't allow leaves the
/// event to the browser.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct When {
    /// Fires while a text field (or `contenteditable`) has focus.
    pub in_text_field: bool,
    /// Fires while the page has a text selection.
    pub with_text_selection: bool,
    /// Fires during a canvas drag or a history-panel reorder.
    pub during_drag: bool,
}

/// Single keys: typing in a field is typing, and a drag isn't interrupted.
const SINGLE: When = When {
    in_text_field: false,
    with_text_selection: true,
    during_drag: false,
};
/// Esc, undo and redo: not in text fields (native text undo, and Esc stays
/// the field's), but they are how a drag gets cancelled.
const EDITOR: When = When {
    in_text_field: false,
    with_text_selection: true,
    during_drag: true,
};
/// Paste, open and download work everywhere.
const ANYWHERE: When = When {
    in_text_field: true,
    with_text_selection: true,
    during_drag: true,
};
/// `⌘C` yields to the browser's text copy.
const COPY_IMAGE: When = When {
    in_text_field: false,
    with_text_selection: false,
    during_drag: true,
};

/// One shortcut.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Binding {
    pub chord: Chord,
    pub action: Action,
    pub when: When,
}

const fn bind(key: Key, mods: Mods, action: Action, when: When) -> Binding {
    Binding {
        chord: Chord { key, mods },
        action,
        when,
    }
}

const fn letter(c: char, action: Action) -> Binding {
    bind(Key::Letter(c), Mods::None, action, SINGLE)
}

/// Every shortcut, in sheet order. The single source of truth: the window
/// listener dispatches through it, the sheet renders it and the tooltips
/// quote it.
pub const BINDINGS: &[Binding] = &[
    letter('v', Action::Tool(Tool::Select)),
    letter('r', Action::Tool(Tool::Rect)),
    letter('l', Action::Tool(Tool::Lasso)),
    letter('t', Action::Op(OpKind::Translate)),
    letter('s', Action::Op(OpKind::Scale)),
    letter('m', Action::Op(OpKind::Mirror)),
    letter('o', Action::Op(OpKind::Rotate)),
    letter('h', Action::Op(OpKind::Hint)),
    letter('n', Action::Name),
    bind(Key::Named("Escape"), Mods::None, Action::Cancel, EDITOR),
    bind(
        Key::Named("Delete"),
        Mods::None,
        Action::DeleteSelected,
        SINGLE,
    ),
    bind(
        Key::Named("Backspace"),
        Mods::None,
        Action::DeleteSelected,
        SINGLE,
    ),
    bind(Key::Letter('z'), Mods::Primary, Action::Undo, EDITOR),
    bind(Key::Letter('z'), Mods::PrimaryShift, Action::Redo, EDITOR),
    bind(Key::Letter('y'), Mods::Ctrl, Action::Redo, EDITOR),
    bind(
        Key::Letter('v'),
        Mods::Primary,
        Action::PasteImage,
        ANYWHERE,
    ),
    bind(Key::Letter('o'), Mods::Primary, Action::OpenFile, ANYWHERE),
    bind(Key::Letter('s'), Mods::Primary, Action::Download, ANYWHERE),
    bind(
        Key::Letter('c'),
        Mods::Primary,
        Action::CopyImage,
        COPY_IMAGE,
    ),
    letter('c', Action::CopyCaption),
    bind(Key::Char('?'), Mods::None, Action::ShowSheet, SINGLE),
];

/// A key event, reduced to what matching needs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KeyInput<'a> {
    /// `KeyboardEvent.key`.
    pub key: &'a str,
    /// `KeyboardEvent.code`.
    pub code: &'a str,
    pub ctrl: bool,
    pub meta: bool,
    pub shift: bool,
    pub alt: bool,
    /// `getModifierState("AltGraph")`.
    pub alt_graph: bool,
}

/// The state a key press happens in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Context {
    /// Focus is in a text field or `contenteditable` element.
    pub text_field: bool,
    /// The page has a non-empty text selection.
    pub text_selected: bool,
    /// A canvas drag or a history-panel reorder is in progress.
    pub dragging: bool,
    /// The shortcut sheet is open.
    pub sheet_open: bool,
}

/// The Latin letter a key event stands for (module docs), lowercase.
pub fn event_letter(key: &str, code: &str) -> Option<char> {
    let mut chars = key.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if c.is_ascii_alphabetic() => Some(c.to_ascii_lowercase()),
        (Some(c), None) if non_latin_letter(c) => code_letter(code),
        _ if key == "Unidentified" => code_letter(code),
        _ => None,
    }
}

/// The Unicode blocks of the non-Latin scripts that keyboard layouts type
/// (whose keys usually carry the Latin letter too): a key typing a letter
/// from one of them stands for its physical position. Ranges are whole
/// blocks; [`non_latin_letter`] keeps only their letters, so block
/// punctuation (Greek `;`, Arabic `،`, Tibetan `༄`), digits (Arabic-Indic
/// `٣`, Myanmar `၃`) and signs (Khmer `៛`) are excluded.
///
/// # Why an allowlist (#57)
///
/// The alternative, "alphabetic and not in a Latin block", would cover every
/// non-Latin script, but `char::is_alphabetic` is also true for many things
/// that are neither: letterlike symbols (`ℓ`, `K` U+212A), circled and
/// parenthesized letters (`ⓜ`), Roman numerals (`Ⅿ`), fullwidth and
/// mathematical Latin (`Ｍ`, `𝐦`), small capitals and superscripts (`ᴍ`,
/// `ᵐ`), and Latin blocks added in later Unicode versions (Latin
/// Extended-F/G). A denylist lets each one fall back to the physical key
/// unless someone remembered to list its block, and a slip fires the wrong
/// shortcut. With an allowlist, a missing block only leaves the shortcuts on
/// that layout doing nothing, which is safe. So this list names every
/// non-Latin script with a keyboard layout shipped by Windows, macOS or
/// Linux. Latin blocks are deliberately absent, as are symbol blocks.
const NON_LATIN_BLOCKS: &[(char, char)] = &[
    ('\u{0370}', '\u{03FF}'),   // Greek and Coptic
    ('\u{0400}', '\u{052F}'),   // Cyrillic, Cyrillic Supplement
    ('\u{0530}', '\u{058F}'),   // Armenian
    ('\u{0590}', '\u{05FF}'),   // Hebrew
    ('\u{0600}', '\u{06FF}'),   // Arabic (also Persian, Urdu)
    ('\u{0700}', '\u{074F}'),   // Syriac
    ('\u{0750}', '\u{077F}'),   // Arabic Supplement
    ('\u{0780}', '\u{07BF}'),   // Thaana (Dhivehi)
    ('\u{07C0}', '\u{07FF}'),   // NKo
    ('\u{08A0}', '\u{08FF}'),   // Arabic Extended-A
    ('\u{0900}', '\u{0DFF}'),   // Indic: Devanagari to Sinhala
    ('\u{0E00}', '\u{0EFF}'),   // Thai, Lao
    ('\u{0F00}', '\u{0FFF}'),   // Tibetan (also Dzongkha)
    ('\u{1000}', '\u{109F}'),   // Myanmar
    ('\u{10A0}', '\u{10FF}'),   // Georgian
    ('\u{1100}', '\u{11FF}'),   // Hangul Jamo
    ('\u{1200}', '\u{139F}'),   // Ethiopic, Ethiopic Supplement
    ('\u{13A0}', '\u{13FF}'),   // Cherokee
    ('\u{1400}', '\u{167F}'),   // Unified Canadian Aboriginal Syllabics
    ('\u{1780}', '\u{17FF}'),   // Khmer
    ('\u{1800}', '\u{18AF}'),   // Mongolian (traditional)
    ('\u{1950}', '\u{19DF}'),   // Tai Le, New Tai Lue
    ('\u{1C50}', '\u{1C7F}'),   // Ol Chiki (Santali)
    ('\u{1C90}', '\u{1CBF}'),   // Georgian Extended (Mtavruli)
    ('\u{1F00}', '\u{1FFF}'),   // Greek Extended
    ('\u{2D30}', '\u{2D7F}'),   // Tifinagh
    ('\u{3040}', '\u{30FF}'),   // Hiragana, Katakana
    ('\u{3100}', '\u{312F}'),   // Bopomofo (Zhuyin)
    ('\u{3130}', '\u{318F}'),   // Hangul Compatibility Jamo (2-set Korean)
    ('\u{4E00}', '\u{9FFF}'),   // CJK Unified Ideographs
    ('\u{A4D0}', '\u{A4FF}'),   // Lisu
    ('\u{A980}', '\u{A9DF}'),   // Javanese
    ('\u{AB70}', '\u{ABBF}'),   // Cherokee Supplement (lowercase)
    ('\u{AC00}', '\u{D7AF}'),   // Hangul Syllables
    ('\u{104B0}', '\u{104FF}'), // Osage
    ('\u{1E900}', '\u{1E95F}'), // Adlam
];

/// Whether `c` is a letter from a non-Latin script ([`NON_LATIN_BLOCKS`]),
/// so its key falls back to `KeyboardEvent.code` (module docs). Both tests
/// must pass: the block says which script, `is_alphabetic` drops the
/// block's punctuation, digits and symbols.
pub fn non_latin_letter(c: char) -> bool {
    c.is_alphabetic()
        && NON_LATIN_BLOCKS
            .iter()
            .any(|&(lo, hi)| (lo..=hi).contains(&c))
}

/// `"KeyO"` → `'o'`.
fn code_letter(code: &str) -> Option<char> {
    let rest = code.strip_prefix("Key")?;
    let mut chars = rest.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if c.is_ascii_uppercase() => Some(c.to_ascii_lowercase()),
        _ => None,
    }
}

impl Chord {
    /// Whether the event is this chord, ignoring context.
    pub fn matches(&self, ev: &KeyInput<'_>) -> bool {
        let key = match self.key {
            Key::Letter(c) => event_letter(ev.key, ev.code) == Some(c),
            Key::Named(name) => ev.key == name,
            Key::Char(c) => ev.key.chars().eq([c]),
        };
        key && self.mods_match(ev)
    }

    fn mods_match(&self, ev: &KeyInput<'_>) -> bool {
        if ev.alt || ev.alt_graph {
            return false;
        }
        let primary = ev.ctrl || ev.meta;
        match self.mods {
            Mods::None => !primary && (!ev.shift || matches!(self.key, Key::Char(_))),
            Mods::Primary => primary && !ev.shift,
            Mods::PrimaryShift => primary && ev.shift,
            Mods::Ctrl => ev.ctrl && !ev.meta && !ev.shift,
        }
    }

    /// A key with no modifier, which typing would produce.
    pub fn is_single_key(&self) -> bool {
        self.mods == Mods::None
    }

    /// The label the user sees, e.g. `⌘⇧Z` on macOS, `Ctrl+Shift+Z`
    /// elsewhere.
    pub fn label(&self, platform: Platform) -> String {
        let apple = platform == Platform::Apple;
        let prefix = match (self.mods, apple) {
            (Mods::None, _) => "",
            (Mods::Primary, true) => "⌘",
            (Mods::PrimaryShift, true) => "⌘⇧",
            (Mods::Ctrl, true) => "⌃",
            (Mods::Primary | Mods::Ctrl, false) => "Ctrl+",
            (Mods::PrimaryShift, false) => "Ctrl+Shift+",
        };
        let key = match self.key {
            Key::Letter(c) | Key::Char(c) => c.to_ascii_uppercase().to_string(),
            Key::Named("Escape") => "Esc".to_owned(),
            Key::Named("Backspace") if apple => "⌫".to_owned(),
            Key::Named("Delete") if apple => "⌦".to_owned(),
            Key::Named(name) => name.to_owned(),
        };
        format!("{prefix}{key}")
    }

    /// The `aria-keyshortcuts` value, e.g. `Meta+O Control+O`.
    pub fn aria(&self) -> String {
        let key = match self.key {
            Key::Letter(c) | Key::Char(c) => c.to_ascii_uppercase().to_string(),
            Key::Named(name) => name.to_owned(),
        };
        match self.mods {
            Mods::None => key,
            Mods::Primary => format!("Meta+{key} Control+{key}"),
            Mods::PrimaryShift => format!("Meta+Shift+{key} Control+Shift+{key}"),
            Mods::Ctrl => format!("Control+{key}"),
        }
    }
}

impl When {
    pub fn allows(&self, ctx: Context) -> bool {
        (self.in_text_field || !ctx.text_field)
            && (self.with_text_selection || !ctx.text_selected)
            && (self.during_drag || !ctx.dragging)
    }
}

/// The binding a key press triggers in `ctx`, if any. While the sheet is
/// open, the only single keys are `Esc` and `?` (both close it); chords
/// still work.
pub fn lookup(ev: &KeyInput<'_>, ctx: Context) -> Option<&'static Binding> {
    let binding = BINDINGS.iter().find(|b| b.chord.matches(ev))?;
    let sheet_blocks = ctx.sheet_open
        && binding.chord.is_single_key()
        && !matches!(binding.action, Action::Cancel | Action::ShowSheet);
    (!sheet_blocks && binding.when.allows(ctx)).then_some(binding)
}

/// What a key press does to the shortcut sheet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SheetStep {
    Show,
    Hide,
    /// Leave it as it is (but still prevent the key's default).
    Keep,
}

/// The sheet's response to `action`, or `None` if the sheet doesn't handle
/// it. `Esc` closes an open sheet; `?` toggles it. A held `?` (`repeat`)
/// never toggles: its first auto-repeat would close the sheet it just
/// opened (#53).
pub fn sheet_step(action: Action, sheet_open: bool, repeat: bool) -> Option<SheetStep> {
    match action {
        Action::Cancel if sheet_open => Some(SheetStep::Hide),
        Action::ShowSheet if repeat => Some(SheetStep::Keep),
        Action::ShowSheet if sheet_open => Some(SheetStep::Hide),
        Action::ShowSheet => Some(SheetStep::Show),
        _ => None,
    }
}

/// Which modifier labels to show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    /// macOS and iOS: `⌘`, `⇧`, `⌃`.
    Apple,
    /// Everything else: `Ctrl+`, `Shift+`.
    Other,
}

impl Platform {
    /// From `navigator.platform` (`MacIntel`, `iPhone`, `Win32`,
    /// `Linux x86_64`…).
    pub fn from_navigator(platform: &str) -> Self {
        let apple = ["Mac", "iPhone", "iPad", "iPod"]
            .iter()
            .any(|p| platform.starts_with(p));
        if apple { Self::Apple } else { Self::Other }
    }
}

/// The bindings of `action`, in keymap order.
pub fn bindings_for(action: Action) -> impl Iterator<Item = &'static Binding> {
    BINDINGS.iter().filter(move |b| b.action == action)
}

/// The tooltip text for `action`'s shortcuts, e.g. `⌘⇧Z / ⌃Y`. Empty if it
/// has none.
pub fn hint(action: Action, platform: Platform) -> String {
    bindings_for(action)
        .map(|b| b.chord.label(platform))
        .collect::<Vec<_>>()
        .join(" / ")
}

/// The `aria-keyshortcuts` value for `action`'s shortcuts.
pub fn aria(action: Action) -> String {
    bindings_for(action)
        .map(|b| b.chord.aria())
        .collect::<Vec<_>>()
        .join(" ")
}

/// One row of the sheet: an action and every chord bound to it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SheetRow {
    pub action: Action,
    pub keys: Vec<String>,
    pub description: &'static str,
}

/// One titled group of rows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SheetSection {
    pub title: &'static str,
    pub rows: Vec<SheetRow>,
}

/// The shortcut sheet, generated from [`BINDINGS`]: sections and rows in
/// keymap order, one row per action.
pub fn sheet(platform: Platform) -> Vec<SheetSection> {
    let mut sections: Vec<SheetSection> = Vec::new();
    for b in BINDINGS {
        let title = b.action.section();
        let index = match sections.iter().position(|s| s.title == title) {
            Some(i) => i,
            None => {
                sections.push(SheetSection {
                    title,
                    rows: Vec::new(),
                });
                sections.len() - 1
            }
        };
        let rows = &mut sections[index].rows;
        let label = b.chord.label(platform);
        match rows.iter_mut().find(|r| r.action == b.action) {
            Some(row) => row.keys.push(label),
            None => rows.push(SheetRow {
                action: b.action,
                keys: vec![label],
                description: b.action.description(),
            }),
        }
    }
    sections
}

/// Runs a canvas or history action on the controller. `Some(true)` if it
/// did something (so the key's default is prevented), `Some(false)` if not,
/// `None` for actions the controller doesn't own (export, file, sheet).
///
/// Canvas actions need an image: with none loaded they do nothing, and the
/// key keeps its ordinary meaning.
pub fn apply(action: Action, c: &mut Controller) -> Option<bool> {
    let loaded = c.editor().is_some();
    let done = match action {
        Action::Tool(tool) => {
            if loaded {
                c.set_tool(tool);
            }
            loaded
        }
        Action::Op(kind) => loaded && c.choose_op(kind).is_ok(),
        Action::Name => c.name_key(),
        Action::Cancel => escape(c),
        Action::DeleteSelected => c.delete_selected(),
        Action::Undo => c.undo(),
        Action::Redo => c.redo(),
        Action::PasteImage
        | Action::OpenFile
        | Action::Download
        | Action::CopyImage
        | Action::CopyCaption
        | Action::ShowSheet => return None,
    };
    Some(done)
}

/// `Esc`: cancel a canvas drag (reverting it), else a history-panel
/// reorder (restoring the order, keeping the redo stack), else deselect.
fn escape(c: &mut Controller) -> bool {
    if c.dragging() {
        return c.cancel();
    }
    if c.reordering() {
        c.cancel_reorder();
        return true;
    }
    c.cancel()
}
