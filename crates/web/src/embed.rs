//! The app mounted inside a host page, for `crates/embed`.
//!
//! The host gives a shadow root, a container in it that scrolls, and the
//! image to start from. The app's window listeners (shortcuts, paste, drop)
//! live only while it is mounted, so they never see the host page's keys
//! while the overlay is closed.

use std::any::Any;
use std::cell::Cell;
use std::rc::Rc;

use leptos::prelude::*;
use web_sys::{Blob, HtmlElement, ShadowRoot};

use crate::App;
use crate::dom::{self, EmbedRoot};

/// A mounted embed. Dropping it unmounts the app.
pub struct Embedded {
    handle: Option<Box<dyn Any>>,
    edits: Rc<Cell<bool>>,
}

impl Embedded {
    /// Whether the user has made any edits, which closing would discard.
    pub fn has_edits(&self) -> bool {
        self.edits.get()
    }
}

/// Mounts the app into `container`, which must be inside `shadow` and is
/// the element the app scrolls in, with `image` loaded.
///
/// One embed at a time: drop the previous one first.
pub fn mount(
    shadow: ShadowRoot,
    container: HtmlElement,
    image: Blob,
    name: Option<String>,
) -> Embedded {
    console_error_panic_hook::set_once();
    dom::set_embed_root(Some(EmbedRoot {
        shadow,
        scroller: container.clone(),
    }));
    let initial = (image, name);
    let edits = Rc::new(Cell::new(false));
    let report = Rc::clone(&edits);
    let handle = leptos::mount::mount_to(container, move || view! { <App initial edits=report /> });
    Embedded {
        handle: Some(Box::new(handle)),
        edits,
    }
}

impl Drop for Embedded {
    fn drop(&mut self) {
        // Unmount while the root is still set, so cleanup finds it.
        self.handle.take();
        dom::set_embed_root(None);
    }
}
