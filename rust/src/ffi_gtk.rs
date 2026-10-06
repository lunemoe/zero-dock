//! GTK / GDK / GIO glue that the gtk-rs dependency set does not wrap, plus the
//! few GObject-data helpers the dock needs.
//!
//! This is one of the four modules allowed to contain `unsafe` (see the crate
//! root); every item below is a panic-free shim whose preconditions are checked
//! here, so the rest of the crate stays safe Rust.

#![allow(unsafe_code)]

use glib::translate::*;
use gtk::prelude::*;
use std::os::raw::c_void;

extern "C" {
    fn pango_cairo_show_layout(
        cr: *mut cairo::ffi::cairo_t,
        layout: *mut pango::ffi::PangoLayout,
    );
    fn g_key_file_set_string_list(
        key_file: *mut glib::ffi::GKeyFile,
        group_name: *const std::os::raw::c_char,
        key: *const std::os::raw::c_char,
        list: *const *const std::os::raw::c_char,
        length: usize,
    );
    fn g_object_set_data_full(
        object: *mut glib::gobject_ffi::GObject,
        key: *const std::os::raw::c_char,
        data: *mut c_void,
        destroy: Option<unsafe extern "C" fn(*mut c_void)>,
    );
    fn g_object_get_data(
        object: *mut glib::gobject_ffi::GObject,
        key: *const std::os::raw::c_char,
    ) -> *mut c_void;
}

/// `gtk_widget_destroy()`.
///
/// The gtk-rs binding is `unsafe` because the widget handle must not be used
/// afterwards. Callers hand over a handle they immediately drop (or a widget
/// they already took out of the model), so the wrapper is safe to expose.
pub fn destroy(widget: &impl IsA<gtk::Widget>) {
    unsafe { widget.as_ref().destroy() }
}

/// `pango_cairo_show_layout()`: draw a layout into the current cairo context.
pub fn show_layout(cr: &cairo::Context, layout: &pango::Layout) {
    unsafe { pango_cairo_show_layout(cr.to_raw_none(), layout.to_glib_none().0) }
}

/// `g_app_info_get_executable()`, but NULL-safe.
///
/// gio-rs turns the (documented, nullable) result straight into a `PathBuf` and
/// would warn on a NULL pointer; the dock only needs a match hint, so map NULL
/// to an empty string.
pub fn app_info_executable(app: &impl IsA<gio::AppInfo>) -> String {
    unsafe {
        let exe = gio::ffi::g_app_info_get_executable(app.as_ref().to_glib_none().0);
        if exe.is_null() {
            String::new()
        } else {
            std::ffi::CStr::from_ptr(exe).to_string_lossy().into_owned()
        }
    }
}

/// `g_key_file_set_string_list()`: store a list that `KeyFile::string_list`
/// reads back.
pub fn key_file_set_string_list(
    file: &glib::KeyFile,
    group: &str,
    key: &str,
    values: &[String],
) {
    let group = std::ffi::CString::new(group).unwrap_or_default();
    let key = std::ffi::CString::new(key).unwrap_or_default();
    let owned: Vec<std::ffi::CString> = values
        .iter()
        .map(|value| std::ffi::CString::new(value.as_str()).unwrap_or_default())
        .collect();
    let ptrs: Vec<*const std::os::raw::c_char> = owned.iter().map(|c| c.as_ptr()).collect();
    unsafe {
        g_key_file_set_string_list(
            file.to_glib_none().0,
            group.as_ptr(),
            key.as_ptr(),
            ptrs.as_ptr(),
            ptrs.len(),
        );
    }
}

/// A string previously attached with `g_object_set_data` under `key`, using
/// the "data is a NUL-terminated C string" convention of the test host.
pub fn object_data_str(object: &impl IsA<glib::Object>, key: &str) -> Option<String> {
    let key = std::ffi::CString::new(key).ok()?;
    unsafe {
        let data = g_object_get_data(object.as_ref().to_glib_none().0, key.as_ptr());
        if data.is_null() {
            None
        } else {
            Some(
                std::ffi::CStr::from_ptr(data as *const std::os::raw::c_char)
                    .to_string_lossy()
                    .into_owned(),
            )
        }
    }
}

/// A `T` owned by a GObject datum, with the destructor GLib should run.
struct Datum<T> {
    value: T,
    destroy: Box<dyn FnOnce(T)>,
}

unsafe extern "C" fn datum_destroy<T>(data: *mut c_void) {
    if data.is_null() {
        return;
    }
    let datum = Box::from_raw(data as *mut Datum<T>);
    (datum.destroy)(datum.value);
}

/// Attach `value` to `object` under `key`, keeping it alive until the object
/// drops the datum (or is finalized) and then calling `destroy(value)`.
///
/// This is `g_object_set_data_full` with the ownership rules of a Rust `Box`,
/// used to hand the dock to the panel plugin without a custom ref-count
/// wrapper.
pub fn set_owned_data<T: 'static>(
    object: &impl IsA<glib::Object>,
    key: &str,
    value: T,
    destroy: impl FnOnce(T) + 'static,
) {
    let key = std::ffi::CString::new(key).unwrap_or_default();
    let datum: Box<Datum<T>> = Box::new(Datum {
        value,
        destroy: Box::new(destroy),
    });
    unsafe {
        g_object_set_data_full(
            object.as_ref().to_glib_none().0,
            key.as_ptr(),
            Box::into_raw(datum) as *mut c_void,
            Some(datum_destroy::<T>),
        );
    }
}
