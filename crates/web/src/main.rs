//! Browser entry point. Built with `trunk`; see the README for how to run it.

mod canvas;
mod export;
mod history_panel;
mod image;
mod input;
mod keymap;

use leptos::prelude::*;
use paste2prompt_core::APP_NAME;

use crate::canvas::{CanvasState, CanvasToolbar, CanvasView, SourceImage};
use crate::export::{DocumentSource, ExportToolbar, Exporter};
use crate::history_panel::{HistoryPanel, InstructionsBox};
use crate::image::ImageStore;
use crate::input::{ImageInput, ImageInputLayer, OpenButton, StartOverButton};
use crate::keymap::{ShortcutLayer, Shortcuts, ShortcutsButton, rules::Action as KeyAction};

fn main() {
    console_error_panic_hook::set_once();
    leptos::mount::mount_to_body(App);
}

/// App shell: header with **Open** and **Start over**, the empty state until an image is
/// loaded, then the canvas view with its tools, the instructions box and
/// the history panel, plus the keyboard shortcuts and their `?` sheet.
#[component]
fn App() -> impl IntoView {
    let store = ImageStore::provide();
    let canvas = CanvasState::provide();
    Shortcuts::provide();
    let input = ImageInput::new(store, canvas.has_edits());
    // Exports read the canvas editor's document (edits and hints) at the
    // gesture. `None` (no session yet) falls back to an empty document.
    let controller = canvas.controller;
    let exporter = Exporter::new(
        store,
        DocumentSource::new(move || {
            controller.with_untracked(|c| c.editor().map(|e| e.document().clone()))
        }),
    );
    let current = store.current();
    // Debug builds only: a generated test card, shown until a real image is
    // loaded. Release builds compile this out. It is not in `ImageStore`, so
    // the export actions stay disabled while it is showing: load a real image
    // to try them.
    #[cfg(debug_assertions)]
    let test_pattern = RwSignal::new_local(None::<SourceImage>);
    // A real image replaces the test card for good, so Start over shows
    // the empty state rather than the card again.
    #[cfg(debug_assertions)]
    Effect::new(move |_| {
        if current.with(Option::is_some) {
            test_pattern.set(None);
        }
    });
    #[cfg(debug_assertions)]
    let fallback = move || test_pattern.get();
    #[cfg(not(debug_assertions))]
    let fallback = || None;
    let image = Signal::derive_local(move || {
        current
            .with(|i| i.as_ref().map(SourceImage::from_loaded))
            .or_else(fallback)
    });
    #[cfg(debug_assertions)]
    let dev_tools = move || {
        view! {
            <button
                type="button"
                class="dev-test-pattern"
                on:click=move |_| test_pattern.set(SourceImage::test_pattern(1280, 800).ok())
            >
                "Show test pattern (debug build)"
            </button>
        }
    };
    #[cfg(not(debug_assertions))]
    let dev_tools = || ();
    let empty = move || image.with(Option::is_none);
    let empty_hint = format!(
        "{} to paste · {} to open · ? for shortcuts",
        keymap::hint(KeyAction::PasteImage),
        keymap::hint(KeyAction::OpenFile),
    );

    view! {
        // Two groups so the header can wrap on a phone: the title with Open
        // and Start over, then the export actions and Shortcuts on their own
        // row (style.css, "Header").
        <header>
            <h1>{APP_NAME}</h1>
            <OpenButton input />
            <StartOverButton input />
            <div class="header-actions">
                <ExportToolbar exporter />
                <ShortcutsButton />
            </div>
        </header>
        <main>
            <Show when=empty fallback=move || view! { <ImageInfo /> }>
                <section class="empty-state">
                    <p class="empty-title">"Paste, drop, or open an image"</p>
                    <p class="empty-hint">{empty_hint.clone()}</p>
                    <OpenButton input class="primary" />
                    {dev_tools}
                </section>
            </Show>
            <section class="canvas-panel" hidden=empty>
                <CanvasToolbar />
                <div node_ref=canvas.workspace class=move || canvas.arrangement.get().class()>
                    <div class="workspace-main">
                        <CanvasView image />
                        <InstructionsBox />
                    </div>
                    <HistoryPanel />
                </div>
            </section>
        </main>
        <ImageInputLayer input />
        <ShortcutLayer input exporter />
    }
}

/// One line describing the loaded image.
#[component]
fn ImageInfo() -> impl IntoView {
    let current = ImageStore::expect().current();
    move || {
        current.with(|img| {
            img.as_ref().map(|img| {
                let name = img.name().unwrap_or("Pasted image").to_owned();
                view! {
                    <p class="image-info">
                        {name} " — " {img.width()} " × " {img.height()} " px"
                    </p>
                }
            })
        })
    }
}
