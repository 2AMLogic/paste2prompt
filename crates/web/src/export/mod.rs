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
//! - **Keyboard (#13):** the three shortcuts are registered by a small local
//!   `keydown` listener ([`ExportToolbar`] installs it). The decision is the
//!   pure [`rules::shortcut`]; when #13 builds the central keymap it should
//!   call `rules::shortcut` (or bind the same chords) and dispatch to
//!   [`Exporter::run`], then delete `Exporter::install_shortcuts`. The
//!   listener skips events another handler already `preventDefault`ed.
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

pub mod rules;

use std::cell::RefCell;
use std::rc::Rc;

use js_sys::{Array, Object, Promise, Reflect, Uint8Array};
use leptos::{ev, prelude::*, task::spawn_local};
use paste2prompt_core::{Caption, Document, ImageSize, RenderError, export_size_ok, render_export};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::{JsFuture, future_to_promise};
use web_sys::{Blob, BlobPropertyBag, Clipboard, ClipboardItem, HtmlAnchorElement, KeyboardEvent};

use crate::image::{ImageStore, LoadedImage};
use crate::input::target_is_editable;
use rules::{Action, CopyFailure, KeyContext, Mods, Timestamp};

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
            Action::CopyImage => self.copy_image(),
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

    /// Copy image: `clipboard.write` with a `ClipboardItem` whose value is a
    /// promise of the rendered PNG, all within this call (see module docs).
    fn copy_image(&self) {
        if self.busy.get_untracked() {
            return;
        }
        let Some((image, doc)) = self.inputs() else {
            return;
        };
        let clipboard = match image_clipboard() {
            Ok(c) => c,
            Err(failure) => return self.copy_failed(failure),
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
            Err(_) => return self.copy_failed(CopyFailure::Unsupported),
        };
        let write = clipboard.write(&Array::of1(&item));
        let this = *self;
        spawn_local(async move {
            let result = JsFuture::from(write).await;
            // `write` settles after the blob promise, so the render error (if
            // any) is already recorded.
            let render_error = render_error.borrow_mut().take();
            match (result, render_error) {
                (Ok(_), _) => this.show("Image copied. Paste it into your chat.", false),
                (Err(_), Some(message)) => this.show(message, false),
                (Err(e), None) => {
                    this.copy_failed(CopyFailure::from_exception_name(&exception_name(&e)))
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

    fn copy_failed(&self, failure: CopyFailure) {
        self.show(
            format!("{} Download the PNG instead?", failure.message()),
            true,
        );
    }

    fn show(&self, message: impl Into<String>, offer_download: bool) {
        let serial = self.toast_serial.with_value(|s| s + 1);
        self.toast_serial.set_value(serial);
        self.toast.set(Some(Toast {
            message: message.into(),
            offer_download,
            serial,
        }));
        let ms = if offer_download {
            TOAST_OFFER_MS
        } else {
            TOAST_MS
        };
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

    fn on_keydown(&self, ev: KeyboardEvent) {
        if ev.default_prevented() {
            return; // another handler (e.g. a future central keymap) took it
        }
        let mods = Mods {
            ctrl: ev.ctrl_key(),
            meta: ev.meta_key(),
            shift: ev.shift_key(),
            alt: ev.alt_key(),
        };
        let ctx = KeyContext {
            editable: target_is_editable(ev.target()),
            text_selected: page_text_selected(),
        };
        let Some(action) = rules::shortcut(&ev.key(), mods, ctx) else {
            return;
        };
        let has_image = self.store.current().with_untracked(Option::is_some);
        match action {
            // Never let ⌘S open the browser's "Save page" dialog.
            Action::Download => ev.prevent_default(),
            // Without an image, ⌘C and C keep their ordinary meaning.
            _ if !has_image => return,
            _ => ev.prevent_default(),
        }
        if has_image && !ev.repeat() {
            self.run(action);
        }
    }

    /// The local shortcut listener, for this component's lifetime. #13's
    /// central keymap replaces it (see module docs).
    fn install_shortcuts(&self) {
        let this = *self;
        let handle = window_event_listener(ev::keydown, move |e| this.on_keydown(e));
        on_cleanup(move || handle.remove());
    }
}

/// The export buttons and the toast, plus the `⌘C` / `⌘S` / `C` shortcuts.
/// Mount once.
#[component]
pub fn ExportToolbar(exporter: Exporter) -> impl IntoView {
    exporter.install_shortcuts();
    let disabled = move || !exporter.ready();
    let toast = exporter.toast;
    view! {
        <div class="export-toolbar" role="group" aria-label="Export" aria-busy=move || exporter.busy.get().to_string()>
            <button
                type="button"
                aria-keyshortcuts="Meta+C Control+C"
                title="Copy the export image (⌘C / Ctrl+C)"
                disabled=disabled
                on:click=move |_| exporter.run(Action::CopyImage)
            >
                "Copy image"
            </button>
            <button
                type="button"
                aria-keyshortcuts="Meta+S Control+S"
                title="Download the export PNG (⌘S / Ctrl+S)"
                disabled=disabled
                on:click=move |_| exporter.run(Action::Download)
            >
                "Download PNG"
            </button>
            <button
                type="button"
                aria-keyshortcuts="C"
                title="Copy the caption as text (C)"
                disabled=disabled
                on:click=move |_| exporter.run(Action::CopyCaption)
            >
                "Copy caption"
            </button>
            <span class="export-busy" hidden=move || !exporter.busy_label.get()>
                "Rendering…"
            </span>
        </div>
        <div class="toast-region" role="status" aria-live="polite">
            {move || toast.get().map(|t| view! {
                <div class="toast">
                    <span>{t.message}</span>
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
fn page_text_selected() -> bool {
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
