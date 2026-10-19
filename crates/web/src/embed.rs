//! The app mounted inside a host page, for `crates/embed`.
//!
//! The host gives a shadow root, a container in it that scrolls, the image
//! to start from and the elements picked from the page before it was
//! captured ([`Pick`]). The app's window listeners (shortcuts, paste, drop)
//! live only while it is mounted, so they never see the host page's keys
//! while the overlay is closed.

use std::any::Any;
use std::cell::Cell;
use std::rc::Rc;

use js_sys::Promise;
use leptos::prelude::*;
use paste2prompt_core::{ElementRef, Point, Region};
use web_sys::{Blob, HtmlElement, ShadowRoot};

use crate::dom::{self, EmbedRoot};
use crate::export::Exporter;
use crate::{App, Initial};

/// An element picked from the page: where it was in the capture and what
/// it was. The editor starts with a hint edit on it.
pub struct Pick {
    region: Region,
    element: ElementRef,
}

impl Pick {
    /// `min` and `max` are opposite corners of the element's box as
    /// fractions of the capture's width and height. A box that sticks out of
    /// the capture is cut to it; one with nothing inside it is rejected, as
    /// is an invalid element field (see [`ElementRef::new`]).
    pub fn new(
        min: (f64, f64),
        max: (f64, f64),
        selector: String,
        tag: String,
        role: Option<String>,
        label: Option<String>,
    ) -> Result<Self, String> {
        let corner = |(x, y): (f64, f64)| Point::new(x, y).clamped();
        let region = Region::rect(corner(min), corner(max))
            .map_err(|_| "the element isn't inside the capture".to_owned())?;
        let element = ElementRef::new(selector, tag, role, label).map_err(|e| e.to_string())?;
        Ok(Self { region, element })
    }
}

/// A mounted embed. Dropping it unmounts the app.
pub struct Embedded {
    handle: Option<Box<dyn Any>>,
    edits: Rc<Cell<bool>>,
    exporter: Rc<Cell<Option<Exporter>>>,
}

impl Embedded {
    /// Whether the user has made any edits, which closing would discard.
    pub fn has_edits(&self) -> bool {
        self.edits.get()
    }

    /// The caption as plain text and the image-level instructions, as
    /// **Copy caption** would copy them; `None` without an image.
    pub fn caption(&self) -> Option<(String, String)> {
        self.exporter.get()?.caption()
    }

    /// The document as JSON, picked elements included; `None` without an
    /// image.
    pub fn document_json(&self) -> Option<String> {
        self.exporter.get()?.document_json()
    }

    /// A promise of the export PNG blob, as **Copy image** renders it.
    /// Rejects with a message (no image, too large, …).
    pub fn render_png(&self) -> Promise {
        match self.exporter.get() {
            Some(exporter) => exporter.png_promise(),
            None => Promise::reject(&"The editor isn't ready.".into()),
        }
    }
}

/// Mounts the app into `container`, which must be inside `shadow` and is
/// the element the app scrolls in, with `image` loaded and a hint edit on
/// each of `picks`, in order.
///
/// One embed at a time: drop the previous one first.
pub fn mount(
    shadow: ShadowRoot,
    container: HtmlElement,
    image: Blob,
    name: Option<String>,
    picks: Vec<Pick>,
) -> Embedded {
    console_error_panic_hook::set_once();
    dom::set_embed_root(Some(EmbedRoot {
        shadow,
        scroller: container.clone(),
    }));
    let initial = Initial {
        image,
        name,
        picks: picks.into_iter().map(|p| (p.region, p.element)).collect(),
    };
    let edits = Rc::new(Cell::new(false));
    let report = Rc::clone(&edits);
    let exporter = Rc::new(Cell::new(None));
    let give = Rc::clone(&exporter);
    let handle = leptos::mount::mount_to(container, move || {
        view! { <App initial edits=report exporter=give /> }
    });
    Embedded {
        handle: Some(Box::new(handle)),
        edits,
        exporter,
    }
}

impl Drop for Embedded {
    fn drop(&mut self) {
        // Unmount while the root is still set, so cleanup finds it.
        self.handle.take();
        dom::set_embed_root(None);
    }
}
