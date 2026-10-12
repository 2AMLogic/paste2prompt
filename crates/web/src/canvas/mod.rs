//! The canvas view: the image fitted to the viewport, region tools (rectangle,
//! lasso) and operation handles (translate, scale, rotate, mirror, hint).
//!
//! - [`controller`]: the pure interaction state machine over the core
//!   `Editor` (tested natively).
//! - [`geom`]: hit-testing, lasso simplification and handle math (tested
//!   natively).
//! - `view`: image-pixel / screen-pixel mapping.
//! - `render`: Canvas2D drawing.
//! - `source`: the drawable image (#9's `LoadedImage`, or a test pattern).
//!
//! # Integration points
//!
//! [`CanvasState`] is provided once by `App` and shared through context.
//! Other UI drives the canvas through its [`Controller`]:
//!
//! - **Shortcuts (#13):** `state.controller.update(|c| c.set_tool(Tool::Rect))`,
//!   `c.choose_op(OpKind::Mirror)` (again toggles the axis), `c.cancel()` for
//!   `Esc`, `c.delete_selected()`, `c.undo()` / `c.redo()`; focus
//!   [`CanvasState::hint_input`] for `H`. Skip single-key shortcuts while
//!   [`Controller::dragging`] is true or a text field has focus.
//! - **History panel and undo UI (#11):** read the document with
//!   `c.editor()`, select a row with `c.select(Some(id))`, undo/redo with
//!   `c.undo()` / `c.redo()` and `c.can_undo()` / `c.can_redo()`.
//! - **Image input (#9):** [`CanvasState::has_edits`] is the "replacing the
//!   image needs a confirmation" signal; a new `ImageStore::current()` value
//!   starts a new session.

pub mod controller;
pub mod geom;
mod render;
mod source;
mod view;

use leptos::html;
use leptos::logging::warn;
use leptos::prelude::*;
use paste2prompt_core::{EditError, MirrorAxis};
use wasm_bindgen::JsCast;
use web_sys::{CanvasRenderingContext2d, PointerEvent};

pub use controller::{Controller, Cursor, OpKind, Pointer, Tool};
pub use source::{SourceImage, test_pattern_requested};
use view::ViewTransform;

/// Margin around the fitted image, in CSS pixels. Leaves room for badges
/// and handles at the image edge.
const VIEW_MARGIN: f64 = 24.0;

/// Shared canvas state, provided once by `App`.
#[derive(Clone, Copy)]
pub struct CanvasState {
    /// Tool, operation, selection, drag and the editor (document + history).
    pub controller: RwSignal<Controller>,
    /// The hint text field, for the `H` shortcut to focus.
    pub hint_input: NodeRef<html::Input>,
}

impl CanvasState {
    /// Creates the state and provides it to descendants. Call once, in `App`.
    pub fn provide() -> Self {
        let state = Self {
            controller: RwSignal::new(Controller::new()),
            hint_input: NodeRef::new(),
        };
        provide_context(state);
        state
    }

    /// The state provided by `App`. Panics outside the app tree.
    pub fn expect() -> Self {
        expect_context()
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
    // Bumped to redraw when the layout changes.
    let relayout = RwSignal::new(0_u32);

    let handle = window_event_listener(leptos::ev::resize, move |_| {
        relayout.update(|n| *n = n.wrapping_add(1));
    });
    on_cleanup(move || handle.remove());

    // New image, new session.
    Effect::new(move |_| {
        let size = image.with(|i| i.as_ref().and_then(SourceImage::size));
        active.set_value(None);
        ctrl.update(|c| match size {
            Some(size) => c.load(size),
            None => c.unload(),
        });
    });

    // Draw whenever the document, selection, drag or layout changes.
    Effect::new(move |_| {
        relayout.track();
        let Some(img) = image.get() else { return };
        let Some(canvas) = canvas_ref.get() else {
            return;
        };
        let rect = canvas.get_bounding_client_rect();
        let (w, h) = (rect.width(), rect.height());
        if w < 1.0 || h < 1.0 {
            // Not laid out yet (just un-hidden): try again next frame.
            request_animation_frame(move || relayout.update(|n| *n = n.wrapping_add(1)));
            return;
        }
        let Some(size) = img.size() else { return };
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
            VIEW_MARGIN,
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
        let rect = canvas.get_bounding_client_rect();
        let screen = (
            f64::from(ev.client_x()) - rect.left(),
            f64::from(ev.client_y()) - rect.top(),
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

/// Minimal tool and operation buttons, the mirror-axis toggle, the hint
/// field and Delete. Keyboard shortcuts (#13) and the history panel (#11)
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
                    title=format!("{} ({})", t.label(), t.key())
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
            view! {
                <button
                    type="button"
                    data-op=k.label()
                    title=format!("{} ({})", k.label(), k.key())
                    aria-pressed=move || (ctrl.with(Controller::op_kind) == k).to_string()
                    on:click=move |_| {
                        ctrl.update(|c| report(c.choose_op(k)));
                        if k == OpKind::Hint
                            && let Some(input) = state.hint_input.get_untracked()
                        {
                            let _ = input.focus();
                        }
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

    let status = move || {
        ctrl.with(|c| {
            let Some(editor) = c.editor() else {
                return "No image".to_owned();
            };
            let doc = editor.document();
            let n = doc.edits().len();
            match c.selected_edit() {
                Some(e) => format!(
                    "{n} edits · #{} {} selected",
                    doc.number_of(e.id()).unwrap_or(0),
                    e.op().name()
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
                    title="Toggle mirror axis (M)"
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
            <button
                type="button"
                data-action="delete"
                title="Delete the selected edit (Delete)"
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
