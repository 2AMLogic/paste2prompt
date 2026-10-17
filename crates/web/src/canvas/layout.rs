//! Layout watchers: tell the canvas to re-lay out and redraw when its
//! space or the device pixel ratio changes, so the stage size, the backing
//! store and the screen-to-image mapping never go stale. Also the DOM side
//! of `arrange`: measuring the space and sizing the stage.
//!
//! - A `ResizeObserver` on the canvas, its column, the instructions box and
//!   the workspace catches every size change: a window resize, the panel
//!   un-hiding, a classic scrollbar appearing, the instructions box being
//!   resized. (A window
//!   that only gets taller changes none of these; the canvas view listens
//!   for the window's `resize` event for that.)
//! - A `matchMedia("(resolution: <dpr>dppx)")` listener catches a DPR
//!   change (browser zoom, or the window moving to another monitor). The
//!   query only matches the current DPR, so it is re-armed with the new
//!   value each time it fires.
//!
//! Both only bump a counter; the draw effect re-reads size and DPR itself.

use std::cell::RefCell;
use std::rc::Rc;

use leptos::logging::warn;
use leptos::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use web_sys::{Element, HtmlElement, MediaQueryList, ResizeObserver};

use super::arrange::Space;

/// The page's bottom padding (`body { padding: 1rem }`), kept free under
/// the canvas column.
const PAGE_BOTTOM: f64 = 16.0;

/// Measures the space the workspace has, and the instructions box's
/// height (`arrange` decides where that height is spent). `None` while the
/// workspace isn't laid out (hidden).
pub fn measure_space(workspace: &Element, instructions: Option<&Element>) -> Option<Space> {
    let ws = workspace.get_bounding_client_rect();
    if ws.width() < 1.0 {
        return None;
    }
    let viewport = crate::dom::viewport();
    // Page coordinates, so the result doesn't depend on the scroll position.
    let top = ws.top() - viewport.top + viewport.scroll;
    let instructions_h = instructions.map_or(0.0, |e| e.get_bounding_client_rect().height());
    Some(Space {
        width: ws.width().floor(),
        height: (viewport.height - top - PAGE_BOTTOM).floor(),
        instructions_h: instructions_h.ceil(),
    })
}

/// Sets the stage's CSS content size, in whole CSS pixels. Through the
/// CSSOM, which the CSP (`style-src 'self'`) allows, unlike a `style`
/// attribute in markup.
pub fn size_stage(stage: &HtmlElement, width: u32, height: u32) {
    let style = stage.style();
    for (prop, len) in [("width", width), ("height", height)] {
        let value = format!("{len}px");
        if style.get_property_value(prop).ok().as_deref() != Some(value.as_str())
            && let Err(e) = style.set_property(prop, &value)
        {
            warn!("stage {prop} not set: {e:?}");
        }
    }
}

/// Observes `targets` for size changes, calling `on_change` for each.
/// Stops when dropped.
pub struct SizeWatch {
    observer: ResizeObserver,
    _callback: Closure<dyn FnMut()>,
}

impl SizeWatch {
    /// Starts observing. `None` if the browser has no `ResizeObserver`.
    pub fn new(targets: &[&Element], on_change: impl Fn() + 'static) -> Option<Self> {
        let callback = Closure::<dyn FnMut()>::new(on_change);
        let observer = match ResizeObserver::new(callback.as_ref().unchecked_ref()) {
            Ok(o) => o,
            Err(e) => {
                warn!("ResizeObserver unavailable: {e:?}");
                return None;
            }
        };
        for target in targets {
            observer.observe(target);
        }
        Some(Self {
            observer,
            _callback: callback,
        })
    }
}

impl Drop for SizeWatch {
    fn drop(&mut self) {
        self.observer.disconnect();
    }
}

#[derive(Default)]
struct DprState {
    query: RefCell<Option<MediaQueryList>>,
    listener: RefCell<Option<js_sys::Function>>,
}

impl DprState {
    fn disarm(&self) {
        if let (Some(query), Some(listener)) = (
            self.query.borrow_mut().take(),
            self.listener.borrow().as_ref(),
        ) {
            let _ = query.remove_event_listener_with_callback("change", listener);
        }
    }

    /// Listens for the DPR to stop being the current one.
    fn arm(&self) {
        self.disarm();
        let window = window();
        let query = format!("(resolution: {}dppx)", window.device_pixel_ratio());
        let Ok(Some(list)) = window.match_media(&query) else {
            return;
        };
        if let Some(listener) = self.listener.borrow().as_ref()
            && list
                .add_event_listener_with_callback("change", listener)
                .is_ok()
        {
            *self.query.borrow_mut() = Some(list);
        }
    }
}

/// Calls `on_change` whenever the device pixel ratio changes. Stops when
/// dropped.
pub struct DprWatch {
    state: Rc<DprState>,
    // Kept alive for the listener; it never drops itself, so re-arming from
    // inside the callback is safe.
    _callback: Closure<dyn FnMut()>,
}

impl DprWatch {
    /// Starts watching.
    pub fn new(on_change: impl Fn() + 'static) -> Self {
        let state = Rc::new(DprState::default());
        let rearm = Rc::clone(&state);
        let callback = Closure::<dyn FnMut()>::new(move || {
            rearm.arm();
            on_change();
        });
        *state.listener.borrow_mut() = Some(
            callback
                .as_ref()
                .unchecked_ref::<js_sys::Function>()
                .clone(),
        );
        state.arm();
        Self {
            state,
            _callback: callback,
        }
    }
}

impl Drop for DprWatch {
    fn drop(&mut self) {
        self.state.disarm();
        // The closure is freed right after this; drop the JS handle to it.
        self.state.listener.borrow_mut().take();
    }
}
