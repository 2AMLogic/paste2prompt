//! The JavaScript API of the embed, for `packages/element`.
//!
//! ```js
//! import init, { open } from "./wasm/paste2prompt_embed.js";
//! await init();
//! const picks = [{ x0: 0.6, y0: 0.82, x1: 0.7, y1: 0.86,
//!   selector: "button.save", tag: "button", label: "Save" }];
//! const editor = open(shadowRoot, container, pngBlob, "Page capture", picks);
//! const url = editor.issueUrl({ repo: "owner/name", labels: ["triage"], page: location.href });
//! // …later
//! editor.close();
//! ```

mod issue;

use issue::IssueTarget;
use js_sys::{Array, Promise, Reflect};
use paste2prompt_web::embed::{self, Embedded, Pick};
use wasm_bindgen::prelude::*;
use web_sys::{Blob, HtmlElement, ShadowRoot, console};

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

    /// The caption as plain text (what **Copy caption** copies), or `""`
    /// without an image.
    pub fn caption(&self) -> String {
        self.embedded
            .caption()
            .map(|(text, _)| text)
            .unwrap_or_default()
    }

    /// The edits as paste2prompt's document JSON (format version 3), with
    /// each picked element's `selector`, `tag`, `role` and `label`; `""`
    /// without an image.
    #[wasm_bindgen(js_name = documentJson)]
    pub fn document_json(&self) -> String {
        self.embedded.document_json().unwrap_or_default()
    }

    /// A promise of the export PNG blob, as **Copy image** renders it. Call
    /// it in the click handler and hand the promise to `ClipboardItem`
    /// (Safari refuses a clipboard write that starts after an await).
    #[wasm_bindgen(js_name = renderPng)]
    pub fn render_png(&self) -> Promise {
        self.embedded.render_png()
    }

    /// The new-issue URL for `issue`, `{ repo, labels?, context?, page? }`,
    /// pre-filled from the current edits. `repo` is `owner/name`.
    #[wasm_bindgen(js_name = issueUrl)]
    pub fn issue_url(&self, issue: &JsValue) -> Result<String, JsError> {
        let target = issue_target(issue)?;
        let (caption, instructions) = self
            .embedded
            .caption()
            .ok_or_else(|| JsError::new("There is no image to report."))?;
        Ok(issue::issue_url(&target, &instructions, &caption))
    }

    /// Unmounts the editor and stops its keyboard, paste and drop listeners.
    pub fn close(self) {}
}

/// Opens the editor in `container` (an element inside `shadow` that the
/// editor scrolls in) with `image` loaded. Close the previous editor first.
///
/// `picks` are the elements picked from the page, each
/// `{ x0, y0, x1, y1, selector, tag, role?, label? }` with the corners as
/// fractions of the image's width and height; the editor starts with a hint
/// edit on each. One that isn't valid is skipped, with a console warning.
#[wasm_bindgen]
pub fn open(
    shadow: ShadowRoot,
    container: HtmlElement,
    image: Blob,
    name: Option<String>,
    picks: Option<Array>,
) -> Editor {
    let picks = picks
        .map(|a| {
            a.iter()
                .filter_map(|p| {
                    pick(&p)
                        .map_err(|e| {
                            console::warn_1(&format!("paste2prompt: pick skipped: {e}").into())
                        })
                        .ok()
                })
                .collect()
        })
        .unwrap_or_default();
    Editor {
        embedded: embed::mount(shadow, container, image, name, picks),
    }
}

fn pick(o: &JsValue) -> Result<Pick, String> {
    let number = |key: &str| {
        Reflect::get(o, &key.into())
            .ok()
            .and_then(|v| v.as_f64())
            .ok_or_else(|| format!("{key} must be a number"))
    };
    let string = |key: &str| {
        Reflect::get(o, &key.into())
            .ok()
            .and_then(|v| v.as_string())
    };
    Pick::new(
        (number("x0")?, number("y0")?),
        (number("x1")?, number("y1")?),
        string("selector").unwrap_or_default(),
        string("tag").unwrap_or_default(),
        string("role"),
        string("label"),
    )
}

fn issue_target(o: &JsValue) -> Result<IssueTarget, JsError> {
    let string = |key: &str| {
        Reflect::get(o, &key.into())
            .ok()
            .and_then(|v| v.as_string())
            .unwrap_or_default()
    };
    let repo = string("repo");
    let parts: Vec<&str> = repo.split('/').collect();
    let valid = |p: &str| {
        !p.is_empty()
            && p.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    };
    if parts.len() != 2 || !parts.iter().all(|p| valid(p)) {
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
