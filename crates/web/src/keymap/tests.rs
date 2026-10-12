//! The keymap, driven natively: every binding in every context, no
//! duplicates, the sheet against the keymap, layout independence (#22) and
//! the controller side of each canvas and history action.

use std::collections::HashSet;

use paste2prompt_core::{EditId, ImageSize};

use super::rules::{
    Action, BINDINGS, Context, KeyInput, Platform, apply, event_letter, hint, lookup, sheet,
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
    for key in "vrltsmohcVRLTSMOHC?".chars() {
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

#[test]
fn letters_match_on_the_physical_key_for_non_latin_layouts() {
    let ctx = Context::default();
    // Russian layout: the O key types "щ", Z "я", S "ы", C "с", Y "н".
    assert_eq!(
        action(ev("щ", "KeyO", M::Ctrl), ctx),
        Some(Action::OpenFile)
    );
    assert_eq!(action(ev("щ", "KeyO", M::Cmd), ctx), Some(Action::OpenFile));
    assert_eq!(
        action(ev("щ", "KeyO", M::None), ctx),
        Some(Action::Op(OpKind::Rotate))
    );
    assert_eq!(action(ev("я", "KeyZ", M::Ctrl), ctx), Some(Action::Undo));
    assert_eq!(
        action(ev("Я", "KeyZ", M::CtrlShift), ctx),
        Some(Action::Redo)
    );
    assert_eq!(action(ev("н", "KeyY", M::Ctrl), ctx), Some(Action::Redo));
    assert_eq!(action(ev("ы", "KeyS", M::Cmd), ctx), Some(Action::Download));
    assert_eq!(
        action(ev("с", "KeyC", M::Ctrl), ctx),
        Some(Action::CopyImage)
    );
    assert_eq!(
        action(ev("м", "KeyV", M::None), ctx),
        Some(Action::Tool(Tool::Select))
    );
    // Greek and Hebrew.
    assert_eq!(
        action(ev("ο", "KeyO", M::Ctrl), ctx),
        Some(Action::OpenFile)
    );
    assert_eq!(
        action(ev("ם", "KeyO", M::Ctrl), ctx),
        Some(Action::OpenFile)
    );
    // A key the browser can't name falls back to its position too.
    assert_eq!(
        action(ev("Unidentified", "KeyO", M::Ctrl), ctx),
        Some(Action::OpenFile)
    );
    // A non-letter position never becomes a letter.
    assert_eq!(event_letter("щ", "Digit1"), None);
    assert_eq!(event_letter("ж", "Semicolon"), None);
}

#[test]
fn latin_layouts_keep_the_letter_printed_on_the_key() {
    let ctx = Context::default();
    // AZERTY: M sits where QWERTY has `;`, and QWERTY's M position types `,`.
    assert_eq!(
        action(ev("m", "Semicolon", M::None), ctx),
        Some(Action::Op(OpKind::Mirror))
    );
    assert_eq!(action(ev(",", "KeyM", M::None), ctx), None);
    // AZERTY: Z is QWERTY's W position, A is Q.
    assert_eq!(action(ev("z", "KeyW", M::Ctrl), ctx), Some(Action::Undo));
    assert_eq!(action(ev("w", "KeyZ", M::Ctrl), ctx), None);
    // Dvorak: O is QWERTY's S position, S is QWERTY's semicolon.
    assert_eq!(action(ev("o", "KeyS", M::Cmd), ctx), Some(Action::OpenFile));
    assert_eq!(
        action(ev("s", "Semicolon", M::Cmd), ctx),
        Some(Action::Download)
    );
    // `?` is matched on the character, wherever the layout puts it.
    assert_eq!(
        action(ev("?", "Comma", M::Shift), ctx),
        Some(Action::ShowSheet)
    );
    assert_eq!(
        action(ev("?", "Minus", M::None), ctx),
        Some(Action::ShowSheet)
    );
    assert_eq!(action(ev("/", "Slash", M::None), ctx), None);
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
