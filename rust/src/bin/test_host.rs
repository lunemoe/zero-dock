//! Standalone GUI test host, replacing the former `tests/host.c`.
//!
//! Constructs a real `XfcePanelPlugin` GObject — the same object shape the
//! panel wrapper creates — inside a plain GTK window and runs the full Rust
//! dock inside it. Usage:
//!
//! ```text
//! zero-dock-test-host [rc-file] [shutdown-seconds]
//! ```
//!
//! With no arguments a private rc file is created and removed on exit.

use glib::ffi::GType;
use glib::gobject_ffi::GObject;

use gtk::prelude::*;
use std::os::raw::{c_char, c_int};

extern "C" {
    fn xfce_panel_plugin_get_type() -> GType;
    fn xfce_panel_plugin_provider_set_size(provider: *mut GObject, size: c_int);
    fn g_object_new(object_type: GType, first_property_name: *const c_char, ...) -> *mut GObject;
    fn g_object_set_data(object: *mut GObject, key: *const c_char, data: *mut std::ffi::c_void);
}

fn host_plugin(name: &str, unique_id: i32, display_name: &str, comment: &str) -> *mut GObject {
    let name_c = std::ffi::CString::new(name).unwrap();
    let display_c = std::ffi::CString::new(display_name).unwrap();
    let comment_c = std::ffi::CString::new(comment).unwrap();
    unsafe {
        g_object_new(
            xfce_panel_plugin_get_type(),
            c"name".as_ptr(),
            name_c.as_ptr(),
            c"unique-id".as_ptr(),
            unique_id,
            c"display-name".as_ptr(),
            display_c.as_ptr(),
            c"comment".as_ptr(),
            comment_c.as_ptr(),
            std::ptr::null::<c_char>(),
        )
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    gtk::init().expect("gtk init");

    // Rc file: caller-provided or a private temporary one.
    let (rc_path, _temp_dir) = match args.get(1) {
        Some(path) if !path.is_empty() => (path.clone(), None),
        _ => {
            let dir = std::env::temp_dir().join(format!("zero-dock-host-{}", std::process::id()));
            std::fs::create_dir_all(&dir).expect("create temp dir");
            (
                dir.join("host.rc").to_string_lossy().into_owned(),
                Some(dir),
            )
        }
    };

    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.set_title("Zero Dock 独立测试台");
    window.set_default_size(800, 64);
    window.set_type_hint(gdk::WindowTypeHint::Utility);

    let plugin = host_plugin("zero-dock", 9001, "Zero Dock 测试", "Independent test host");
    assert!(!plugin.is_null(), "failed to create XfcePanelPlugin");

    let rc_c = std::ffi::CString::new(rc_path.clone()).unwrap();
    unsafe {
        g_object_set_data(
            plugin,
            c"test-rc".as_ptr(),
            rc_c.as_ptr() as *mut std::os::raw::c_void,
        );
    }

    let plugin_object: glib::Object =
        unsafe { glib::translate::FromGlibPtrNone::from_glib_none(plugin) };
    let plugin_widget: gtk::Widget = plugin_object
        .downcast()
        .expect("XfcePanelPlugin is a GtkWidget");
    window.set_child(Some(&plugin_widget));
    unsafe {
        xfce_panel_plugin_provider_set_size(plugin, 48);
    }
    unsafe { zero_dock::construct_plugin(plugin) };

    window.connect_destroy(|_| gtk::main_quit());
    window.show_all();

    if let Some(seconds) = args.get(2).and_then(|s| s.parse::<u64>().ok()) {
        glib::timeout_add_seconds_local(seconds as u32, move || {
            window.close();
            glib::ControlFlow::Break
        });
    }

    gtk::main();

    if let Some(dir) = _temp_dir {
        let _ = std::fs::remove_file(dir.join("host.rc"));
        let _ = std::fs::remove_dir(dir);
    }
}
