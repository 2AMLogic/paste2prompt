//! Layout watchers: tell the canvas to redraw when its CSS size or the
//! device pixel ratio changes, so the backing store and the
//! screen-to-image mapping never go stale.
//!
//! - A `ResizeObserver` on the canvas and its container catches every
//!   size change: a window resize, the panel un-hiding, a classic
//!   scrollbar appearing.
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
use web_sys::{Element, MediaQueryList, ResizeObserver};

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
