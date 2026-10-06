//! Zero Dock XFCE panel plugin core — Rust implementation.
//!
//! There is no C code: the XFCE panel module ABI (`xfce_panel_module_construct`)
//! is exported directly from this crate's cdylib (see `plugin_abi.rs`), and
//! `unsafe` is confined to four modules:
//!
//! * [`ffi_xfce`] — libxfce4panel / libxfce4windowing bindings,
//! * [`ffi_x11`] — Xlib / XComposite / XInput2 and the GDK X11 glue,
//! * [`ffi_glib`] — GLib callbacks and borrowed `GList` walks,
//! * [`ffi_gtk`] — the GTK/GDK/GIO helpers the bindings omit,
//!
//! plus the [`plugin_abi`] module that implements the panel module ABI itself.
//! `#![deny(unsafe_code)]` below makes that a compile-time guarantee rather
//! than a comment: everything else in the crate is safe Rust, and the unsafe
//! modules hand out safe wrappers.

#![deny(unsafe_code)]

mod apps;
mod associations;
mod audio;
mod button;
mod dock;
mod ffi_glib;
mod ffi_gtk;
mod ffi_x11;
mod ffi_xfce;
mod input;
mod layout;
mod menu;
mod plugin_abi;
mod preview;
mod settings;
mod util;

/// Construct the dock inside an existing `XfcePanelPlugin` GObject.
/// Used by the module ABI and by the Rust test host.
///
/// # Safety
/// `plugin` must point to a valid `XfcePanelPlugin` GObject on the GTK main
/// thread; the pointer is borrowed for the duration of the call.
#[allow(unsafe_code)]
pub unsafe fn construct_plugin(plugin: *mut glib::gobject_ffi::GObject) {
    plugin_abi::construct_plugin(plugin)
}

#[cfg(feature = "test-harness")]
pub mod test_api {
    pub use crate::audio::{Audio, AudioRef, AudioStatus, Stream};
    pub use crate::button::{Button, ScrollInfo};
    pub use crate::dock::{Dock, DockRef};
    pub use crate::ffi_xfce::{Screen, Window as XfwWindow, Workspace as XfwWorkspace};
    pub use crate::input::Input;
    pub use crate::settings::Settings;

    /// Construct a real dock and return the strong test handle while preserving
    /// the production ownership hand-off to the plugin GObject.
    ///
    /// # Safety
    /// plugin must be a live XfcePanelPlugin on the GTK main thread.
    #[allow(unsafe_code)]
    pub unsafe fn construct(plugin: *mut glib::gobject_ffi::GObject) -> Option<DockRef> {
        crate::dock::construct(crate::ffi_xfce::XfcePanelPlugin(plugin))
    }

    pub fn button_for_xid(dock: &Dock, xid: u64) -> Option<u64> {
        dock.buttons
            .iter()
            .find(|button| button.window_xid() == xid)
            .map(|button| button.id)
    }

    /// Destroy a widget the scenario is holding; see [`crate::ffi_gtk::destroy`].
    pub fn destroy_widget(widget: &impl glib::prelude::IsA<gtk::Widget>) {
        crate::ffi_gtk::destroy(widget)
    }

    pub fn workspaces(screen: &Screen) -> Vec<glib::Object> {
        screen.workspaces()
    }

    pub fn association_key(window: &glib::Object) -> Option<String> {
        crate::associations::association_key(window)
    }

    pub fn xdisplay() -> *mut x11::xlib::Display {
        crate::ffi_x11::xdisplay()
    }

    pub fn gdk_xid(window: &gdk::Window) -> u64 {
        crate::ffi_x11::window_xid(window)
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod smoke {
    #[test]
    fn version_present() {
        assert!(!crate::util::VERSION.is_empty());
    }
}
