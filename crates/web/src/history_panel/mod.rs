//! The history panel (ordered edit list with select, delete and reorder),
//! the image-level instructions box, and the undo/redo buttons.
//!
//! Everything goes through the shared [`Controller`] (see
//! [`crate::canvas::CanvasState`]), so the panel and the canvas always show
//! the same document and selection.
//!
//! - A row click is [`Controller::select`]; its delete button is
//!   [`Controller::delete_edit`].
//! - The up and down buttons are one [`Controller::move_edit`] each, so one
//!   undo step each.
//! - Dragging a row by its grip is one gesture
//!   ([`Controller::begin_reorder`] … [`Controller::end_reorder`]), so one
//!   undo step however far it moves; `pointercancel` reverts it.
//! - **Clear edits** is [`Controller::clear_edits`]: every edit and the
//!   instructions go in one undo step, so it needs no confirmation.
//! - The instructions box calls [`Controller::set_instructions`] per
//!   keystroke (which coalesces) and [`Controller::end_instructions_edit`]
//!   on focus and blur, so one typing session is one undo step.
//!
//! # Undo/redo keys
//!
//! `⌘Z` / `Ctrl+Z` (undo) and `⌘⇧Z` / `Ctrl+Shift+Z` / `Ctrl+Y` (redo) are
//! bound in the central keymap ([`crate::keymap`]), which leaves text fields
//! to their native undo, and makes `Esc` revert a drag-reorder
//! ([`Controller::cancel_reorder`]).

pub mod rules;
#[cfg(test)]
mod tests;

use leptos::html;
use leptos::logging::warn;
use leptos::prelude::*;
use paste2prompt_core::{EditError, EditId};
use wasm_bindgen::JsCast;
use web_sys::{HtmlElement, PointerEvent};

use crate::canvas::colors::mark_class;
use crate::canvas::{CanvasState, Controller};
use crate::keymap::{self, rules::Action};
use rules::{Row, Step, step_index};

fn report(result: Result<(), EditError>) {
    if let Err(e) = result {
        warn!("history change rejected: {e}");
    }
}

/// Focuses `selector` once the list has re-rendered, so keyboard users keep
/// their place after a row moves or disappears.
fn focus_later(selector: String) {
    request_animation_frame(move || {
        if let Some(el) = document()
            .query_selector(&selector)
            .ok()
            .flatten()
            .and_then(|e| e.dyn_into::<HtmlElement>().ok())
        {
            let _ = el.focus();
        }
    });
}

/// A drag-reorder in progress: which edit, and which pointer.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Reorder {
    id: EditId,
    pointer: i32,
}

/// The history panel: undo and redo, then one row per edit in order.
/// Mount once.
#[component]
pub fn HistoryPanel() -> impl IntoView {
    let ctrl = CanvasState::expect().controller;

    let list_ref = NodeRef::<html::Ol>::new();
    let reorder = RwSignal::new(None::<Reorder>);
    let rows = Memo::new(move |_| {
        ctrl.with(|c| {
            c.editor()
                .map(|e| rules::rows(e.document(), c.selected()))
                .unwrap_or_default()
        })
    });
    let empty = move || rows.with(Vec::is_empty);

    let on_move = move |ev: PointerEvent| {
        let Some(r) = reorder.get_untracked() else {
            return;
        };
        if r.pointer != ev.pointer_id() {
            return;
        }
        let Some(list) = list_ref.get_untracked() else {
            return;
        };
        // Midpoints of the rows as laid out now, and where the dragged one is.
        let children = list.children();
        let mut mids = Vec::with_capacity(children.length() as usize);
        let mut from = None;
        let key = r.id.get().to_string();
        for i in 0..children.length() {
            let Some(row) = children.item(i) else {
                continue;
            };
            let rect = row.get_bounding_client_rect();
            if row.get_attribute("data-edit-id").as_deref() == Some(key.as_str()) {
                from = Some(mids.len());
            }
            mids.push(rect.top() + rect.height() / 2.0);
        }
        let Some(from) = from else { return };
        let to = rules::reorder_index(&mids, from, f64::from(ev.client_y()));
        if to != from {
            ctrl.update(|c| report(c.reorder_to(r.id, to)));
        }
    };
    let finish = move |ev: &PointerEvent| -> bool {
        if reorder.get_untracked().map(|r| r.pointer) != Some(ev.pointer_id()) {
            return false;
        }
        reorder.set(None);
        true
    };
    let on_up = move |ev: PointerEvent| {
        if finish(&ev) {
            ctrl.update(Controller::end_reorder);
            if let Some(list) = list_ref.get_untracked()
                && list.has_pointer_capture(ev.pointer_id())
            {
                let _ = list.release_pointer_capture(ev.pointer_id());
            }
        }
    };
    let on_cancel = move |ev: PointerEvent| {
        if finish(&ev) {
            ctrl.update(Controller::cancel_reorder);
        }
    };

    let start_drag = move |id: EditId, ev: PointerEvent| {
        if !ev.is_primary() || ev.button() != 0 || reorder.get_untracked().is_some() {
            return;
        }
        ev.prevent_default();
        let Some(list) = list_ref.get_untracked() else {
            return;
        };
        if !ctrl.try_update(Controller::begin_reorder).unwrap_or(false) {
            return;
        }
        // Capture on the list, which never re-renders: the rows do.
        if let Err(e) = list.set_pointer_capture(ev.pointer_id()) {
            warn!("pointer capture failed: {e:?}");
        }
        reorder.set(Some(Reorder {
            id,
            pointer: ev.pointer_id(),
        }));
    };

    let row_view = move |index: usize, len: usize, row: Row| {
        let Row {
            id,
            number,
            op,
            name,
            summary,
            selected,
        } = row;
        let key = id.get();
        let dragging = reorder.with(|r| r.is_some_and(|r| r.id == id));
        let class = match (selected, dragging) {
            (_, true) => "history-row selected dragging",
            (true, false) => "history-row selected",
            (false, false) => "history-row",
        };
        let up = step_index(index, len, Step::Up);
        let down = step_index(index, len, Step::Down);
        let move_to = move |to: Option<usize>, action: &'static str| {
            let Some(to) = to else { return };
            ctrl.update(|c| report(c.move_edit(id, to)));
            // The same button, or its sibling if this one is now disabled.
            let other = if action == "up" { "down" } else { "up" };
            let at_end = (action == "up" && to == 0) || (action == "down" && to + 1 == len);
            let target = if at_end { other } else { action };
            focus_later(format!(
                "[data-edit-id=\"{key}\"] [data-action=\"{target}\"]"
            ));
        };
        let delete = move |_| {
            ctrl.update(|c| {
                c.delete_edit(id);
            });
            let next = index.min(len.saturating_sub(2));
            if len > 1 {
                focus_later(format!(
                    ".history-list > li:nth-child({}) .history-select",
                    next + 1
                ));
            }
        };
        let quoted = name.as_ref().map(|n| format!("\u{201c}{n}\u{201d}"));
        let label = match &quoted {
            Some(q) => format!("Edit {number}: {op} {q}, {summary}"),
            None => format!("Edit {number}: {op}, {summary}"),
        };
        view! {
            <li class=class data-edit-id=key.to_string()>
                <span
                    class="history-grip"
                    aria-hidden="true"
                    title="Drag to reorder"
                    on:pointerdown=move |ev| start_drag(id, ev)
                >
                    "⠿"
                </span>
                <button
                    type="button"
                    class="history-select"
                    aria-pressed=selected.to_string()
                    aria-label=format!("Select {label}")
                    title=label
                    on:click=move |_| ctrl.update(|c| c.select(Some(id)))
                >
                    <span class=format!("badge {}", mark_class(number))>{number}</span>
                    <span class="history-op">{op}</span>
                    {quoted.map(|q| view! { <span class="history-name">{q}</span> })}
                    <span class="history-summary">{summary}</span>
                </button>
                <button
                    type="button"
                    class="history-action"
                    data-action="up"
                    aria-label=format!("Move edit {number} up")
                    title="Move up"
                    disabled=up.is_none()
                    on:click=move |_| move_to(up, "up")
                >
                    "↑"
                </button>
                <button
                    type="button"
                    class="history-action"
                    data-action="down"
                    aria-label=format!("Move edit {number} down")
                    title="Move down"
                    disabled=down.is_none()
                    on:click=move |_| move_to(down, "down")
                >
                    "↓"
                </button>
                <button
                    type="button"
                    class="history-action"
                    data-action="delete"
                    aria-label=format!("Delete edit {number}")
                    title="Delete"
                    on:click=delete
                >
                    "✕"
                </button>
            </li>
        }
    };

    let list = move || {
        let rows = rows.get();
        let len = rows.len();
        rows.into_iter()
            .enumerate()
            .map(|(i, row)| row_view(i, len, row))
            .collect_view()
    };

    view! {
        <aside class="history-panel" aria-label="Edit history">
            <div class="history-header">
                <h2>"Edits"</h2>
                <button
                    type="button"
                    class="history-clear"
                    data-action="clear-edits"
                    aria-label="Clear edits: remove every edit and the instructions (undoable)"
                    title="Remove every edit and the instructions. Undo brings them back."
                    disabled=move || !ctrl.with(Controller::can_clear_edits)
                    on:click=move |_| {
                        ctrl.update(|c| {
                            c.clear_edits();
                        });
                        // This button is now disabled: keep focus nearby.
                        focus_later("[data-action=\"undo\"]".to_owned());
                    }
                >
                    "Clear edits"
                </button>
                <div class="history-undo" role="group" aria-label="Undo and redo">
                    <button
                        type="button"
                        data-action="undo"
                        aria-label="Undo"
                        aria-keyshortcuts=keymap::aria(Action::Undo)
                        title=keymap::titled("Undo", Action::Undo)
                        disabled=move || !ctrl.with(Controller::can_undo)
                        on:click=move |_| {
                            ctrl.update(|c| {
                                c.undo();
                            })
                        }
                    >
                        "Undo"
                    </button>
                    <button
                        type="button"
                        data-action="redo"
                        aria-label="Redo"
                        aria-keyshortcuts=keymap::aria(Action::Redo)
                        title=keymap::titled("Redo", Action::Redo)
                        disabled=move || !ctrl.with(Controller::can_redo)
                        on:click=move |_| {
                            ctrl.update(|c| {
                                c.redo();
                            })
                        }
                    >
                        "Redo"
                    </button>
                </div>
            </div>
            <p class="history-empty" hidden=move || !empty()>
                "No edits yet. Draw a region on the image."
            </p>
            <ol
                node_ref=list_ref
                class="history-list"
                on:pointermove=on_move
                on:pointerup=on_up
                on:pointercancel=on_cancel
                on:lostpointercapture=on_cancel
            >
                {list}
            </ol>
        </aside>
    }
}

/// The image-level instructions (spec §The flow, step 4): under the canvas,
/// or above the history panel beside it, by arrangement (#73).
#[component]
pub fn InstructionsBox() -> impl IntoView {
    let state = CanvasState::expect();
    let ctrl = state.controller;
    let value = move || ctrl.with(|c| c.instructions().to_owned());
    view! {
        <label node_ref=state.instructions class="instructions">
            <span class="instructions-label">"Instructions for the whole image"</span>
            <textarea
                class="instructions-input"
                rows="3"
                placeholder="e.g. tighten the spacing between the cards"
                prop:value=value
                on:focus=move |_| ctrl.update(Controller::end_instructions_edit)
                on:blur=move |_| ctrl.update(Controller::end_instructions_edit)
                on:input=move |ev| {
                    let text = event_target_value(&ev);
                    ctrl.update(|c| c.set_instructions(&text));
                }
            ></textarea>
        </label>
    }
}
