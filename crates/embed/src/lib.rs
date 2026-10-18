//! The JavaScript API of the embed, for `packages/element`.
//!
//! ```js
//! import init, { open } from "./wasm/paste2prompt_embed.js";
//! await init();
//! const editor = open(shadowRoot, container, pngBlob, "Page capture", {
//!   repo: "owner/name", labels: ["triage"], context: "Build 1.2.3", page: location.href,
//! });
//! // …later
//! editor.close();
//! ```

use js_sys::{Array, Reflect};
use paste2prompt_web::embed::{self, Embedded, IssueTarget};
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
///
/// `issue`, when given, is `{ repo, labels?, context?, page? }` and adds an
/// **Open issue** button for `repo` (`owner/name`).
#[wasm_bindgen]
pub fn open(
    shadow: ShadowRoot,
    container: HtmlElement,
    image: Blob,
    name: Option<String>,
    issue: Option<js_sys::Object>,
) -> Result<Editor, JsError> {
    let issue = issue.map(|o| issue_target(&o)).transpose()?;
    Ok(Editor {
        embedded: embed::mount(shadow, container, image, name, issue),
    })
}

fn issue_target(o: &JsValue) -> Result<IssueTarget, JsError> {
    let string = |key: &str| {
        Reflect::get(o, &key.into())
            .ok()
            .and_then(|v| v.as_string())
            .unwrap_or_default()
    };
    let repo = string("repo");
    if repo.split('/').filter(|part| !part.is_empty()).count() != 2 {
        return Err(JsError::new("issue.repo must be \"owner/name\""));
    }
    let labels = Reflect::get(o, &"labels".into())
        .ok()
        .filter(Array::is_array)
        .map(|v| {
            Array::from(&v)
                .iter()
                .filter_map(|l| l.as_string())
                .collect()
        })
        .unwrap_or_default();
    Ok(IssueTarget {
        repo,
        labels,
        context: string("context"),
        page: string("page"),
    })
}
