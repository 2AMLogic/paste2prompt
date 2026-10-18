//! Export actions (spec §Output → Export actions): **Copy image**,
//! **Download PNG** and **Copy caption**, as toolbar buttons and as the
//! shortcuts `⌘C` / `Ctrl+C`, `⌘S` / `Ctrl+S` and `C`.
//!
//! The export is rendered only on demand, from two inputs: the current
//! [`LoadedImage`] (from [`ImageStore`]) and the current [`Document`]. The
//! render is [`export_size_ok`] → `LoadedImage::to_rgba()` → [`render_export`]
//! → a PNG `Blob`. The size check comes first, so an image the renderer would
//! refuse (over 40 MP, or a side over the limit) gets the "too large" toast
//! without its pixels ever being copied: wasm memory never shrinks, so a
//! refused copy would cost its size for the rest of the session.
//! Nothing is cached: `to_rgba` peaks at about three copies of the image, so
//! holding a render between clicks would cost more than it saves.
//!
//! Nothing here makes a network request (CLAUDE.md rule 1). The PNG goes to
//! the clipboard, or to a download through a blob URL on an `<a download>`
//! element. The blob URL is never read back by the page.
//!
//! # Integration points
//!
//! - **Document (#10 canvas, #11 history):** [`Exporter::new`] takes a
//!   [`DocumentSource`]: a non-reactive accessor that returns the current
//!   document when an action runs. `App` passes the canvas editor's
//!   document (`CanvasState::controller` → `Controller::editor`); when it
//!   returns `None`, the export is the image plus the header-only caption of
//!   an empty document.
//!   A document whose image size doesn't match the current image is an
//!   error (shown in the toast), never silently replaced.
//! - **Keyboard (#13):** the central keymap ([`crate::keymap`]) binds the
//!   three shortcuts and calls [`Exporter::run`]. `⌘C` yields to text
//!   fields and page text selections; `⌘S` always suppresses the browser's
//!   "Save page".
//!
//! # Browser notes
//!
//! - **Copy image** builds the `ClipboardItem` synchronously in the gesture
//!   handler, with a `Promise<Blob>` value that resolves when the render
//!   finishes. Safari rejects `clipboard.write` with `NotAllowedError` if the
//!   render is awaited first (WebKit bug 222262). Chrome and Firefox (127 and
//!   later) accept the same shape.
//! - When the clipboard can't take an image (no `ClipboardItem`, insecure
//!   context, permission denied), the toast says why and offers Download.
//! - **Open issue** (embed only, [`issue`]) copies the image first and opens
//!   the new-issue tab once the copy has worked: a tab opened first would
//!   take the focus, and the clipboard refuses a page without it. A browser
//!   that blocks the late `window.open` gets a link in the toast instead.

pub mod issue;
pub mod rules;

use std::cell::RefCell;
use std::rc::Rc;

use js_sys::{Array, Object, Promise, Reflect, Uint8Array};
use leptos::{prelude::*, task::spawn_local};
use paste2prompt_core::{Caption, Document, ImageSize, RenderError, export_size_ok, render_export};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::{JsFuture, future_to_promise};
use web_sys::{Blob, BlobPropertyBag, Clipboard, ClipboardItem, HtmlAnchorElement};

use crate::image::{ImageStore, LoadedImage};
use crate::keymap::{self, rules::Action as KeyAction};
use issue::IssueTarget;
use rules::{Action, CopyFailure, Timestamp};

const PNG: &str = "image/png";
/// How long a plain status toast stays up.
const TOAST_MS: i32 = 4_000;
/// How long a toast that offers Download stays up.
const TOAST_OFFER_MS: i32 = 10_000;
/// How long a download's blob URL stays alive after the click. The browser
/// has started reading it by then; revoking frees the PNG's memory.
const REVOKE_AFTER_MS: i32 = 10_000;

/// Returns the current document when an export action runs, or `None` when
/// there is none yet (the export then uses an empty document). Copyable.
#[derive(Clone, Copy)]
pub struct DocumentSource(UnsyncCallback<(), Option<Document>>);

impl DocumentSource {
    /// Read the document with `get`. Called once per action, untracked.
    pub fn new(get: impl Fn() -> Option<Document> + 'static) -> Self {
        Self(UnsyncCallback::new(move |()| get()))
    }
}

/// A message in the export toast.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Toast {
    message: String,
    /// Show a **Download PNG** button (Copy image failed).
    offer_download: bool,
    /// A link to show, as `(label, href)`: the new issue, when its tab
    /// couldn't be opened.
    link: Option<(&'static str, String)>,
    /// Bumped per toast, so an old auto-dismiss timer can't close a new one.
    serial: u64,
}

/// The export controller. Cheap to copy into event handlers.
#[derive(Clone, Copy)]
pub struct Exporter {
    store: ImageStore,
    document: DocumentSource,
    /// `true` while an export renders. Actions are ignored meanwhile.
    busy: RwSignal<bool>,
    /// `true` while a large image renders, to show "Rendering…".
    busy_label: RwSignal<bool>,
    toast: RwSignal<Option<Toast>>,
    toast_serial: StoredValue<u64>,
}

impl Exporter {
    pub fn new(store: ImageStore, document: DocumentSource) -> Self {
        Self {
            store,
            document,
            busy: RwSignal::new(false),
            busy_label: RwSignal::new(false),
            toast: RwSignal::new(None),
            toast_serial: StoredValue::new(0),
        }
    }

    /// Run one export action. Must be called inside a user gesture (a click
    /// or a key press): the clipboard requires it.
    pub fn run(&self, action: Action) {
        match action {
            Action::CopyImage => self.copy_image(None),
            Action::Download => self.download(),
            Action::CopyCaption => self.copy_caption(),
        }
    }

    /// `true` when an image is loaded and no export is rendering.
    fn ready(&self) -> bool {
        !self.busy.get() && self.store.current().with(Option::is_some)
    }

    /// The image and document to export, read once at the gesture.
    fn inputs(&self) -> Option<(LoadedImage, Result<Document, String>)> {
        let image = self.store.current().get_untracked()?;
        let doc = match self.document.0.run(()) {
            Some(doc) => Ok(doc),
            None => ImageSize::new(image.width(), image.height())
                .map(Document::new)
                .map_err(|e| format!("Couldn't export: {e}.")),
        };
        Some((image, doc))
    }

    /// Open issue: copy the image, then open `target`'s new-issue page
    /// pre-filled from the caption (see module docs).
    pub fn open_issue(&self, target: &IssueTarget) {
        let Some((_, Ok(doc))) = self.inputs() else {
            return self.copy_image(None);
        };
        let caption = Caption::from_document(&doc).to_plain_text();
        self.copy_image(Some(issue::issue_url(target, doc.instructions(), &caption)));
    }

    /// Copy image: `clipboard.write` with a `ClipboardItem` whose value is a
    /// promise of the rendered PNG, all within this call (see module docs).
    /// With `issue`, opens that URL in a new tab once the copy is done.
    fn copy_image(&self, issue: Option<String>) {
        if self.busy.get_untracked() {
            return;
        }
        let Some((image, doc)) = self.inputs() else {
            return;
        };
        let clipboard = match image_clipboard() {
            Ok(c) => c,
            Err(failure) => return self.copy_failed(failure, issue),
        };
        let render_error: Rc<RefCell<Option<String>>> = Rc::default();
        let blob = {
            let this = *self;
            let render_error = Rc::clone(&render_error);
            future_to_promise(async move {
                match this.render(image, doc).await {
                    Ok(blob) => Ok(blob.into()),
                    Err(message) => {
                        *render_error.borrow_mut() = Some(message.clone());
                        Err(JsValue::from_str(&message))
                    }
                }
            })
        };
        let record = Object::new();
        let item = Reflect::set(&record, &PNG.into(), &blob)
            .and_then(|_| ClipboardItem::new_with_record_from_str_to_blob_promise(&record));
        let item = match item {
            Ok(item) => item,
            Err(_) => return self.copy_failed(CopyFailure::Unsupported, issue),
        };
        let write = clipboard.write(&Array::of1(&item));
        let this = *self;
        spawn_local(async move {
            let result = JsFuture::from(write).await;
            // `write` settles after the blob promise, so the render error (if
            // any) is already recorded.
            let render_error = render_error.borrow_mut().take();
            match (result, render_error, issue) {
                (Ok(_), _, None) => this.show("Image copied. Paste it into your chat.", false),
                (Ok(_), _, Some(url)) => {
                    if open_tab(&url) {
                        this.show("Image copied. Paste it into the new issue.", false);
                    } else {
                        this.show_link(
                            "Image copied. Open the new issue and paste it there.",
                            false,
                            url,
                        );
                    }
                }
                (Err(_), Some(message), _) => this.show(message, false),
                (Err(e), None, issue) => {
                    this.copy_failed(CopyFailure::from_exception_name(&exception_name(&e)), issue)
                }
            }
        });
    }

    /// Download PNG: render, then click an `<a download>` holding a blob URL.
    fn download(&self) {
        if self.busy.get_untracked() {
            return;
        }
        let Some((image, doc)) = self.inputs() else {
            return;
        };
        let this = *self;
        spawn_local(async move {
            let result = match this.render(image, doc).await {
                Ok(blob) => save_blob(&blob, &rules::file_name(now())),
                Err(message) => Err(message),
            };
            match result {
                Ok(()) => this.show("PNG downloaded.", false),
                Err(message) => this.show(message, false),
            }
        });
    }

    /// Copy caption: `clipboard.writeText` of the caption's plain text. The
    /// caption is cheap, so `writeText` is called directly in the gesture.
    fn copy_caption(&self) {
        let Some((_, doc)) = self.inputs() else {
            return;
        };
        let doc = match doc {
            Ok(doc) => doc,
            Err(message) => return self.show(message, false),
        };
        let Some(clipboard) = clipboard() else {
            let failure = if window().is_secure_context() {
                CopyFailure::Unsupported
            } else {
                CopyFailure::InsecureContext
            };
            return self.show(failure.message(), false);
        };
        let text = Caption::from_document(&doc).to_plain_text();
        let write = clipboard.write_text(&text);
        let this = *self;
        spawn_local(async move {
            match JsFuture::from(write).await {
                Ok(_) => this.show("Caption copied as text.", false),
                Err(e) => {
                    let failure = CopyFailure::from_exception_name(&exception_name(&e));
                    this.show(failure.message(), false);
                }
            }
        });
    }

    /// Render the export PNG as a `Blob`, with the busy state set meanwhile.
    async fn render(
        &self,
        image: LoadedImage,
        doc: Result<Document, String>,
    ) -> Result<Blob, String> {
        let doc = doc?;
        // Before `to_rgba`, so a refused image is never copied (#37).
        export_size_ok(image.width(), image.height(), &doc).map_err(render_failed)?;
        self.busy.set(true);
        self.busy_label
            .set(rules::shows_busy_label(image.width(), image.height()));
        // Let the busy state paint before the synchronous render blocks the
        // main thread.
        after_next_paint().await;
        let result = render_blob(&image, &doc);
        self.busy.set(false);
        self.busy_label.set(false);
        result
    }

    /// With `issue`, the toast also links the new issue, so the user can
    /// attach the downloaded PNG there.
    fn copy_failed(&self, failure: CopyFailure, issue: Option<String>) {
        let message = format!("{} Download the PNG instead?", failure.message());
        match issue {
            Some(url) => self.show_link(message, true, url),
            None => self.show(message, true),
        }
    }

    fn show(&self, message: impl Into<String>, offer_download: bool) {
        self.toast_with(message.into(), offer_download, None);
    }

    fn show_link(&self, message: impl Into<String>, offer_download: bool, issue_url: String) {
        self.toast_with(
            message.into(),
            offer_download,
            Some(("Open the new issue", issue_url)),
        );
    }

    fn toast_with(
        &self,
        message: String,
        offer_download: bool,
        link: Option<(&'static str, String)>,
    ) {
        let serial = self.toast_serial.with_value(|s| s + 1);
        self.toast_serial.set_value(serial);
        let lingers = offer_download || link.is_some();
        self.toast.set(Some(Toast {
            message,
            offer_download,
            link,
            serial,
        }));
        let ms = if lingers { TOAST_OFFER_MS } else { TOAST_MS };
        let toast = self.toast;
        spawn_local(async move {
            sleep(ms).await;
            toast.update(|t| {
                if t.as_ref().is_some_and(|t| t.serial == serial) {
                    *t = None;
                }
            });
        });
    }

    fn dismiss(&self) {
        self.toast.set(None);
    }

    /// `true` when an image is loaded (not tracked).
    pub fn has_image(&self) -> bool {
        self.store.current().with_untracked(Option::is_some)
    }
}

/// The export buttons and the toast. Mount once. Their shortcuts are in
/// [`crate::keymap`].
#[component]
pub fn ExportToolbar(exporter: Exporter) -> impl IntoView {
    let disabled = move || !exporter.ready();
    let toast = exporter.toast;
    // Embed only: the host's repository for Open issue.
    let issue = use_context::<IssueTarget>().map(|target| {
        let title = format!("Copy the image and open a new issue in {}", target.repo);
        view! {
            <button
                type="button"
                class="primary"
                aria-label="Open issue"
                title=title
                disabled=disabled
                on:click=move |_| exporter.open_issue(&target)
            >
                <span class="label-full">"Open issue"</span>
                <span class="label-short">"Issue"</span>
            </button>
        }
    });
    view! {
        <div class="export-toolbar" role="group" aria-label="Export" aria-busy=move || exporter.busy.get().to_string()>
            {issue}
            <button
                type="button"
                aria-label="Copy image"
                aria-keyshortcuts=keymap::aria(KeyAction::CopyImage)
                title=keymap::titled("Copy the export image", KeyAction::CopyImage)
                disabled=disabled
                on:click=move |_| exporter.run(Action::CopyImage)
            >
                // Narrow screens show the short label (style.css, "Header");
                // aria-label keeps the full name either way.
                <span class="label-full">"Copy image"</span>
                <span class="label-short">"Copy"</span>
            </button>
            <button
                type="button"
                aria-label="Download PNG"
                aria-keyshortcuts=keymap::aria(KeyAction::Download)
                title=keymap::titled("Download the export PNG", KeyAction::Download)
                disabled=disabled
                on:click=move |_| exporter.run(Action::Download)
            >
                <span class="label-full">"Download PNG"</span>
                <span class="label-short">"Download"</span>
            </button>
            <button
                type="button"
                aria-label="Copy caption"
                aria-keyshortcuts=keymap::aria(KeyAction::CopyCaption)
                title=keymap::titled("Copy the caption as text", KeyAction::CopyCaption)
                disabled=disabled
                on:click=move |_| exporter.run(Action::CopyCaption)
            >
                <span class="label-full">"Copy caption"</span>
                <span class="label-short">"Caption"</span>
            </button>
            <span class="export-busy" hidden=move || !exporter.busy_label.get()>
                "Rendering…"
            </span>
        </div>
        <div class="toast-region" role="status" aria-live="polite">
            {move || toast.get().map(|t| view! {
                <div class="toast">
                    <span>{t.message}</span>
                    {t.link.map(|(label, href)| view! {
                        <a href=href target="_blank" rel="noopener noreferrer">{label}</a>
                    })}
                    <Show when=move || t.offer_download>
                        <button type="button" class="primary" on:click=move |_| {
                            exporter.dismiss();
                            exporter.run(Action::Download);
                        }>
                            "Download PNG"
                        </button>
                    </Show>
                    <button
                        type="button"
                        class="toast-close"
                        aria-label="Dismiss"
                        on:click=move |_| exporter.dismiss()
                    >
                        "×"
                    </button>
                </div>
            })}
        </div>
    }
}

/// `image`'s pixels and `doc` → the export PNG as an `image/png` blob.
fn render_blob(image: &LoadedImage, doc: &Document) -> Result<Blob, String> {
    let rgba = image
        .to_rgba()
        .map_err(|_| "Couldn't read the image's pixels.".to_owned())?;
    let png = render_export(&rgba, doc).map_err(render_failed)?;
    drop(rgba);
    png_blob(&png).map_err(|_| "Couldn't create the PNG.".to_owned())
}

/// The toast for a render error, such as "too large".
fn render_failed(e: RenderError) -> String {
    format!("Couldn't render the export: {e}.")
}

fn png_blob(png: &[u8]) -> Result<Blob, JsValue> {
    let bytes = Uint8Array::from(png);
    let options = BlobPropertyBag::new();
    options.set_type(PNG);
    Blob::new_with_u8_array_sequence_and_options(&Array::of1(&bytes), &options)
}

/// Save `blob` as `name` through a temporary `<a download>`, then revoke
/// the blob URL once the browser has had time to start the download.
fn save_blob(blob: &Blob, name: &str) -> Result<(), String> {
    let fail = |_| "Couldn't start the download.".to_owned();
    let document = window().document().ok_or("No document.")?;
    let body = document.body().ok_or("No document body.")?;
    let url = web_sys::Url::create_object_url_with_blob(blob).map_err(fail)?;
    let anchor: HtmlAnchorElement = document.create_element("a").map_err(fail)?.unchecked_into();
    anchor.set_download(name);
    anchor.set_href(&url);
    anchor.set_hidden(true);
    // Firefox only follows a click on an anchor that is in the document.
    let appended = body.append_child(&anchor).map_err(fail);
    if appended.is_ok() {
        anchor.click();
        anchor.remove();
    }
    spawn_local(async move {
        sleep(REVOKE_AFTER_MS).await;
        let _ = web_sys::Url::revoke_object_url(&url);
    });
    appended.map(|_| ())
}

/// Opens `url` in a new tab, without giving it a handle back to this page.
/// `false` when the browser blocked it.
fn open_tab(url: &str) -> bool {
    // Opened blank, then sent on: `noopener` in the features would make
    // `open` return null, hiding whether it was blocked.
    let Ok(Some(tab)) = window().open_with_url_and_target("", "_blank") else {
        return false;
    };
    let _ = Reflect::set(&tab, &"opener".into(), &JsValue::NULL);
    tab.location().set_href(url).is_ok()
}

/// The async clipboard, when the page has one (secure contexts only).
fn clipboard() -> Option<Clipboard> {
    let navigator = window().navigator();
    let value = Reflect::get(&navigator, &"clipboard".into()).ok()?;
    (!value.is_undefined() && !value.is_null()).then(|| value.unchecked_into())
}

/// The clipboard, if it can take an image, or why not.
fn image_clipboard() -> Result<Clipboard, CopyFailure> {
    let window = window();
    if !window.is_secure_context() {
        return Err(CopyFailure::InsecureContext);
    }
    let has_item = Reflect::has(&window, &"ClipboardItem".into()).unwrap_or(false);
    let clipboard = clipboard().filter(|_| has_item);
    let can_write = clipboard
        .as_ref()
        .is_some_and(|c| Reflect::has(c, &"write".into()).unwrap_or(false));
    match clipboard {
        Some(c) if can_write => Ok(c),
        _ => Err(CopyFailure::Unsupported),
    }
}

/// The `name` of a rejected promise's `DOMException`, or `""`.
fn exception_name(e: &JsValue) -> String {
    Reflect::get(e, &"name".into())
        .ok()
        .and_then(|n| n.as_string())
        .unwrap_or_default()
}

/// `true` if the page has a non-empty text selection (a `Range`, not a
/// caret), so `⌘C` should copy the text instead.
pub(crate) fn page_text_selected() -> bool {
    window()
        .get_selection()
        .ok()
        .flatten()
        .is_some_and(|s| !s.is_collapsed() && s.type_() == "Range")
}

/// The local date and time.
fn now() -> Timestamp {
    let d = js_sys::Date::new_0();
    Timestamp {
        year: d.get_full_year(),
        month: d.get_month() + 1,
        day: d.get_date(),
        hour: d.get_hours(),
        minute: d.get_minutes(),
        second: d.get_seconds(),
    }
}

/// Resolve after `ms` milliseconds.
async fn sleep(ms: i32) {
    let promise = Promise::new(&mut |resolve, _| {
        let _ = window().set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms);
    });
    let _ = JsFuture::from(promise).await;
}

/// Resolve once the browser has painted the current state: the next
/// animation frame, then a macrotask after it.
async fn after_next_paint() {
    let frame = Promise::new(&mut |resolve, _| {
        let _ = window().request_animation_frame(&resolve);
    });
    let _ = JsFuture::from(frame).await;
    sleep(0).await;
}
