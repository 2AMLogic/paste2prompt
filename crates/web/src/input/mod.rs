//! Image input (spec §Input): paste, the file browser (**Open** button and
//! `⌘O` / `Ctrl+O`), and drag and drop. All three paths end in
//! [`ImageInput::load`], which asks before replacing an image that has edits,
//! decodes the blob and stores it in [`ImageStore`]. **Start over**
//! ([`ImageInput::start_over`]) asks the same way before clearing it.
//!
//! Files are read only through `Blob` / `createImageBitmap`. Nothing here
//! makes a network request (CLAUDE.md rule 1), which is why a dropped link or
//! a remote image URL is refused rather than fetched.

pub mod rules;

use leptos::{ev, html, prelude::*, task::spawn_local};
use paste2prompt_core::{Document, ImageSize, export_size_ok};
use wasm_bindgen::JsCast;
use web_sys::{
    Blob, ClipboardEvent, DataTransfer, DragEvent, Event, EventTarget, File, FileList, HtmlElement,
    HtmlInputElement,
};

use crate::image::{self, ImageStore};
use crate::keymap::{self, rules::Action};
use rules::{DropAction, Entry, PasteAction};

const REPLACE_PROMPT: &str = "Load the new image? It replaces the current image and its edits.";
const START_OVER_PROMPT: &str = "Start over? This removes the image and all its edits.";
const MSG_LOADING: &str = "Loading image…";
const MSG_PASTE_NO_IMAGE: &str =
    "The clipboard has no image. Copy an image or take a screenshot, then paste.";
const MSG_FILE_NOT_IMAGE: &str = "That file isn't an image.";
const MSG_DROP_NO_FILE: &str = "Drop an image file. Links and text can't be loaded.";
const MSG_TOO_LARGE_TO_EXPORT: &str =
    "This image is too large to export as a PNG. You can still mark it up and copy the caption.";

/// Controller shared by every input path. Cheap to copy into event handlers.
#[derive(Clone, Copy)]
pub struct ImageInput {
    store: ImageStore,
    /// Whether the current session has edits, so replacing it needs a
    /// confirmation. The edit history (#11) supplies the real signal.
    has_edits: Signal<bool>,
    file_input: NodeRef<html::Input>,
    /// The latest status or error line, shown in a live region.
    notice: RwSignal<Option<&'static str>>,
    /// `true` while files are dragged over the window (shows the overlay).
    dragging: RwSignal<bool>,
    /// `dragenter` minus `dragleave` count, so moving between child elements
    /// doesn't flicker the overlay.
    drag_depth: StoredValue<u32>,
    /// Bumped per load, so a slow decode can't overwrite a newer image.
    generation: StoredValue<u64>,
}

impl ImageInput {
    pub fn new(store: ImageStore, has_edits: Signal<bool>) -> Self {
        Self {
            store,
            has_edits,
            file_input: NodeRef::new(),
            notice: RwSignal::new(None),
            dragging: RwSignal::new(false),
            drag_depth: StoredValue::new(0),
            generation: StoredValue::new(0),
        }
    }

    /// Open the browser's file picker. Must run inside a user gesture
    /// (a click or a key press).
    pub fn open_picker(&self) {
        if let Some(input) = self.file_input.get_untracked() {
            input.click();
        }
    }

    /// The single load path: confirm if edits exist, decode, then store.
    pub fn load(&self, blob: Blob, name: Option<String>) {
        if self.has_edits.get_untracked() && !confirm(REPLACE_PROMPT) {
            return;
        }
        let generation = self.generation.with_value(|g| g + 1);
        self.generation.set_value(generation);
        self.notice.set(Some(MSG_LOADING));
        let this = *self;
        spawn_local(async move {
            let result = image::decode(&blob, name).await;
            if this.generation.get_value() != generation {
                // A newer load started while this one decoded; drop this one.
                if let Ok(stale) = result {
                    stale.bitmap().close();
                }
                return;
            }
            match result {
                Ok(img) => {
                    let notice =
                        (!exportable(img.width(), img.height())).then_some(MSG_TOO_LARGE_TO_EXPORT);
                    this.store.set(img);
                    this.notice.set(notice);
                }
                Err(err) => this.notice.set(Some(err.message())),
            }
        });
    }

    /// **Start over**: drop the image and its edits and return to the empty
    /// state, asking first if there are edits (like [`Self::load`]).
    /// Cancelling keeps everything. A decode still in flight is dropped too,
    /// so it can't bring an image back afterwards.
    pub fn start_over(&self) {
        if self.store.current().with_untracked(Option::is_none) {
            return;
        }
        if self.has_edits.get_untracked() && !confirm(START_OVER_PROMPT) {
            return;
        }
        self.generation.update_value(|g| *g += 1);
        self.notice.set(None);
        self.store.clear();
        // The button is gone with the image; give focus to the empty
        // state's Open button once it shows.
        request_animation_frame(|| {
            if let Some(el) = document()
                .query_selector(".empty-state button.primary")
                .ok()
                .flatten()
                .and_then(|e| e.dyn_into::<HtmlElement>().ok())
            {
                let _ = el.focus();
            }
        });
    }

    fn load_file(&self, file: File) {
        let name = Some(file.name()).filter(|n| !n.is_empty());
        self.load(file.into(), name);
    }

    fn on_paste(&self, ev: ClipboardEvent) {
        let Some(data) = ev.clipboard_data() else {
            return;
        };
        let editable = target_is_editable(ev.target());
        let entries = clipboard_entries(&data);
        let borrowed: Vec<Entry<'_>> = entries
            .iter()
            .map(|(is_file, mime)| Entry {
                is_file: *is_file,
                mime,
            })
            .collect();
        match rules::paste_action(&borrowed, editable) {
            PasteAction::Ignore => {}
            PasteAction::NotImage => self.notice.set(Some(MSG_PASTE_NO_IMAGE)),
            PasteAction::Load(index) => {
                ev.prevent_default();
                match clipboard_file(&data, index) {
                    Some(file) => self.load_file(file),
                    None => self.notice.set(Some(MSG_PASTE_NO_IMAGE)),
                }
            }
        }
    }

    /// Load the first image in `files`, or explain why there is none.
    fn load_first_image(&self, files: Option<FileList>) {
        let files = files.map(file_vec).unwrap_or_default();
        let mimes: Vec<String> = files.iter().map(|f| f.type_()).collect();
        let entries: Vec<Entry<'_>> = mimes
            .iter()
            .map(|mime| Entry {
                is_file: true,
                mime,
            })
            .collect();
        match rules::drop_action(&entries) {
            DropAction::Load(index) => self.load_file(files[index].clone()),
            DropAction::NotImage => self.notice.set(Some(MSG_FILE_NOT_IMAGE)),
            DropAction::NoFiles => self.notice.set(Some(MSG_DROP_NO_FILE)),
        }
    }

    fn on_picked(&self, ev: Event) {
        let Some(input) = ev
            .target()
            .and_then(|t| t.dyn_into::<HtmlInputElement>().ok())
        else {
            return;
        };
        let files = input.files();
        let picked_any = files.as_ref().is_some_and(|f| f.length() > 0);
        if picked_any {
            self.load_first_image(files);
        }
        // Reset so choosing the same file again still fires `change`.
        input.set_value("");
    }

    fn on_drag_enter(&self, ev: DragEvent) {
        if carries_files(&ev) {
            ev.prevent_default();
            self.drag_depth.update_value(|d| *d += 1);
            self.dragging.set(true);
        }
    }

    fn on_drag_over(&self, ev: DragEvent) {
        if carries_files(&ev) || !target_is_editable(ev.target()) {
            // Cancelling `dragover` is what makes the window a drop target.
            // Text dragged over a text field keeps its default behaviour.
            ev.prevent_default();
            if let Some(data) = ev.data_transfer() {
                data.set_drop_effect("copy");
            }
        }
    }

    fn on_drag_leave(&self, _ev: DragEvent) {
        if self.dragging.get_untracked() {
            self.drag_depth.update_value(|d| *d = d.saturating_sub(1));
            if self.drag_depth.get_value() == 0 {
                self.dragging.set(false);
            }
        }
    }

    fn on_drop(&self, ev: DragEvent) {
        self.drag_depth.set_value(0);
        self.dragging.set(false);
        if !carries_files(&ev) && target_is_editable(ev.target()) {
            return; // ordinary text drop into a field
        }
        // Otherwise the browser would navigate to the file or link.
        ev.prevent_default();
        self.load_first_image(ev.data_transfer().and_then(|d| d.files()));
    }

    /// Register the window-level listeners for this component's lifetime.
    fn install_listeners(&self) {
        let this = *self;
        let handles = [
            window_event_listener(ev::paste, move |e| this.on_paste(e)),
            window_event_listener(ev::dragenter, move |e| this.on_drag_enter(e)),
            window_event_listener(ev::dragover, move |e| this.on_drag_over(e)),
            window_event_listener(ev::dragleave, move |e| this.on_drag_leave(e)),
            window_event_listener(ev::drop, move |e| this.on_drop(e)),
        ];
        on_cleanup(move || handles.into_iter().for_each(|h| h.remove()));
    }
}

/// The hidden file input, the drop overlay and the status line, plus the
/// window listeners for paste and drag and drop. Mount once. `⌘O` /
/// `Ctrl+O` is bound in [`crate::keymap`], which calls
/// [`ImageInput::open_picker`].
#[component]
pub fn ImageInputLayer(input: ImageInput) -> impl IntoView {
    input.install_listeners();
    view! {
        <input
            type="file"
            accept="image/*"
            hidden=true
            tabindex="-1"
            aria-hidden="true"
            node_ref=input.file_input
            on:change=move |ev| input.on_picked(ev)
        />
        <div class="drop-overlay" class:visible=move || input.dragging.get() aria-hidden="true">
            <p>"Drop an image to load it"</p>
        </div>
        <p class="notice" role="status" aria-live="polite">
            {move || input.notice.get()}
        </p>
    }
}

/// A button that opens the file picker.
#[component]
pub fn OpenButton(input: ImageInput, #[prop(optional)] class: &'static str) -> impl IntoView {
    view! {
        <button
            type="button"
            class=class
            aria-keyshortcuts=keymap::aria(Action::OpenFile)
            title=keymap::titled("Open an image", Action::OpenFile)
            on:click=move |_| input.open_picker()
        >
            "Open…"
        </button>
    }
}

/// The header's **Start over** button: drops the image and its edits (see
/// [`ImageInput::start_over`]). Disabled with no image. No shortcut, on
/// purpose.
#[component]
pub fn StartOverButton(input: ImageInput) -> impl IntoView {
    let current = input.store.current();
    view! {
        <button
            type="button"
            class="start-over"
            data-action="start-over"
            aria-label="Start over: remove the image and all edits"
            title="Start over: remove the image and all edits"
            disabled=move || current.with(Option::is_none)
            on:click=move |_| input.start_over()
        >
            "Start over"
        </button>
    }
}

/// `true` if an export of a `width × height` image with no edits yet would
/// render. Only a warning at load; the export re-checks with the real
/// document.
fn exportable(width: u32, height: u32) -> bool {
    ImageSize::new(width, height)
        .is_ok_and(|size| export_size_ok(width, height, &Document::new(size)).is_ok())
}

fn confirm(message: &str) -> bool {
    window().confirm_with_message(message).unwrap_or(false)
}

/// `(is_file, mime)` for each clipboard item. Falls back to `files` for
/// browsers that expose pasted files only there.
fn clipboard_entries(data: &DataTransfer) -> Vec<(bool, String)> {
    let items = data.items();
    if items.length() > 0 {
        (0..items.length())
            .filter_map(|i| items.get(i))
            .map(|item| (item.kind() == "file", item.type_()))
            .collect()
    } else {
        data.files()
            .map(file_vec)
            .unwrap_or_default()
            .iter()
            .map(|f| (true, f.type_()))
            .collect()
    }
}

/// The file behind entry `index` of [`clipboard_entries`].
fn clipboard_file(data: &DataTransfer, index: usize) -> Option<File> {
    let index = u32::try_from(index).ok()?;
    let items = data.items();
    if items.length() > 0 {
        items.get(index)?.get_as_file().ok().flatten()
    } else {
        data.files()?.get(index)
    }
}

fn file_vec(list: FileList) -> Vec<File> {
    (0..list.length()).filter_map(|i| list.get(i)).collect()
}

/// `true` if the drag carries files (the `"Files"` type), not text or links.
fn carries_files(ev: &DragEvent) -> bool {
    ev.data_transfer().is_some_and(|d| {
        d.types()
            .iter()
            .any(|t| t.as_string().as_deref() == Some("Files"))
    })
}

/// `true` for text fields and `contenteditable` elements, where pasting or
/// dropping text has its own meaning.
pub(crate) fn target_is_editable(target: Option<EventTarget>) -> bool {
    let Some(el) = target.and_then(|t| t.dyn_into::<HtmlElement>().ok()) else {
        return false;
    };
    matches!(el.tag_name().as_str(), "INPUT" | "TEXTAREA") || el.is_content_editable()
}
