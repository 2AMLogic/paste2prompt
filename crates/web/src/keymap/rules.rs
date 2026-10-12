//! The keymap itself: every keyboard shortcut (spec §Keyboard shortcuts) as
//! data, how a key event matches one, the context it may fire in, and the
//! labels the shortcut sheet and the tooltips show. No browser types here, so
//! `cargo test` covers it natively.
//!
//! # Layout independence (#22)
//!
//! Letters are matched on the character the user **sees**, falling back to
//! the physical key only when that character is not Latin:
//!
//! - If `KeyboardEvent.key` is a single ASCII letter, that letter decides.
//!   On AZERTY, Dvorak or Colemak the user presses the key labelled `M` to
//!   mirror, wherever it sits, which is what the sheet and the tooltips
//!   show. Matching on `code` alone would bind AZERTY's `,` key (where
//!   QWERTY has `M`) and leave its labelled `M` key dead.
//! - If `key` is a single non-ASCII character (Cyrillic `щ`, Greek `ο`,
//!   Hebrew `ם`…) or `"Unidentified"`, the letter comes from
//!   `KeyboardEvent.code` (`KeyO` → `o`), so `Ctrl+O`, `⌘Z`, `O` and the
//!   rest still work, on the key that has the Latin letter printed on it on
//!   those keyboards.
//! - Any other ASCII character (AZERTY's `,` on `KeyM`) is not a letter, so
//!   it never falls back. Digits, punctuation and named keys never match a
//!   letter binding.
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
            Self::Op(_) => "Operations",
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

/// The modifiers a chord needs. Alt never matches: `⌥⌘C` opens DevTools on
/// macOS, and Ctrl+Alt is AltGr on many European layouts (spec).
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
        (Some(c), None) if !c.is_ascii() => code_letter(code),
        _ if key == "Unidentified" => code_letter(code),
        _ => None,
    }
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
        if ev.alt {
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
