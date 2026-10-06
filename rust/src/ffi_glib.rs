//! GLib plumbing that the `glib` crate leaves to callers, plus the borrowed
//! `GList` walk used when talking to libxfce4windowing.
//!
//! This is one of the four modules allowed to contain `unsafe` (see the crate
//! root).

#![allow(unsafe_code)]

/// `g_source_remove()`, ignoring the "no such source" failure.
///
/// GLib emits a critical when the id is gone, so callers must only pass an id
/// they still consider armed; they cannot act on the result anyway.
pub fn remove_source_id(id: glib::SourceId) {
    unsafe {
        glib::ffi::g_source_remove(id.as_raw());
    }
}

/// Iterate a transfer-none `GList` of GObjects without taking ownership of
/// the list or its items. The caller keeps the list lifetime rules of the
/// underlying API.
///
/// # Safety
/// `list` must be a live `GList` whose data pointers are GObjects owned by
/// something that outlives the call (the usual transfer-none contract).
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
