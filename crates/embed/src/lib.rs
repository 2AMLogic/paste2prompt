//! The JavaScript API of the embed, for `packages/element`.
//!
//! ```js
//! import init, { open } from "./wasm/paste2prompt_embed.js";
//! await init();
//! const editor = open(shadowRoot, container, pngBlob, "Page capture");
//! // …later
//! editor.close();
//! ```

use paste2prompt_web::embed::{self, Embedded};
use wasm_bindgen::prelude::*;
use web_sys::{Blob, HtmlElement, ShadowRoot};

/// The open editor. `close()` unmounts it; so does garbage collection, but
/// don't rely on that.
#[wasm_bindgen]
pub struct Editor {
    embedded: Embedded,
}

#[wasm_bindgen]
impl Editor {
    /// Whether the user has made any edits, which closing would discard.
    #[wasm_bindgen(js_name = hasEdits)]
    pub fn has_edits(&self) -> bool {
        self.embedded.has_edits()
    }

    /// Unmounts the editor and stops its keyboard, paste and drop listeners.
    pub fn close(self) {}
}

/// Opens the editor in `container` (an element inside `shadow` that the
/// editor scrolls in) with `image` loaded. Close the previous editor first.
#[wasm_bindgen]
pub fn open(
    shadow: ShadowRoot,
    container: HtmlElement,
    image: Blob,
    name: Option<String>,
) -> Editor {
    Editor {
        embedded: embed::mount(shadow, container, image, name),
    }
}
