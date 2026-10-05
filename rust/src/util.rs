//! Shared helpers: gettext wrapper, RAII timers, misc small utilities.
//!
//! Timers are the Rust replacement for the C code's `guint *_id` fields plus
//! scattered `if (id) g_source_remove(id)` calls: a [`Timer`] removes its
//! source automatically when dropped or overwritten, so disposal can never
//! leave a callback pointing at freed state.

use glib::SourceId;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

/// gettext domain for the plugin; the C shim binds the domain before the Rust
/// core runs, so only the lookup is needed here.
pub const GETTEXT_DOMAIN: &str = "zero-dock";

pub fn t(s: &str) -> glib::GString {
    glib::dgettext(Some(GETTEXT_DOMAIN), s)
}

pub const VERSION: &str = match option_env!("ZERO_DOCK_VERSION") {
    Some(v) => v,
    None => "0.3.0",
};

pub fn monotonic_us() -> i64 {
    glib::monotonic_time()
}

pub fn real_time_us() -> i64 {
    glib::real_time()
}

/// An owned GLib timeout that removes its live source when dropped or replaced.
///
/// GLib destroys a timeout automatically when its callback returns
/// `ControlFlow::Break`. `SourceId` itself does not become invalidatable, so
/// retaining only the id makes a later `g_source_remove()` hit a stale id and
/// emit a GLib critical. `live` is shared with the callback wrapper and flips
/// to false whenever the callback breaks, making cleanup idempotent.
#[derive(Default)]
pub struct Timer {
    id: Option<SourceId>,
    live: Option<Rc<Cell<bool>>>,
}

impl Timer {
    pub fn is_set(&self) -> bool {
        self.id.is_some() && self.live.as_ref().is_some_and(|live| live.get())
    }

    /// Replace any pending source with a new timeout.
    pub fn set_timeout(&mut self, ms: u64, mut f: impl FnMut() -> glib::ControlFlow + 'static) {
        self.clear();
        let live = Rc::new(Cell::new(true));
        let callback_live = live.clone();
        let id = glib::timeout_add_local(Duration::from_millis(ms), move || {
            let flow = f();
            if flow == glib::ControlFlow::Break {
                callback_live.set(false);
            }
            flow
        });
        self.id = Some(id);
        self.live = Some(live);
    }

    /// Replace any pending source with a new seconds-based timeout.
    pub fn set_timeout_seconds(
        &mut self,
        secs: u64,
        mut f: impl FnMut() -> glib::ControlFlow + 'static,
    ) {
        self.clear();
        let live = Rc::new(Cell::new(true));
        let callback_live = live.clone();
        let id = glib::timeout_add_seconds_local(secs as u32, move || {
            let flow = f();
            if flow == glib::ControlFlow::Break {
                callback_live.set(false);
            }
            flow
        });
        self.id = Some(id);
        self.live = Some(live);
    }

    /// Stop tracking without removing the source.
    ///
    /// This remains useful for callbacks that explicitly clear their own timer
    /// before returning `Break`, but it is no longer required for stale-id
    /// safety because the wrapper above observes `Break` automatically.
    pub fn take(&mut self) {
        self.id = None;
        self.live = None;
    }

    /// Remove the source immediately if it is still live.
    pub fn clear(&mut self) {
        let id = self.id.take();
        let live = self.live.take();
        if let (Some(id), Some(live)) = (id, live) {
            if live.replace(false) {
                unsafe {
                    glib::ffi::g_source_remove(id.as_raw());
                }
            }
        }
    }
}

impl Drop for Timer {
    fn drop(&mut self) {
        self.clear();
    }
}

pub type DockRef = Rc<RefCell<crate::dock::Dock>>;
pub type DockWeak = std::rc::Weak<RefCell<crate::dock::Dock>>;

/// Run `f` with mutable access to the [`crate::dock::Dock`] behind `rc`.
///
/// Returns `None` when the dock is already mutably borrowed (a re-entrant
/// callback) or has been disposed. Re-entrant callbacks are treated as
/// no-ops instead of aborting the process.
pub fn with_dock<T>(rc: &DockRef, f: impl FnOnce(&mut crate::dock::Dock) -> T) -> Option<T> {
    let mut borrow = rc.try_borrow_mut().ok()?;
    Some(f(&mut borrow))
}

/// Iterate a transfer-none `GList` of GObjects without taking ownership of
/// the list or its items. The caller keeps the list lifetime rules of the
/// underlying API.
pub unsafe fn glist_borrow_objects(list: *mut glib::ffi::GList) -> Vec<glib::Object> {
    // Transfer-none container: ref each item (released on drop); the list
    // nodes themselves stay owned by the source object.
    let mut out = Vec::new();
    let mut node = list;
    while !node.is_null() {
        let data = (*node).data as *mut glib::gobject_ffi::GObject;
        if !data.is_null() {
            out.push(glib::translate::FromGlibPtrNone::from_glib_none(data));
        }
        node = (*node).next;
    }
    out
}
