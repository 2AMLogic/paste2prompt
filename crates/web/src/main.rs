//! Browser entry point. Built with `trunk`; see the README for how to run it.

mod canvas;
mod export;
mod image;
mod input;

use leptos::prelude::*;
use paste2prompt_core::APP_NAME;

use crate::canvas::{CanvasState, CanvasToolbar, CanvasView, SourceImage};
use crate::export::{DocumentSource, ExportToolbar, Exporter};
use crate::image::ImageStore;
use crate::input::{ImageInput, ImageInputLayer, OpenButton};

fn main() {
    console_error_panic_hook::set_once();
    leptos::mount::mount_to_body(App);
}

/// App shell: header with **Open**, the empty state until an image is
/// loaded, then the canvas view with its tools.
#[component]
fn App() -> impl IntoView {
    let store = ImageStore::provide();
    let canvas = CanvasState::provide();
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

    view! {
        <header>
            <h1>{APP_NAME}</h1>
            <OpenButton input />
            <ExportToolbar exporter />
        </header>
        <main>
            <Show when=empty fallback=move || view! { <ImageInfo /> }>
                <section class="empty-state">
                    <p class="empty-title">"Paste, drop, or open an image"</p>
                    <p class="empty-hint">"⌘V / Ctrl+V to paste · ⌘O / Ctrl+O to open"</p>
                    <OpenButton input class="primary" />
                    {dev_tools}
                </section>
            </Show>
            <section class="canvas-panel" hidden=empty>
                <CanvasToolbar />
                <CanvasView image />
            </section>
        </main>
        <ImageInputLayer input />
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
