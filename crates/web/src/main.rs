//! Browser entry point. Built with `trunk`; see the README for how to run it.

mod image;
mod input;

use leptos::prelude::*;
use paste2prompt_core::APP_NAME;

use crate::image::ImageStore;
use crate::input::{ImageInput, ImageInputLayer, OpenButton};

fn main() {
    console_error_panic_hook::set_once();
    leptos::mount::mount_to_body(App);
}

/// App shell: header with **Open**, the empty state until an image is
/// loaded, then the (still placeholder) canvas. The canvas view, tools and
/// controls arrive in later issues.
#[component]
fn App() -> impl IntoView {
    let store = ImageStore::provide();
    // No edit model yet, so loading never needs a confirmation. The history
    // work (#11) replaces this with "the session has at least one edit".
    let has_edits = Signal::derive(|| false);
    let input = ImageInput::new(store, has_edits);
    let current = store.current();
    let empty = move || current.with(Option::is_none);

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
            <canvas id="stage" width="800" height="500" hidden=empty aria-label="Image canvas"></canvas>
        </main>
        <ImageInputLayer input />
    }
}

/// One line describing the loaded image, until the canvas view (#10) draws it.
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
