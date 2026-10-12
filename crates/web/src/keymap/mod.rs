//! Keyboard shortcuts (spec §Keyboard shortcuts) and the `?` shortcut sheet.
//!
//! [`rules::BINDINGS`] is the single source of truth. One window `keydown`
//! listener ([`ShortcutLayer`]) looks each event up there and dispatches it;
//! the sheet is generated from it ([`rules::sheet`]), and so are the
//! toolbar tooltips ([`hint`]). No other module listens for `keydown`.
//!
//! Dispatch rules, kept from the per-module listeners this replaced (#9's
//! `⌘O`, #11's undo/redo, #12's export keys):
//!
//! - Nothing fires during IME composition.
//! - The binding's [`rules::When`] decides the context: single keys never
//!   fire while a text field has focus or a drag is in progress; undo/redo
//!   leave text fields to their native undo; `⌘C` yields to a page text
//!   selection; `⌘O`, `⌘S` and `⌘V` work everywhere.
//! - `preventDefault` only when the action ran, with two exceptions that
//!   always suppress the browser's own dialog: `⌘S` (Save page) and `⌘O`
//!   (Open file). `⌘V` is never prevented: the `paste` event does the work
//!   (`crate::input`).
//! - A held key (`repeat`) doesn't re-run export, open or the sheet.
//!
//! Layout independence (#22) is in [`rules`].

pub mod rules;
#[cfg(test)]
mod tests;

use leptos::{ev, html, prelude::*};
use wasm_bindgen::JsCast;
use web_sys::{HtmlElement, KeyboardEvent};

use crate::canvas::{CanvasState, OpKind};
use crate::export::{self, Exporter};
use crate::input::{ImageInput, target_is_editable};
use rules::{Action, Context, KeyInput, Platform};

/// The platform whose modifier labels to show (⌘ on macOS, Ctrl elsewhere).
pub fn platform() -> Platform {
    let name = window().navigator().platform().unwrap_or_default();
    Platform::from_navigator(&name)
}

/// The shortcut(s) for `action` as a tooltip suffix, e.g. `"R"` or
/// `"⌘Z"`, for the current platform.
pub fn hint(action: Action) -> String {
    rules::hint(action, platform())
}

/// `"{label} ({shortcut})"`, the tooltip of a button with a shortcut.
pub fn titled(label: &str, action: Action) -> String {
    format!("{label} ({})", hint(action))
}

/// `action`'s `aria-keyshortcuts` value.
pub fn aria(action: Action) -> String {
    rules::aria(action)
}

/// The shortcut sheet's state, shared through context so a button can open
/// it. Cheap to copy.
#[derive(Clone, Copy)]
pub struct Shortcuts {
    open: RwSignal<bool>,
    close_button: NodeRef<html::Button>,
    /// What had focus before the sheet opened, to give it back on close.
    return_focus: StoredValue<Option<HtmlElement>, LocalStorage>,
}

impl Shortcuts {
    /// Creates the state and provides it to descendants. Call once, in `App`.
    pub fn provide() -> Self {
        let this = Self {
            open: RwSignal::new(false),
            close_button: NodeRef::new(),
            return_focus: StoredValue::new_local(None),
        };
        provide_context(this);
        this
    }

    /// The state provided by `App`. Panics outside the app tree.
    pub fn expect() -> Self {
        expect_context()
    }

    pub fn is_open(&self) -> bool {
        self.open.get_untracked()
    }

    pub fn show(&self) {
        if self.is_open() {
            return;
        }
        let active = document()
            .active_element()
            .and_then(|e| e.dyn_into::<HtmlElement>().ok());
        self.return_focus.set_value(active);
        self.open.set(true);
        let close = self.close_button;
        request_animation_frame(move || {
            if let Some(button) = close.get_untracked() {
                let _ = button.focus();
            }
        });
    }

    pub fn hide(&self) {
        if !self.is_open() {
            return;
        }
        self.open.set(false);
        if let Some(el) = self.return_focus.try_update_value(Option::take).flatten() {
            let _ = el.focus();
        }
    }

    pub fn toggle(&self) {
        if self.is_open() {
            self.hide();
        } else {
            self.show();
        }
    }
}

/// Everything a shortcut can drive. Cheap to copy into the listener.
#[derive(Clone, Copy)]
struct Targets {
    canvas: CanvasState,
    input: ImageInput,
    exporter: Exporter,
    sheet: Shortcuts,
}

impl Targets {
    fn on_keydown(&self, ev: KeyboardEvent) {
        if ev.default_prevented() || ev.is_composing() || ev.key() == "Process" {
            return;
        }
        let (key, code) = (ev.key(), ev.code());
        let input = KeyInput {
            key: &key,
            code: &code,
            ctrl: ev.ctrl_key(),
            meta: ev.meta_key(),
            shift: ev.shift_key(),
            alt: ev.alt_key(),
        };
        let ctrl = self.canvas.controller;
        let ctx = Context {
            text_field: target_is_editable(ev.target()),
            text_selected: export::page_text_selected(),
            dragging: ctrl.with_untracked(|c| c.dragging() || c.reordering()),
            sheet_open: self.sheet.is_open(),
        };
        let Some(binding) = rules::lookup(&input, ctx) else {
            return;
        };
        if self.run(binding.action, ev.repeat()) {
            ev.prevent_default();
        }
    }

    /// Runs `action`; returns whether to prevent the key's default.
    fn run(&self, action: Action, repeat: bool) -> bool {
        let has_image = self.exporter.has_image();
        match action {
            Action::Cancel | Action::ShowSheet if self.sheet.is_open() => {
                self.sheet.hide();
                true
            }
            Action::ShowSheet => {
                if !repeat {
                    self.sheet.show();
                }
                true
            }
            Action::PasteImage => false,
            Action::OpenFile => {
                // Always: stop the browser's own Open dialog.
                if !repeat {
                    self.input.open_picker();
                }
                true
            }
            Action::Download => {
                // Always: never the browser's "Save page".
                if has_image && !repeat {
                    self.exporter.run(export::rules::Action::Download);
                }
                true
            }
            // Without an image, ⌘C and C keep their ordinary meaning.
            Action::CopyImage | Action::CopyCaption if !has_image => false,
            Action::CopyImage | Action::CopyCaption => {
                if !repeat {
                    self.exporter.run(if action == Action::CopyImage {
                        export::rules::Action::CopyImage
                    } else {
                        export::rules::Action::CopyCaption
                    });
                }
                true
            }
            Action::Tool(_)
            | Action::Op(_)
            | Action::Cancel
            | Action::DeleteSelected
            | Action::Undo
            | Action::Redo => {
                let done = self
                    .canvas
                    .controller
                    .try_update(|c| rules::apply(action, c))
                    .flatten()
                    .unwrap_or(false);
                if done
                    && action == Action::Op(OpKind::Hint)
                    && let Some(field) = self.canvas.hint_input.get_untracked()
                {
                    // `preventDefault` keeps the "h" out of the field.
                    let _ = field.focus();
                }
                done
            }
        }
    }
}

/// The window `keydown` listener for every shortcut, and the `?` sheet.
/// Mount once.
#[component]
pub fn ShortcutLayer(input: ImageInput, exporter: Exporter) -> impl IntoView {
    let sheet = Shortcuts::expect();
    let targets = Targets {
        canvas: CanvasState::expect(),
        input,
        exporter,
        sheet,
    };
    let handle = window_event_listener(ev::keydown, move |e| targets.on_keydown(e));
    on_cleanup(move || handle.remove());

    let sections = rules::sheet(platform())
        .into_iter()
        .map(|section| {
            let rows = section
                .rows
                .into_iter()
                .map(|row| {
                    let keys = row
                        .keys
                        .into_iter()
                        .enumerate()
                        .map(|(i, k)| {
                            view! {
                                {(i > 0).then_some(" / ")}
                                <kbd>{k}</kbd>
                            }
                        })
                        .collect_view();
                    view! {
                        <tr>
                            <td class="shortcut-keys">{keys}</td>
                            <td>{row.description}</td>
                        </tr>
                    }
                })
                .collect_view();
            view! {
                <section>
                    <h3>{section.title}</h3>
                    <table>
                        <tbody>{rows}</tbody>
                    </table>
                </section>
            }
        })
        .collect_view();

    let on_backdrop = move |ev: web_sys::MouseEvent| {
        if ev.target() == ev.current_target() {
            sheet.hide();
        }
    };

    view! {
        <div class="shortcut-sheet" hidden=move || !sheet.open.get() on:click=on_backdrop>
            <div
                class="shortcut-sheet-panel"
                role="dialog"
                aria-modal="true"
                aria-labelledby="shortcut-sheet-title"
            >
                <div class="shortcut-sheet-header">
                    <h2 id="shortcut-sheet-title">"Keyboard shortcuts"</h2>
                    <button
                        type="button"
                        class="toast-close"
                        aria-label="Close"
                        title=format!("Close ({} / {})", hint(Action::Cancel), hint(Action::ShowSheet))
                        node_ref=sheet.close_button
                        on:click=move |_| sheet.hide()
                    >
                        "×"
                    </button>
                </div>
                <p class="shortcut-sheet-note">
                    "Single-key shortcuts are off while you type in a text field."
                </p>
                <div class="shortcut-sheet-body">{sections}</div>
            </div>
        </div>
    }
}

/// The header button that opens the shortcut sheet.
#[component]
pub fn ShortcutsButton() -> impl IntoView {
    let sheet = Shortcuts::expect();
    view! {
        <button
            type="button"
            aria-keyshortcuts=aria(Action::ShowSheet)
            title=titled("Keyboard shortcuts", Action::ShowSheet)
            on:click=move |_| sheet.toggle()
        >
            "Shortcuts"
        </button>
    }
}
