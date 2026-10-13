//! The canvas view: the image fitted to the viewport, region tools (rectangle,
//! lasso), operation handles (translate, scale, rotate, mirror, hint) and a
//! selected lasso's control-point handles.
//!
//! - [`controller`]: the pure interaction state machine over the core
//!   `Editor` (tested natively).
//! - [`geom`]: hit-testing, capped lasso simplification and handle math (tested
//!   natively).
//! - `view`: image-pixel / screen-pixel mapping.
//! - [`arrange`]: the stage size that hugs the image, and whether the
//!   history panel goes beside or below (tested natively).
//! - `layout`: re-layout triggers for size and device-pixel-ratio changes,
//!   and the DOM measuring behind `arrange`.
//! - [`colors`]: per-edit colors from the export's palette, shared with
//!   the history panel (tested natively).
//! - `render`: Canvas2D drawing.
//! - `source`: the drawable image (#9's `LoadedImage`, or a debug-only test
//!   pattern).
//!
//! # Integration points
//!
//! [`CanvasState`] is provided once by `App` and shared through context.
//! Other UI drives the canvas through its [`Controller`]:
//!
//! - **Shortcuts (#13):** [`crate::keymap`] drives `c.set_tool`,
//!   `c.choose_op` (`M` again toggles the axis), `c.cancel()` for `Esc`,
//!   `c.delete_selected()` and `c.undo()` / `c.redo()`, and focuses
//!   [`CanvasState::hint_input`] for `H`. `N` is `c.name_key()`, after which
//!   [`CanvasState::focus_name_if_asked`] focuses the name field. Single keys are skipped while
//!   [`Controller::dragging`] is true or a text field has focus. The
//!   toolbar's tooltips quote the keymap.
//! - **History panel and undo UI (#11):** read the document with
//!   `c.editor()`, select a row with `c.select(Some(id))`, undo/redo with
//!   `c.undo()` / `c.redo()` and `c.can_undo()` / `c.can_redo()`.
//! - **Image input (#9):** [`CanvasState::has_edits`] is the "replacing the
//!   image needs a confirmation" signal; a new `ImageStore::current()` value
//!   starts a new session.

pub mod arrange;
pub mod colors;
pub mod controller;
pub mod geom;
mod layout;
mod render;
mod source;
mod view;

use leptos::html;
use leptos::logging::warn;
use leptos::prelude::*;
use paste2prompt_core::{EditError, EditId, ImageSize, MAX_NAME_CHARS, MirrorAxis};
use wasm_bindgen::JsCast;
use web_sys::{CanvasRenderingContext2d, PointerEvent};

pub use arrange::Arrangement;
use arrange::Chrome;
pub use controller::{Controller, Cursor, OpKind, Pointer, Tool, name_message};
pub use source::SourceImage;
use view::ViewTransform;

use crate::keymap::{self, rules::Action};

/// Shared canvas state, provided once by `App`.
#[derive(Clone, Copy)]
pub struct CanvasState {
    /// Tool, operation, selection, drag and the editor (document + history).
    pub controller: RwSignal<Controller>,
    /// The hint text field, for the `H` shortcut to focus.
    pub hint_input: NodeRef<html::Input>,
    /// The name text field, for the `N` shortcut to focus.
    pub name_input: NodeRef<html::Input>,
    /// Why the last name typed into the field for this edit was rejected
    /// (e.g. a duplicate), shown under the field until the next valid name,
    /// blur, or another selection.
    pub name_error: RwSignal<Option<(EditId, String)>>,
    /// The `.workspace` element (canvas column, instructions box and
    /// history panel), which the canvas measures to size itself (#69).
    pub workspace: NodeRef<html::Div>,
    /// The instructions box, whose height the canvas budgets under the
    /// stage or not, by arrangement (#73).
    pub instructions: NodeRef<html::Label>,
    /// Where the history panel sits, chosen by the canvas from the image's
    /// shape; `App` puts [`Arrangement::class`] on the workspace.
    pub arrangement: RwSignal<Arrangement>,
}

impl CanvasState {
    /// Creates the state and provides it to descendants. Call once, in `App`.
    pub fn provide() -> Self {
        let state = Self {
            controller: RwSignal::new(Controller::new()),
            hint_input: NodeRef::new(),
            name_input: NodeRef::new(),
            name_error: RwSignal::new(None),
            workspace: NodeRef::new(),
            instructions: NodeRef::new(),
            arrangement: RwSignal::new(Arrangement::default()),
        };
        provide_context(state);
        state
    }

    /// The state provided by `App`. Panics outside the app tree.
    pub fn expect() -> Self {
        expect_context()
    }

    /// Focuses the name field, with its text selected so typing replaces a
    /// new label's `label N`, if the controller asked for it (`N`, or a
    /// label just drawn). Waits a frame: the field is enabled by the same
    /// update that selected the edit.
    pub fn focus_name_if_asked(&self) {
        if !self
            .controller
            .try_update(Controller::take_name_focus)
            .unwrap_or(false)
        {
            return;
        }
        let input = self.name_input;
        request_animation_frame(move || {
            if let Some(field) = input.get_untracked() {
                let _ = field.focus();
                field.select();
            }
        });
    }

    /// Whether the current session has at least one edit. Reactive.
    pub fn has_edits(&self) -> Signal<bool> {
        let controller = self.controller;
        Signal::derive(move || {
            controller.with(|c| c.editor().is_some_and(|e| !e.document().edits().is_empty()))
        })
    }
}

fn report(result: Result<(), EditError>) {
    if let Err(e) = result {
        warn!("edit rejected: {e}");
    }
}

fn cursor_class(c: Cursor) -> &'static str {
    match c {
        Cursor::Default => "stage",
        Cursor::Crosshair => "stage cursor-crosshair",
        Cursor::Move => "stage cursor-move",
        Cursor::Resize => "stage cursor-resize",
        Cursor::Rotate => "stage cursor-rotate",
    }
}

/// The canvas. A new `image` value starts a new session (fresh editor and
/// history); `None` ends it.
#[component]
pub fn CanvasView(image: Signal<Option<SourceImage>, LocalStorage>) -> impl IntoView {
    let state = CanvasState::expect();
    let ctrl = state.controller;
    let canvas_ref = NodeRef::<html::Canvas>::new();
    let view_store = StoredValue::new(None::<ViewTransform>);
    let active = StoredValue::new(None::<i32>);
    let cursor = RwSignal::new(Cursor::Default);
    // Bumped to re-lay out and redraw when the space changes.
    let relayout = RwSignal::new(0_u32);
    let bump = move || relayout.update(|n| *n = n.wrapping_add(1));
    // The relayout count and image size the stage was last sized for.
    let laid_out = StoredValue::new(None::<(u32, ImageSize)>);

    // Any change to the space (window resize, the panel un-hiding, a
    // scrollbar appearing, the instructions box resized) or to the device
    // pixel ratio (zoom, another monitor) re-sizes the stage and its
    // backing store and re-fits the view on the next draw.
    let resize = window_event_listener(leptos::ev::resize, move |_| bump());
    on_cleanup(move || resize.remove());
    canvas_ref.on_load(move |canvas| {
        let canvas: &web_sys::Element = &canvas;
        let mut targets = vec![canvas.clone()];
        targets.extend(canvas.parent_element());
        targets.extend(
            state
                .workspace
                .get_untracked()
                .map(|w| web_sys::Element::clone(&w)),
        );
        targets.extend(
            state
                .instructions
                .get_untracked()
                .map(|e| web_sys::Element::clone(&e)),
        );
        let targets: Vec<&web_sys::Element> = targets.iter().collect();
        // Local (not Send) storage; disposing it drops the watches, which
        // disconnects the observer and removes the media listener.
        let watches = StoredValue::new_local((
            layout::SizeWatch::new(&targets, bump),
            layout::DprWatch::new(bump),
        ));
        on_cleanup(move || watches.dispose());
    });

    // New image, new session.
    Effect::new(move |_| {
        let size = image.with(|i| i.as_ref().and_then(SourceImage::size));
        active.set_value(None);
        ctrl.update(|c| match size {
            Some(size) => c.load(size),
            None => c.unload(),
        });
    });

    // Draw whenever the document, selection, drag or layout changes. A new
    // image or a layout change first re-sizes the stage to hug the image
    // (`arrange`), synchronously, so this frame already measures the new
    // size: the view, and with it hit-testing, is fitted to the stage as
    // it is now, never as it was.
    Effect::new(move |_| {
        let generation = relayout.get();
        let Some(img) = image.get() else { return };
        let Some(canvas) = canvas_ref.get() else {
            return;
        };
        let Some(size) = img.size() else { return };
        let last = laid_out.get_value();
        if last != Some((generation, size)) {
            let instructions = state.instructions.get_untracked();
            let space = state.workspace.get_untracked().and_then(|ws| {
                layout::measure_space(&ws, instructions.as_ref().map(|e| e.as_ref()))
            });
            let Some(space) = space else {
                // Not laid out yet (just un-hidden): try again next frame.
                request_animation_frame(bump);
                return;
            };
            // Hysteresis against the arrangement on screen, unless this is
            // a new image.
            let current = last
                .filter(|(_, s)| *s == size)
                .map(|_| state.arrangement.get_untracked());
            let fitted = arrange::choose_layout(
                size.width(),
                size.height(),
                space,
                Chrome::DEFAULT,
                current,
            );
            layout::size_stage(&canvas, fitted.stage_w, fitted.stage_h);
            if state.arrangement.get_untracked() != fitted.arrangement {
                state.arrangement.set(fitted.arrangement);
            }
            laid_out.set_value(Some((generation, size)));
        }
        // The content box, inside the border: what the canvas draws into.
        let (w, h) = (
            f64::from(canvas.client_width()),
            f64::from(canvas.client_height()),
        );
        if w < 1.0 || h < 1.0 {
            request_animation_frame(bump);
            return;
        }
        let dpr = window().device_pixel_ratio().max(1.0);
        let (bw, bh) = ((w * dpr).round() as u32, (h * dpr).round() as u32);
        if canvas.width() != bw {
            canvas.set_width(bw);
        }
        if canvas.height() != bh {
            canvas.set_height(bh);
        }
        let Some(ctx) = canvas
            .get_context("2d")
            .ok()
            .flatten()
            .and_then(|c| c.dyn_into::<CanvasRenderingContext2d>().ok())
        else {
            return;
        };
        let view = ViewTransform::fit(
            f64::from(size.width()),
            f64::from(size.height()),
            w,
            h,
            Chrome::DEFAULT.margin,
            arrange::max_scale(size.width(), size.height()),
        );
        view_store.set_value(Some(view));
        ctrl.update_untracked(|c| c.set_view_scale(view.scale));
        if let Err(e) = ctx.set_transform(dpr, 0.0, 0.0, dpr, 0.0, 0.0) {
            warn!("canvas transform failed: {e:?}");
            return;
        }
        ctx.set_image_smoothing_enabled(view.scale < 1.0);
        if let Err(e) = ctrl.with(|c| render::draw(&ctx, w, h, &img, &view, c)) {
            warn!("canvas draw failed: {e:?}");
        }
    });

    let to_pointer = move |ev: &PointerEvent| -> Option<Pointer> {
        let canvas = canvas_ref.get_untracked()?;
        let view = view_store.get_value()?;
        // Relative to the content box (inside the border), the box the view
        // was fitted to.
        let rect = canvas.get_bounding_client_rect();
        let screen = (
            f64::from(ev.client_x()) - rect.left() - f64::from(canvas.client_left()),
            f64::from(ev.client_y()) - rect.top() - f64::from(canvas.client_top()),
        );
        Some(Pointer {
            pos: view.to_image(screen),
            shift: ev.shift_key(),
        })
    };

    let on_down = move |ev: PointerEvent| {
        // One pointer at a time; primary mouse button, pen tip or a touch.
        if !ev.is_primary() || ev.button() != 0 || active.get_value().is_some() {
            return;
        }
        let Some(p) = to_pointer(&ev) else { return };
        ev.prevent_default();
        let started = ctrl.try_update(|c| c.pointer_down(p)).unwrap_or(false);
        if started {
            if let Some(canvas) = canvas_ref.get_untracked()
                && let Err(e) = canvas.set_pointer_capture(ev.pointer_id())
            {
                warn!("pointer capture failed: {e:?}");
            }
            active.set_value(Some(ev.pointer_id()));
        }
    };
    let on_move = move |ev: PointerEvent| {
        let Some(p) = to_pointer(&ev) else { return };
        if active.get_value() == Some(ev.pointer_id()) {
            ctrl.update(|c| c.pointer_move(p));
        } else if active.get_value().is_none() {
            let c = ctrl.with_untracked(|c| c.cursor_at(p.pos));
            if cursor.get_untracked() != c {
                cursor.set(c);
            }
        }
    };
    let on_up = move |ev: PointerEvent| {
        if active.get_value() != Some(ev.pointer_id()) {
            return;
        }
        active.set_value(None);
        if let Some(p) = to_pointer(&ev) {
            ctrl.update(|c| c.pointer_up(p));
            // A label just drawn: its name field takes focus.
            state.focus_name_if_asked();
        } else {
            ctrl.update(Controller::pointer_cancel);
        }
        if let Some(canvas) = canvas_ref.get_untracked()
            && canvas.has_pointer_capture(ev.pointer_id())
        {
            let _ = canvas.release_pointer_capture(ev.pointer_id());
        }
    };
    let on_cancel = move |ev: PointerEvent| {
        if active.get_value() == Some(ev.pointer_id()) {
            active.set_value(None);
            ctrl.update(Controller::pointer_cancel);
        }
    };

    let label = move || {
        ctrl.with(|c| {
            let n = c.editor().map_or(0, |e| e.document().edits().len());
            format!("Image canvas, {n} edits")
        })
    };

    view! {
        <canvas
            node_ref=canvas_ref
            class=move || cursor_class(cursor.get())
            aria-label=label
            on:pointerdown=on_down
            on:pointermove=on_move
            on:pointerup=on_up
            on:pointercancel=on_cancel
            on:lostpointercapture=on_cancel
        ></canvas>
    }
}

/// Minimal tool and operation buttons, the mirror-axis toggle, the hint and
/// name fields and Delete. Keyboard shortcuts (#13) and the history panel (#11)
/// drive the same [`Controller`] methods.
#[component]
pub fn CanvasToolbar() -> impl IntoView {
    let state = CanvasState::expect();
    let ctrl = state.controller;
    let has_selection = move || ctrl.with(|c| c.selected().is_some());

    let tools = Tool::ALL
        .into_iter()
        .map(|t| {
            view! {
                <button
                    type="button"
                    data-tool=t.label()
                    title=keymap::titled(t.label(), Action::Tool(t))
                    aria-keyshortcuts=keymap::aria(Action::Tool(t))
                    aria-pressed=move || (ctrl.with(Controller::tool) == t).to_string()
                    on:click=move |_| ctrl.update(|c| c.set_tool(t))
                >
                    {t.label()}
                </button>
            }
        })
        .collect_view();

    let ops = OpKind::ALL
        .into_iter()
        .map(|k| {
            // Label has no key of its own: `N` arms it (with nothing
            // selected), so its tooltip quotes `N`.
            let action = if k == OpKind::Label {
                Action::Name
            } else {
                Action::Op(k)
            };
            view! {
                <button
                    type="button"
                    data-op=k.label()
                    title=keymap::titled(k.label(), action)
                    aria-keyshortcuts=keymap::aria(action)
                    aria-pressed=move || (ctrl.with(Controller::op_kind) == k).to_string()
                    on:click=move |_| {
                        ctrl.update(|c| report(c.choose_op(k)));
                        if k == OpKind::Hint
                            && let Some(input) = state.hint_input.get_untracked()
                        {
                            let _ = input.focus();
                        }
                        state.focus_name_if_asked();
                    }
                >
                    {k.label()}
                </button>
            }
        })
        .collect_view();

    let axis_label = move || match ctrl.with(Controller::mirror_axis) {
        MirrorAxis::Horizontal => "Axis: left-right",
        MirrorAxis::Vertical => "Axis: top-bottom",
    };

    let hint_value = move || {
        ctrl.with(|c| {
            c.selected_edit()
                .and_then(|e| e.hint().map(str::to_owned))
                .unwrap_or_default()
        })
    };

    let name_value = move || {
        ctrl.with(|c| {
            c.selected_edit()
                .and_then(|e| e.name().map(str::to_owned))
                .unwrap_or_default()
        })
    };
    // Shows the stored name in the field, but only when it differs from
    // what the field already says, trimmed. Names are stored trimmed, so a
    // plain `prop:value` binding rewrote `nav ` to `nav` the moment it was
    // accepted, eating the space before the next word: typing `top nav`
    // ended as `topnav`. A rejected name doesn't notify at all
    // (`on_name_input`), so it stays in the field until blur.
    Effect::new(move |_| {
        let stored = name_value();
        if let Some(field) = state.name_input.get()
            && field.value().trim() != stored
        {
            field.set_value(&stored);
        }
    });
    let name_error = state.name_error;
    let name_message_text = move || {
        let selected = ctrl.with(Controller::selected);
        name_error.with(|e| match e {
            Some((id, message)) if Some(*id) == selected => message.clone(),
            _ => String::new(),
        })
    };
    let on_name_input = move |ev: web_sys::Event| {
        let text = event_target_value(&ev);
        let Some(id) = ctrl.with_untracked(Controller::selected) else {
            return;
        };
        // Notify only when the name was accepted: a notification re-runs
        // the field sync above, which would put the stored name back while
        // the user is still typing (typing `logo text` with `logo` taken
        // would stall at `log`; clearing a label's name would snap back).
        // A rejected name stays in the field, with its message, until blur
        // puts the stored name back.
        let result = ctrl
            .try_maybe_update(|c| {
                let r = c.set_name(&text);
                (r.is_ok(), r)
            })
            .unwrap_or(Ok(()));
        match result {
            Ok(()) => name_error.set(None),
            Err(e) => match name_message(&e, &text) {
                Some(message) => name_error.set(Some((id, message))),
                None => report(Err(e)),
            },
        }
    };
    // Leaving the field ends the typing step and puts back the stored name,
    // so a rejected name never lingers looking accepted.
    let on_name_blur = move |_| {
        ctrl.update(Controller::end_name_edit);
        name_error.set(None);
        if let Some(field) = state.name_input.get_untracked() {
            field.set_value(&name_value());
        }
    };

    let status = move || {
        ctrl.with(|c| {
            let Some(editor) = c.editor() else {
                return "No image".to_owned();
            };
            let doc = editor.document();
            let n = doc.edits().len();
            match c.selected_edit() {
                Some(e) => format!(
                    "{n} edits · #{} {}{} selected",
                    doc.number_of(e.id()).unwrap_or(0),
                    e.op().name(),
                    e.name()
                        .map(|n| format!(" \u{201c}{n}\u{201d}"))
                        .unwrap_or_default()
                ),
                None => format!("{n} edits"),
            }
        })
    };

    view! {
        <div class="toolbar" role="toolbar" aria-label="Canvas tools">
            <div class="toolbar-group" role="group" aria-label="Region tool">
                {tools}
            </div>
            <div class="toolbar-group" role="group" aria-label="Operation">
                {ops}
                <button
                    type="button"
                    data-action="mirror-axis"
                    title=format!("Toggle mirror axis ({} again)", keymap::hint(Action::Op(OpKind::Mirror)))
                    on:click=move |_| ctrl.update(|c| report(c.toggle_mirror_axis()))
                >
                    {axis_label}
                </button>
            </div>
            <input
                node_ref=state.hint_input
                type="text"
                class="hint-input"
                placeholder="Hint for the selected edit"
                aria-label="Hint for the selected edit"
                disabled=move || !has_selection()
                prop:value=hint_value
                on:focus=move |_| ctrl.update(Controller::end_hint_edit)
                on:blur=move |_| ctrl.update(Controller::end_hint_edit)
                on:input=move |ev| {
                    let text = event_target_value(&ev);
                    ctrl.update(|c| report(c.set_hint(&text)));
                }
            />
            <span class="name-field">
                <input
                    node_ref=state.name_input
                    type="text"
                    class="name-input"
                    placeholder="Name (optional)"
                    aria-label="Name of the selected region"
                    aria-describedby="name-error"
                    title=keymap::titled("Name of the selected region", Action::Name)
                    aria-keyshortcuts=keymap::aria(Action::Name)
                    maxlength=MAX_NAME_CHARS.to_string()
                    disabled=move || !has_selection()
                    aria-invalid=move || (!name_message_text().is_empty()).to_string()
                    on:focus=move |_| ctrl.update(Controller::end_name_edit)
                    on:blur=on_name_blur
                    on:input=on_name_input
                />
                <output id="name-error" class="name-error" role="alert" aria-live="assertive">
                    {name_message_text}
                </output>
            </span>
            <button
                type="button"
                data-action="delete"
                title=keymap::titled("Delete the selected edit", Action::DeleteSelected)
                disabled=move || !has_selection()
                on:click=move |_| {
                    ctrl.update(|c| {
                        c.delete_selected();
                    })
                }
            >
                "Delete"
            </button>
            <output class="canvas-status" aria-live="polite">{status}</output>
        </div>
    }
}
