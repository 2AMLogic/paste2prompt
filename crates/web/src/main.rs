//! Browser entry point. Built with `trunk`; see the README for how to run it.

mod canvas;
mod history_panel;
mod image;
mod input;

use leptos::prelude::*;
use paste2prompt_core::APP_NAME;

use crate::canvas::{CanvasState, CanvasToolbar, CanvasView, SourceImage};
use crate::history_panel::{HistoryPanel, InstructionsBox};
use crate::image::ImageStore;
use crate::input::{ImageInput, ImageInputLayer, OpenButton};

fn main() {
    console_error_panic_hook::set_once();
    leptos::mount::mount_to_body(App);
}

/// App shell: header with **Open**, the empty state until an image is
/// loaded, then the canvas view with its tools, the instructions box and
/// the history panel.
#[component]
fn App() -> impl IntoView {
    let store = ImageStore::provide();
    let canvas = CanvasState::provide();
    let input = ImageInput::new(store, canvas.has_edits());
    let current = store.current();
    // `?test-pattern` shows a generated image until a real one is loaded.
    let test_pattern = canvas::test_pattern_requested()
        .then(|| SourceImage::test_pattern(1280, 800).ok())
        .flatten();
    let image = Signal::derive_local(move || {
        current
            .with(|i| i.as_ref().map(SourceImage::from_loaded))
            .or_else(|| test_pattern.clone())
    });
    let empty = move || image.with(Option::is_none);

    view! {
        <header>
            <h1>{APP_NAME}</h1>
            <OpenButton input />
        </header>
        <main>
            <Show when=empty fallback=move || view! { <ImageInfo /> }>
                <section class="empty-state">
                    <p class="empty-title">"Paste, drop, or open an image"</p>
                    <p class="empty-hint">"⌘V / Ctrl+V to paste · ⌘O / Ctrl+O to open"</p>
                    <OpenButton input class="primary" />
                </section>
            </Show>
            <section class="canvas-panel" hidden=empty>
                <CanvasToolbar />
                <div class="workspace">
                    <div class="workspace-main">
                        <CanvasView image />
                        <InstructionsBox />
                    </div>
                    <HistoryPanel />
                </div>
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
