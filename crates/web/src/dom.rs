//! Page-level DOM lookups that also work when the app is embedded.
//!
//! The standalone app owns the whole document. The embed (`crate::embed`)
//! mounts the same app inside a shadow root, in a scrolling container, on
//! top of someone else's page. There:
//!
//! - `document().query_selector` and `document().active_element()` can't see
//!   into the shadow root, so lookups go through [`query_selector`] and
//!   [`active_element`];
//! - a window listener sees events from inside the shadow root retargeted to
//!   the host element, so the element the event really started on comes from
//!   [`event_origin`];
//! - the host page's scroll position means nothing to the app; the
//!   container's does ([`viewport`]).

use std::cell::RefCell;

use leptos::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{Element, Event, EventTarget, HtmlElement, ShadowRoot};

/// Where an embedded app lives: its shadow root and the element it scrolls in.
#[derive(Clone)]
pub(crate) struct EmbedRoot {
    pub(crate) shadow: ShadowRoot,
    pub(crate) scroller: HtmlElement,
}

thread_local! {
    static EMBED: RefCell<Option<EmbedRoot>> = const { RefCell::new(None) };
}

/// Set (or clear, `None`) the embed root. Only `crate::embed` calls this.
pub(crate) fn set_embed_root(root: Option<EmbedRoot>) {
    EMBED.with(|e| *e.borrow_mut() = root);
}

fn embed_root() -> Option<EmbedRoot> {
    EMBED.with(|e| e.borrow().clone())
}

/// The first element matching `selector` in the app.
pub(crate) fn query_selector(selector: &str) -> Option<Element> {
    match embed_root() {
        Some(root) => root.shadow.query_selector(selector),
        None => document().query_selector(selector),
    }
    .ok()
    .flatten()
}

/// The focused element, looking through shadow roots (the host page's
/// `document.activeElement` is only the shadow host).
pub(crate) fn active_element() -> Option<Element> {
    let mut el = document().active_element()?;
    while let Some(inner) = el.shadow_root().and_then(|s| s.active_element()) {
        el = inner;
    }
    Some(el)
}

/// The element an event started on. Unlike `Event::target`, this isn't
/// retargeted to the shadow host when a window listener sees an event from
/// inside the embed.
pub(crate) fn event_origin(ev: &Event) -> Option<EventTarget> {
    ev.composed_path()
        .get(0)
        .dyn_into::<EventTarget>()
        .ok()
        .or_else(|| ev.target())
}

/// The area the app's page scrolls in, in client coordinates.
pub(crate) struct Viewport {
    /// Top edge.
    pub(crate) top: f64,
    /// Visible height.
    pub(crate) height: f64,
    /// How far the page is scrolled.
    pub(crate) scroll: f64,
}

/// The window, or the embed's scrolling container.
pub(crate) fn viewport() -> Viewport {
    match embed_root() {
        Some(root) => Viewport {
            top: root.scroller.get_bounding_client_rect().top(),
            height: f64::from(root.scroller.client_height()),
            scroll: f64::from(root.scroller.scroll_top()),
        },
        None => {
            let window = window();
            let height = document().document_element().map_or_else(
                || {
                    window
                        .inner_height()
                        .ok()
                        .and_then(|h| h.as_f64())
                        .unwrap_or(0.0)
                },
                |e| f64::from(e.client_height()),
            );
            Viewport {
                top: 0.0,
                height,
                scroll: window.scroll_y().unwrap_or(0.0),
            }
        }
    }
}
