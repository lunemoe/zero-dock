//! FFI declarations and thin safe wrappers for libxfce4panel and
//! libxfce4windowing. Neither library ships GObject-Introspection based Rust
//! bindings, so the dock talks to them through hand-written declarations.
//!
//! All `Xfw*` objects are plain GObjects, so they are carried around as
//! [`glib::Object`] values (real reference-counted handles, dropped via
//! `Drop`) and cast down to the raw pointers the C functions expect. That
//! keeps every window/workspace handle memory-safe without writing a custom
//! ref-count wrapper.

use glib::ffi::gboolean;
use glib::gobject_ffi::GObject;
use glib::translate::*;
use std::os::raw::{c_char, c_int, c_ulong};

// ---------------------------------------------------------------------------
// XfcePanelPlugin
// ---------------------------------------------------------------------------

#[repr(transparent)]
#[derive(Clone, Copy, Debug)]
pub struct XfcePanelPlugin(pub *mut GObject);

impl XfcePanelPlugin {
    pub fn as_object(&self) -> glib::Object {
        // Takes its own reference (released on drop); the plugin object
        // outlives this wrapper.
        unsafe { glib::Object::from_glib_none(self.0) }
    }
    fn ptr(&self) -> *mut GObject {
        self.0
    }
}

extern "C" {
    pub fn xfce_panel_plugin_get_size(plugin: *mut GObject) -> c_int;
    pub fn xfce_panel_plugin_get_nrows(plugin: *mut GObject) -> c_int;
    pub fn xfce_panel_plugin_get_icon_size(plugin: *mut GObject) -> std::os::raw::c_uint;
    pub fn xfce_panel_plugin_get_orientation(plugin: *mut GObject) -> c_int;
    pub fn xfce_panel_plugin_set_expand(plugin: *mut GObject, expand: gboolean);
    pub fn xfce_panel_plugin_set_shrink(plugin: *mut GObject, shrink: gboolean);
    pub fn xfce_panel_plugin_add_action_widget(
        plugin: *mut GObject,
        widget: *mut gtk::ffi::GtkWidget,
    );
    pub fn xfce_panel_plugin_take_window(plugin: *mut GObject, window: *mut gtk::ffi::GtkWindow);
    pub fn xfce_panel_plugin_menu_show_configure(plugin: *mut GObject);
    pub fn xfce_panel_plugin_menu_show_about(plugin: *mut GObject);
    pub fn xfce_panel_plugin_menu_insert_item(
        plugin: *mut GObject,
        item: *mut gtk::ffi::GtkMenuItem,
    );
    pub fn xfce_panel_plugin_save_location(plugin: *mut GObject, create: gboolean) -> *mut c_char;
    pub fn xfce_panel_plugin_get_arguments(plugin: *mut GObject) -> *mut *mut c_char;
    pub fn xfce_panel_plugin_block_autohide(plugin: *mut GObject, blocked: gboolean);
    pub fn xfce_panel_plugin_popup_menu(
        plugin: *mut GObject,
        menu: *mut gtk::ffi::GtkMenu,
        widget: *mut gtk::ffi::GtkWidget,
        trigger_event: *mut gdk::ffi::GdkEvent,
    );
}

impl XfcePanelPlugin {
    pub fn size(&self) -> i32 {
        unsafe { xfce_panel_plugin_get_size(self.ptr()) }
    }
    pub fn nrows(&self) -> i32 {
        unsafe { xfce_panel_plugin_get_nrows(self.ptr()) }
    }
    pub fn icon_size(&self) -> i32 {
        unsafe { xfce_panel_plugin_get_icon_size(self.ptr()) as i32 }
    }
    pub fn orientation(&self) -> gtk::Orientation {
        unsafe { gtk::Orientation::from_glib(xfce_panel_plugin_get_orientation(self.ptr())) }
    }
    pub fn set_expand(&self, expand: bool) {
        unsafe { xfce_panel_plugin_set_expand(self.ptr(), expand.into_glib()) }
    }
    pub fn set_shrink(&self, shrink: bool) {
        unsafe { xfce_panel_plugin_set_shrink(self.ptr(), shrink.into_glib()) }
    }
    pub fn add_action_widget(&self, widget: &impl glib::IsA<gtk::Widget>) {
        unsafe { xfce_panel_plugin_add_action_widget(self.ptr(), widget.as_ref().to_glib_none().0) }
    }
    pub fn take_window(&self, window: &impl glib::IsA<gtk::Window>) {
        unsafe { xfce_panel_plugin_take_window(self.ptr(), window.as_ref().to_glib_none().0) }
    }
    pub fn menu_show_configure(&self) {
        unsafe { xfce_panel_plugin_menu_show_configure(self.ptr()) }
    }
    pub fn menu_show_about(&self) {
        unsafe { xfce_panel_plugin_menu_show_about(self.ptr()) }
    }
    pub fn menu_insert_item(&self, item: &impl glib::IsA<gtk::MenuItem>) {
        unsafe { xfce_panel_plugin_menu_insert_item(self.ptr(), item.as_ref().to_glib_none().0) }
    }
    /// Returns the save location (transfer full) as an owned string.
    pub fn save_location(&self, create: bool) -> Option<String> {
        unsafe {
            let p = xfce_panel_plugin_save_location(self.ptr(), create.into_glib());
            if p.is_null() {
                None
            } else {
                Some(glib::GString::from_glib_full(p).to_string())
            }
        }
    }
    /// Command line arguments (transfer none).
    pub fn arguments(&self) -> Vec<String> {
        unsafe {
            let argv = xfce_panel_plugin_get_arguments(self.ptr());
            let mut out = Vec::new();
            if argv.is_null() {
                return out;
            }
            let mut i = 0;
            loop {
                let s = *argv.offset(i);
                if s.is_null() {
                    break;
                }
                out.push(std::ffi::CStr::from_ptr(s).to_string_lossy().into_owned());
                i += 1;
            }
            out
        }
    }
    pub fn block_autohide(&self, blocked: bool) {
        unsafe { xfce_panel_plugin_block_autohide(self.ptr(), blocked.into_glib()) }
    }
    pub fn popup_menu(
        &self,
        menu: &impl glib::IsA<gtk::Menu>,
        widget: Option<&impl glib::IsA<gtk::Widget>>,
        event: Option<&gdk::Event>,
    ) {
        unsafe {
            xfce_panel_plugin_popup_menu(
                self.ptr(),
                menu.as_ref().to_glib_none().0,
                widget
                    .map(|w| w.as_ref().to_glib_none().0)
                    .unwrap_or(std::ptr::null_mut()),
                event
                    .map(|e| e.to_glib_none().0)
                    .unwrap_or(std::ptr::null_mut()),
            )
        }
    }
}

// ---------------------------------------------------------------------------
// Xfw constants (from libxfce4windowing headers)
// ---------------------------------------------------------------------------

pub type XfwWindowType = c_int;
pub const XFW_WINDOW_TYPE_DESKTOP: XfwWindowType = 1;
pub const XFW_WINDOW_TYPE_DOCK: XfwWindowType = 2;

pub const XFW_WORKSPACE_STATE_ACTIVE: u32 = 1 << 0;

pub const XFW_WINDOW_CAPABILITIES_CAN_MINIMIZE: u32 = 1 << 0;
pub const XFW_WINDOW_CAPABILITIES_CAN_UNMINIMIZE: u32 = 1 << 1;
pub const XFW_WINDOW_CAPABILITIES_CAN_MAXIMIZE: u32 = 1 << 2;
pub const XFW_WINDOW_CAPABILITIES_CAN_UNMAXIMIZE: u32 = 1 << 3;
pub const XFW_WINDOW_CAPABILITIES_CAN_FULLSCREEN: u32 = 1 << 4;
pub const XFW_WINDOW_CAPABILITIES_CAN_UNFULLSCREEN: u32 = 1 << 5;
pub const XFW_WINDOW_CAPABILITIES_CAN_PLACE_ABOVE: u32 = 1 << 10;
pub const XFW_WINDOW_CAPABILITIES_CAN_UNPLACE_ABOVE: u32 = 1 << 11;
pub const XFW_WINDOW_CAPABILITIES_CAN_CHANGE_WORKSPACE: u32 = 1 << 14;

// ---------------------------------------------------------------------------
// Xfw raw declarations
// ---------------------------------------------------------------------------

pub type XfwWindowPtr = *mut GObject;

extern "C" {
    pub fn xfw_screen_get_default() -> *mut GObject;
    pub fn xfw_screen_get_windows(screen: *mut GObject) -> *mut glib::ffi::GList;
    pub fn xfw_screen_get_active_window(screen: *mut GObject) -> XfwWindowPtr;
    pub fn xfw_screen_get_show_desktop(screen: *mut GObject) -> gboolean;
    pub fn xfw_screen_set_show_desktop(screen: *mut GObject, show: gboolean);
    pub fn xfw_screen_get_workspace_manager(screen: *mut GObject) -> *mut GObject;

    pub fn xfw_workspace_manager_list_workspaces(mgr: *mut GObject) -> *mut glib::ffi::GList;
    pub fn xfw_workspace_manager_list_workspace_groups(mgr: *mut GObject) -> *mut glib::ffi::GList;

    pub fn xfw_window_get_class_ids(window: XfwWindowPtr) -> *const *const c_char;
    pub fn xfw_window_get_name(window: XfwWindowPtr) -> *const c_char;
    pub fn xfw_window_get_icon(
        window: XfwWindowPtr,
        size: c_int,
        scale: c_int,
    ) -> *mut gdk_pixbuf::ffi::GdkPixbuf;
    pub fn xfw_window_get_window_type(window: XfwWindowPtr) -> XfwWindowType;
    pub fn xfw_window_get_capabilities(window: XfwWindowPtr) -> u32;
    pub fn xfw_window_is_active(window: XfwWindowPtr) -> gboolean;
    pub fn xfw_window_is_minimized(window: XfwWindowPtr) -> gboolean;
    pub fn xfw_window_is_urgent(window: XfwWindowPtr) -> gboolean;
    pub fn xfw_window_is_skip_tasklist(window: XfwWindowPtr) -> gboolean;
    pub fn xfw_window_is_pinned(window: XfwWindowPtr) -> gboolean;
    pub fn xfw_window_get_workspace(window: XfwWindowPtr) -> *mut GObject;
    pub fn xfw_window_get_application(window: XfwWindowPtr) -> *mut GObject;
    pub fn xfw_window_activate(
        window: XfwWindowPtr,
        seat: *mut GObject,
        timestamp: u64,
        error: *mut *mut glib::ffi::GError,
    ) -> gboolean;
    pub fn xfw_window_close(
        window: XfwWindowPtr,
        timestamp: u64,
        error: *mut *mut glib::ffi::GError,
    ) -> gboolean;
    pub fn xfw_window_set_minimized(
        window: XfwWindowPtr,
        minimized: gboolean,
        error: *mut *mut glib::ffi::GError,
    ) -> gboolean;
    pub fn xfw_window_set_maximized(
        window: XfwWindowPtr,
        maximized: gboolean,
        error: *mut *mut glib::ffi::GError,
    ) -> gboolean;
    pub fn xfw_window_is_maximized(window: XfwWindowPtr) -> gboolean;
    pub fn xfw_window_is_fullscreen(window: XfwWindowPtr) -> gboolean;
    pub fn xfw_window_is_above(window: XfwWindowPtr) -> gboolean;
    pub fn xfw_window_set_fullscreen(
        window: XfwWindowPtr,
        fullscreen: gboolean,
        error: *mut *mut glib::ffi::GError,
    ) -> gboolean;
    pub fn xfw_window_set_above(
        window: XfwWindowPtr,
        above: gboolean,
        error: *mut *mut glib::ffi::GError,
    ) -> gboolean;
    pub fn xfw_window_set_button_geometry(
        window: XfwWindowPtr,
        relative_to: *mut gdk::ffi::GdkWindow,
        rect: *const gdk::ffi::GdkRectangle,
        error: *mut *mut glib::ffi::GError,
    ) -> gboolean;
    pub fn xfw_window_move_to_workspace(
        window: XfwWindowPtr,
        workspace: *mut GObject,
        error: *mut *mut glib::ffi::GError,
    ) -> gboolean;

    pub fn xfw_workspace_get_state(workspace: *mut GObject) -> u32;
    pub fn xfw_workspace_get_name(workspace: *mut GObject) -> *const c_char;
    pub fn xfw_workspace_get_number(workspace: *mut GObject) -> std::os::raw::c_uint;
    pub fn xfw_workspace_activate(
        workspace: *mut GObject,
        error: *mut *mut glib::ffi::GError,
    ) -> gboolean;

    pub fn xfw_application_get_instance(
        application: *mut GObject,
        window: XfwWindowPtr,
    ) -> *mut GObject;
    pub fn xfw_application_instance_get_pid(instance: *mut GObject) -> c_int;
}

// ---------------------------------------------------------------------------
// Safe wrappers
// ---------------------------------------------------------------------------

pub unsafe fn xfw_window(ptr: XfwWindowPtr) -> glib::Object {
    // Transfer-none getters: take our own reference.
    glib::Object::from_glib_none(ptr)
}

pub unsafe fn xfw_object(ptr: *mut GObject) -> glib::Object {
    glib::Object::from_glib_none(ptr)
}

/// Take a transfer-full ref (`xfw_screen_get_default`).
pub unsafe fn xfw_object_full(ptr: *mut GObject) -> glib::Object {
    glib::Object::from_glib_full(ptr)
}

#[derive(Clone)]
pub struct Screen {
    pub obj: glib::Object,
}

impl Screen {
    pub fn default_screen() -> Option<Screen> {
        unsafe {
            let p = xfw_screen_get_default();
            if p.is_null() {
                None
            } else {
                Some(Screen {
                    obj: xfw_object_full(p),
                })
            }
        }
    }
    fn p(&self) -> *mut GObject {
        self.obj.to_glib_none().0
    }
    /// Live windows (transfer-none list, borrowed).
    pub fn windows(&self) -> Vec<glib::Object> {
        unsafe {
            let list = xfw_screen_get_windows(self.p());

            crate::util::glist_borrow_objects(list)
        }
    }
    pub fn active_window(&self) -> Option<glib::Object> {
        unsafe {
            let p = xfw_screen_get_active_window(self.p());
            if p.is_null() {
                None
            } else {
                Some(xfw_window(p))
            }
        }
    }
    pub fn show_desktop(&self) -> bool {
        unsafe { from_glib(xfw_screen_get_show_desktop(self.p())) }
    }
    pub fn set_show_desktop(&self, show: bool) {
        unsafe { xfw_screen_set_show_desktop(self.p(), show.into_glib()) }
    }
    pub fn workspace_manager(&self) -> glib::Object {
        unsafe { xfw_object(xfw_screen_get_workspace_manager(self.p())) }
    }
}

pub struct Window<'a>(&'a glib::Object);

impl<'a> Window<'a> {
    pub fn new(obj: &'a glib::Object) -> Window<'a> {
        Window(obj)
    }
    fn p(&self) -> XfwWindowPtr {
        self.0.to_glib_none().0
    }
    pub fn class_ids(&self) -> Vec<String> {
        unsafe {
            let mut out = Vec::new();
            let ids = xfw_window_get_class_ids(self.p());
            if ids.is_null() {
                return out;
            }
            let mut i = 0;
            loop {
                let s = *ids.offset(i);
                if s.is_null() {
                    break;
                }
                out.push(std::ffi::CStr::from_ptr(s).to_string_lossy().into_owned());
                i += 1;
            }
            out
        }
    }
    pub fn name(&self) -> Option<String> {
        unsafe {
            let n = xfw_window_get_name(self.p());
            if n.is_null() {
                None
            } else {
                Some(std::ffi::CStr::from_ptr(n).to_string_lossy().into_owned())
            }
        }
    }
    pub fn icon(&self, size: i32, scale: i32) -> Option<gdk_pixbuf::Pixbuf> {
        unsafe {
            let p = xfw_window_get_icon(self.p(), size, scale);
            if p.is_null() {
                None
            } else {
                Some(gdk_pixbuf::Pixbuf::from_glib_none(p))
            }
        }
    }
    pub fn window_type(&self) -> XfwWindowType {
        unsafe { xfw_window_get_window_type(self.p()) }
    }
    pub fn capabilities(&self) -> u32 {
        unsafe { xfw_window_get_capabilities(self.p()) }
    }
    pub fn is_active(&self) -> bool {
        unsafe { from_glib(xfw_window_is_active(self.p())) }
    }
    pub fn is_minimized(&self) -> bool {
        unsafe { from_glib(xfw_window_is_minimized(self.p())) }
    }
    pub fn is_maximized(&self) -> bool {
        unsafe { from_glib(xfw_window_is_maximized(self.p())) }
    }
    pub fn is_fullscreen(&self) -> bool {
        unsafe { from_glib(xfw_window_is_fullscreen(self.p())) }
    }
    pub fn is_above(&self) -> bool {
        unsafe { from_glib(xfw_window_is_above(self.p())) }
    }
    pub fn is_urgent(&self) -> bool {
        unsafe { from_glib(xfw_window_is_urgent(self.p())) }
    }
    pub fn is_skip_tasklist(&self) -> bool {
        unsafe { from_glib(xfw_window_is_skip_tasklist(self.p())) }
    }
    pub fn is_pinned(&self) -> bool {
        unsafe { from_glib(xfw_window_is_pinned(self.p())) }
    }
    pub fn workspace(&self) -> Option<glib::Object> {
        unsafe {
            let p = xfw_window_get_workspace(self.p());
            if p.is_null() {
                None
            } else {
                Some(xfw_object(p))
            }
        }
    }
    pub fn application(&self) -> Option<glib::Object> {
        unsafe {
            let p = xfw_window_get_application(self.p());
            if p.is_null() {
                None
            } else {
                Some(xfw_object(p))
            }
        }
    }
    pub fn activate(&self, timestamp: u32) {
        unsafe {
            let mut err: *mut glib::ffi::GError = std::ptr::null_mut();
            xfw_window_activate(self.p(), std::ptr::null_mut(), timestamp as u64, &mut err);
            report_error("activate window", err);
        }
    }
    pub fn close(&self, timestamp: u32) {
        unsafe {
            let mut err: *mut glib::ffi::GError = std::ptr::null_mut();
            xfw_window_close(self.p(), timestamp as u64, &mut err);
            report_error("close window", err);
        }
    }
    pub fn set_minimized(&self, minimized: bool) {
        unsafe {
            let mut err: *mut glib::ffi::GError = std::ptr::null_mut();
            xfw_window_set_minimized(self.p(), minimized.into_glib(), &mut err);
            report_error("change minimized state", err);
        }
    }
    pub fn set_maximized(&self, maximized: bool) {
        unsafe {
            let mut err: *mut glib::ffi::GError = std::ptr::null_mut();
            xfw_window_set_maximized(self.p(), maximized.into_glib(), &mut err);
            report_error("change maximized state", err);
        }
    }
    pub fn set_fullscreen(&self, fullscreen: bool) {
        unsafe {
            let mut err: *mut glib::ffi::GError = std::ptr::null_mut();
            xfw_window_set_fullscreen(self.p(), fullscreen.into_glib(), &mut err);
            report_error("change fullscreen state", err);
        }
    }
    pub fn set_above(&self, above: bool) {
        unsafe {
            let mut err: *mut glib::ffi::GError = std::ptr::null_mut();
            xfw_window_set_above(self.p(), above.into_glib(), &mut err);
            report_error("change always-on-top state", err);
        }
    }
    pub fn move_to_workspace(&self, workspace: &glib::Object) {
        unsafe {
            let mut err: *mut glib::ffi::GError = std::ptr::null_mut();
            xfw_window_move_to_workspace(self.p(), workspace.to_glib_none().0, &mut err);
            report_error("move window to workspace", err);
        }
    }
    pub fn set_button_geometry(&self, relative_to: &gdk::Window, rect: &gdk::Rectangle) {
        unsafe {
            let mut err: *mut glib::ffi::GError = std::ptr::null_mut();
            xfw_window_set_button_geometry(
                self.p(),
                relative_to.to_glib_none().0,
                rect.to_glib_none().0,
                &mut err,
            );
            report_error("set button geometry", err);
        }
    }
    pub fn xid(&self) -> crate::ffi_x11::Window {
        unsafe { xfw_window_x11_get_xid(self.p()) }
    }
}

extern "C" {
    pub fn xfw_window_x11_get_xid(window: XfwWindowPtr) -> c_ulong;
}

pub struct Workspace;

impl Workspace {
    pub fn state(ws: &glib::Object) -> u32 {
        unsafe { xfw_workspace_get_state(ws.to_glib_none().0) }
    }
    pub fn name(ws: &glib::Object) -> Option<String> {
        unsafe {
            let n = xfw_workspace_get_name(ws.to_glib_none().0);
            if n.is_null() {
                None
            } else {
                Some(std::ffi::CStr::from_ptr(n).to_string_lossy().into_owned())
            }
        }
    }
    pub fn number(ws: &glib::Object) -> u32 {
        unsafe { xfw_workspace_get_number(ws.to_glib_none().0) as u32 }
    }
    pub fn activate(ws: &glib::Object) {
        unsafe {
            let mut err: *mut glib::ffi::GError = std::ptr::null_mut();
            xfw_workspace_activate(ws.to_glib_none().0, &mut err);
            report_error("activate workspace", err);
        }
    }
}

/// `xfw_application_get_instance` + pid lookup.
pub fn application_pid(app: Option<&glib::Object>, window: &glib::Object) -> i32 {
    let Some(app) = app else {
        return 0;
    };
    unsafe {
        let inst = xfw_application_get_instance(app.to_glib_none().0, window.to_glib_none().0);
        if inst.is_null() {
            return 0;
        }
        let pid = xfw_application_instance_get_pid(inst);
        if pid > 0 {
            pid
        } else {
            0
        }
    }
}

/// Log and free an error returned through a libxfce4windowing `GError **`.
///
/// The C APIs allocate these errors for the caller. Leaving them untouched
/// leaks on every failed operation; converting the message alone is not enough.
unsafe fn report_error(operation: &str, err: *mut glib::ffi::GError) {
    if err.is_null() {
        return;
    }
    let message = glib::GString::from_glib_none((*err).message);
    glib::g_warning!("zero-dock", "Zero Dock {operation}: {message}");
    glib::ffi::g_error_free(err);
}
