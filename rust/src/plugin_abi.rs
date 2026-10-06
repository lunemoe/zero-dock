//! XFCE panel plugin ABI, implemented entirely in Rust.
//!
//! The wrapper process (`wrapper-2.0`) looks up `xfce_panel_module_init`
//! first and falls back to the legacy `xfce_panel_module_construct`. We
//! deliberately export **only** the legacy construct entry point: in the
//! init path the wrapper instantiates a bare `XfcePanelPlugin` itself and
//! never calls the plugin's construct function, which would leave the dock
//! empty. The construct path hands us the plugin object and defers our setup
//! until the widget is realized — exactly what the C
//! `XFCE_PANEL_PLUGIN_REGISTER` macro did.
//!
//! This module is the ABI boundary itself, so it is one of the places allowed
//! to contain `unsafe` (see the crate root).

#![allow(unsafe_code)]
#![allow(clippy::missing_safety_doc)]

use crate::ffi_xfce::XfcePanelPlugin;
use glib::ffi::GType;
use glib::gobject_ffi::GObject;
use std::os::raw::{c_char, c_int, c_ulong, c_void};

// ---------------------------------------------------------------------------
// C library / GObject glue that has no Rust wrapper in our dependency set
// ---------------------------------------------------------------------------

extern "C" {
    fn xfce_panel_plugin_get_type() -> GType;
    fn gdk_screen_get_type() -> GType;
    fn g_type_check_instance_is_a(
        instance: *mut glib::gobject_ffi::GTypeInstance,
        type_: GType,
    ) -> glib::ffi::gboolean;
    fn g_object_new(object_type: GType, first_property_name: *const c_char, ...) -> *mut GObject;
    fn g_signal_connect_data(
        instance: *mut GObject,
        detailed_signal: *const c_char,
        c_handler: *mut c_void,
        data: *mut c_void,
        destroy_notify: *mut c_void,
        connect_flags: glib::gobject_ffi::GConnectFlags,
    ) -> c_ulong;
    fn g_signal_handlers_disconnect_matched(
        instance: *mut GObject,
        mask: u32,
        signal_id: u32,
        detail: u32,
        closure: *mut glib::gobject_ffi::GClosure,
        func: *mut c_void,
        data: *mut c_void,
    ) -> u32;

    // libc gettext bindings; the domain only needs binding once per process.
    fn bindtextdomain(domainname: *const c_char, dirname: *const c_char) -> *mut c_char;
    fn bind_textdomain_codeset(domainname: *const c_char, codeset: *const c_char) -> *mut c_char;
}

/// Bind the gettext domain. Called from the construct path; idempotent.
pub fn bind_i18n() {
    let localedir = option_env!("ZERO_DOCK_LOCALEDIR").unwrap_or("/usr/share/locale");
    let domain = crate::util::GETTEXT_DOMAIN;
    let domain_c = std::ffi::CString::new(domain).unwrap_or_default();
    let locale_c = std::ffi::CString::new(localedir).unwrap_or_default();
    let codeset_c = std::ffi::CString::new("UTF-8").unwrap_or_default();
    unsafe {
        bindtextdomain(domain_c.as_ptr(), locale_c.as_ptr());
        bind_textdomain_codeset(domain_c.as_ptr(), codeset_c.as_ptr());
    }
}

/// The plugin constructor body shared by the ABI entry and the test host.
///
/// # Safety
/// `plugin` must be a valid `XfcePanelPlugin` GObject, invoked on the GTK
/// main thread.
pub unsafe fn construct_plugin(plugin: *mut GObject) {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // The panel wrapper (or test host) has already run C gtk_init.
        // GTK3's gtk_init is idempotent, so re-running it here registers the
        // initialized state with the Rust bindings.
        if !gtk::is_initialized_main_thread() && gtk::init().is_err() {
            glib::g_warning!("zero-dock", "Zero Dock: GTK 初始化失败");
            return;
        }
        bind_i18n();
        let plugin = XfcePanelPlugin(plugin);
        crate::dock::construct(plugin);
    }));
    if result.is_err() {
        glib::g_warning!("zero-dock", "Zero Dock: 构造过程发生内部错误");
    }
}

/// `realize` hook: runs the constructor body once, then disconnects itself,
/// mirroring `xfce_panel_module_realize` from the C macro.
unsafe extern "C" fn module_realize(xpp: *mut GObject, _data: glib::ffi::gpointer) {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        g_signal_handlers_disconnect_matched(
            xpp,
            glib::gobject_ffi::G_SIGNAL_MATCH_FUNC | glib::gobject_ffi::G_SIGNAL_MATCH_DATA,
            0,
            0,
            std::ptr::null_mut(),
            module_realize as *mut c_void,
            std::ptr::null_mut(),
        );
        construct_plugin(xpp);
    }));
    let _ = result;
}

fn is_screen(ptr: *mut glib::gobject_ffi::GTypeInstance) -> bool {
    unsafe { g_type_check_instance_is_a(ptr, gdk_screen_get_type()) != glib::ffi::GFALSE }
}

/// Legacy module entry point: builds the `XfcePanelPlugin` widget and hooks
/// `realize` so the Rust constructor runs exactly once.
///
/// # Safety
/// Called by the panel wrapper through GModule with C ownership semantics:
/// the returned object (when non-NULL) is owned by the caller.
#[no_mangle]
pub unsafe extern "C" fn xfce_panel_module_construct(
    xpp_name: *const c_char,
    xpp_unique_id: c_int,
    xpp_display_name: *const c_char,
    xpp_comment: *const c_char,
    xpp_arguments: *mut *mut c_char,
    xpp_screen: *mut glib::gobject_ffi::GTypeInstance,
) -> *mut GObject {
    // g_return_val_if_fail equivalents: refuse invalid input like the C macro.
    if !is_screen(xpp_screen) || xpp_name.is_null() || xpp_unique_id == -1 {
        glib::g_critical!("zero-dock", "Zero Dock: 插件模块收到无效的构造参数");
        return std::ptr::null_mut();
    }

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let name = std::ffi::CStr::from_ptr(xpp_name);
        let display_name = if xpp_display_name.is_null() {
            c""
        } else {
            std::ffi::CStr::from_ptr(xpp_display_name)
        };
        let comment = if xpp_comment.is_null() {
            c""
        } else {
            std::ffi::CStr::from_ptr(xpp_comment)
        };
        let plugin_type = xfce_panel_plugin_get_type();

        // g_object_new with the C varargs property list. For the strv
        // "arguments" property the collector reads a plain pointer, so the
        // raw argv array is passed through directly (NULL when absent).
        let plugin = if xpp_arguments.is_null() {
            g_object_new(
                plugin_type,
                c"name".as_ptr(),
                name.as_ptr(),
                c"unique-id".as_ptr(),
                xpp_unique_id,
                c"display-name".as_ptr(),
                display_name.as_ptr(),
                c"comment".as_ptr(),
                comment.as_ptr(),
                std::ptr::null::<c_char>(),
            )
        } else {
            g_object_new(
                plugin_type,
                c"name".as_ptr(),
                name.as_ptr(),
                c"unique-id".as_ptr(),
                xpp_unique_id,
                c"display-name".as_ptr(),
                display_name.as_ptr(),
                c"comment".as_ptr(),
                comment.as_ptr(),
                c"arguments".as_ptr(),
                xpp_arguments,
                std::ptr::null::<c_char>(),
            )
        };
        if plugin.is_null() {
            return std::ptr::null_mut();
        }

        g_signal_connect_data(
            plugin,
            c"realize".as_ptr(),
            module_realize as *mut c_void,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            glib::gobject_ffi::G_CONNECT_AFTER,
        );

        // The full reference from g_object_new is transferred to the caller.
        plugin
    }));

    match result {
        Ok(ptr) => ptr,
        Err(_) => {
            glib::g_critical!("zero-dock", "Zero Dock: 模块构造发生内部错误");
            std::ptr::null_mut()
        }
    }
}
