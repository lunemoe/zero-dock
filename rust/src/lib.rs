//! Zero Dock XFCE panel plugin core — Rust implementation.
//!
//! There is no C code: the XFCE panel module ABI (`xfce_panel_module_construct`)
//! is exported directly from this crate's cdylib (see `plugin_abi.rs`), and
//! `unsafe` is confined to that module, the `ffi_*` modules and the XComposite
//! capture path. All business logic is safe Rust.

mod apps;
mod associations;
mod audio;
mod button;
mod dock;
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
    pub unsafe fn construct(plugin: *mut glib::gobject_ffi::GObject) -> Option<DockRef> {
        crate::dock::construct(crate::ffi_xfce::XfcePanelPlugin(plugin))
    }

    pub fn button_for_xid(dock: &Dock, xid: u64) -> Option<u64> {
        dock.buttons
            .iter()
            .find(|button| button.window_xid() == xid)
            .map(|button| button.id)
    }

    pub fn workspaces(screen: &Screen) -> Vec<glib::Object> {
        let manager = screen.workspace_manager();
        unsafe {
            crate::util::glist_borrow_objects(
                crate::ffi_xfce::xfw_workspace_manager_list_workspaces(
                    glib::translate::ToGlibPtr::to_glib_none(&manager).0,
                ),
            )
        }
    }

    pub fn association_key(window: &glib::Object) -> Option<String> {
        crate::associations::association_key(window)
    }

    pub fn xdisplay() -> *mut x11::xlib::Display {
        crate::ffi_x11::xdisplay()
    }

    pub fn gdk_xid(window: &gdk::Window) -> u64 {
        extern "C" {
            fn gdk_x11_window_get_xid(window: *mut gdk::ffi::GdkWindow) -> std::os::raw::c_ulong;
        }
        unsafe {
            use glib::translate::ToGlibPtr;
            gdk_x11_window_get_xid(window.to_glib_none().0) as u64
        }
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
