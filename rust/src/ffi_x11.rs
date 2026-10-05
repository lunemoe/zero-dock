//! Thin unsafe layer over Xlib / XComposite / XInput2 and the GDK X11 glue.
//!
//! The `x11` crate supplies the core declarations; this module adds the few
//! missing ones (XComposite, the GDK X11 helpers, cairo-xlib) and small safe
//! wrappers for the exact call shapes the dock needs. Every X11 call that can
//! raise an asynchronous X error goes through a GDK error trap, exactly like
//! the C implementation did.

use gdk::ffi as gdk_ffi;
use std::os::raw::{c_int, c_ulong, c_void};
use x11::xlib;

pub use x11::xlib::{Atom, Display, Window, XA_CARDINAL};

// ---------------------------------------------------------------------------
// GDK X11 glue (not wrapped by the gdk crate for GTK3)
// ---------------------------------------------------------------------------

extern "C" {
    pub fn gdk_x11_display_get_xdisplay(display: *mut gdk_ffi::GdkDisplay) -> *mut Display;
    pub fn gdk_x11_display_error_trap_push(display: *mut gdk_ffi::GdkDisplay);
    pub fn gdk_x11_display_error_trap_pop(display: *mut gdk_ffi::GdkDisplay) -> c_int;
    pub fn gdk_x11_display_error_trap_pop_ignored(display: *mut gdk_ffi::GdkDisplay);
    pub fn gdk_x11_window_foreign_new_for_display(
        display: *mut gdk_ffi::GdkDisplay,
        window: Window,
    ) -> *mut gdk_ffi::GdkWindow;
    pub fn gdk_x11_get_server_time(window: *mut gdk_ffi::GdkWindow) -> c_ulong;
    pub fn gdk_x11_display_get_type() -> glib::ffi::GType;

    pub fn gdk_window_add_filter(
        window: *mut gdk_ffi::GdkWindow,
        func: Option<GdkFilterFunc>,
        data: *mut c_void,
    );
    pub fn gdk_window_remove_filter(
        window: *mut gdk_ffi::GdkWindow,
        func: Option<GdkFilterFunc>,
        data: *mut c_void,
    );

    pub fn cairo_xlib_surface_create(
        dpy: *mut Display,
        drawable: c_ulong,
        visual: *mut x11::xlib::Visual,
        width: c_int,
        height: c_int,
    ) -> *mut cairo::ffi::cairo_surface_t;
}

pub type GdkFilterFunc = unsafe extern "C" fn(
    xevent: *mut c_void,
    event: *mut gdk_ffi::GdkEvent,
    data: *mut c_void,
) -> c_int;

pub const GDK_FILTER_CONTINUE: c_int = 0;

/// Safe accessor for the default GDK X display pointer.
pub fn gdk_display() -> *mut gdk_ffi::GdkDisplay {
    unsafe { gdk_ffi::gdk_display_get_default() }
}

/// The Xlib `Display*` behind the default GDK display.
pub fn xdisplay() -> *mut Display {
    unsafe { gdk_x11_display_get_xdisplay(display_ptr_unchecked()) }
}

fn display_ptr_unchecked() -> *mut gdk_ffi::GdkDisplay {
    // gdk_ffi::gdk_display_get_default() is safe to call once GTK is
    // initialized; the Rust pointer type mismatch is handled here once.
    unsafe { gdk_ffi::gdk_display_get_default() }
}

/// The default root GDK window (for server time queries).
pub fn default_root_window() -> gdk::Window {
    let screen = gdk::Screen::default().unwrap();
    screen.root_window().unwrap()
}

pub fn intern_atom(name: &str) -> Atom {
    let cname = std::ffi::CString::new(name).unwrap_or_default();
    unsafe { x11::xlib::XInternAtom(xdisplay(), cname.as_ptr(), xlib::False) }
}

/// GDK-equivalent of `GDK_IS_X11_DISPLAY(gdk_display_get_default())`.
pub fn is_x11() -> bool {
    unsafe {
        let display = gdk_display();
        !display.is_null()
            && glib::gobject_ffi::g_type_check_instance_is_a(
                display as *mut glib::gobject_ffi::GTypeInstance,
                gdk_x11_display_get_type(),
            ) != glib::ffi::GFALSE
    }
}

// --- error traps -----------------------------------------------------------

pub fn error_trap_push() {
    unsafe { gdk_x11_display_error_trap_push(gdk_display()) }
}

pub fn error_trap_pop() -> bool {
    unsafe { gdk_x11_display_error_trap_pop(gdk_display()) != 0 }
}

pub fn error_trap_pop_ignored() {
    unsafe { gdk_x11_display_error_trap_pop_ignored(gdk_display()) }
}

// ---------------------------------------------------------------------------
// Window properties (self-contained safe wrappers)
// ---------------------------------------------------------------------------

/// `XGetWindowProperty` for a single 32-bit CARDINAL value (e.g. `_NET_WM_PID`).
pub fn get_cardinal_property(xid: Window, atom: Atom) -> Option<u64> {
    unsafe {
        let display = xdisplay();
        error_trap_push();
        let mut rtype: Atom = 0;
        let mut format = 0;
        let mut nitems: c_ulong = 0;
        let mut rest: c_ulong = 0;
        let mut data: *mut std::os::raw::c_uchar = std::ptr::null_mut();
        let ok = x11::xlib::XGetWindowProperty(
            display,
            xid,
            atom,
            0,
            1,
            xlib::False,
            XA_CARDINAL,
            &mut rtype,
            &mut format,
            &mut nitems,
            &mut rest,
            &mut data,
        ) == xlib::Success as i32;
        let value = if ok && !data.is_null() && nitems == 1 && format == 32 && rtype == XA_CARDINAL
        {
            Some(*(data as *const c_ulong) as u64)
        } else {
            None
        };
        if !data.is_null() {
            x11::xlib::XFree(data as *mut c_void);
        }
        if error_trap_pop() {
            None
        } else {
            value
        }
    }
}

/// `XGetWindowProperty` for a string property (`XA_STRING` or `UTF8_STRING`).
pub fn get_string_property(xid: Window, name: &str) -> Option<String> {
    unsafe {
        let display = xdisplay();
        let utf8 = intern_atom("UTF8_STRING");
        let cname = std::ffi::CString::new(name).ok()?;
        error_trap_push();
        let atom = x11::xlib::XInternAtom(display, cname.as_ptr(), xlib::False);
        let mut rtype: Atom = 0;
        let mut format = 0;
        let mut nitems: c_ulong = 0;
        let mut rest: c_ulong = 0;
        let mut data: *mut std::os::raw::c_uchar = std::ptr::null_mut();
        let ok = x11::xlib::XGetWindowProperty(
            display,
            xid,
            atom,
            0,
            1024,
            xlib::False,
            xlib::AnyPropertyType as Atom,
            &mut rtype,
            &mut format,
            &mut nitems,
            &mut rest,
            &mut data,
        ) == xlib::Success as i32;
        let value = if ok
            && !data.is_null()
            && format == 8
            && rest == 0
            && nitems > 0
            && (rtype == xlib::XA_STRING as Atom || rtype == utf8)
        {
            let bytes = std::slice::from_raw_parts(data, nitems as usize);
            std::str::from_utf8(bytes).ok().map(|s| s.to_string())
        } else {
            None
        };
        if !data.is_null() {
            x11::xlib::XFree(data as *mut c_void);
        }
        if error_trap_pop() {
            None
        } else {
            value
        }
    }
}

// ---------------------------------------------------------------------------
// XComposite capture helpers
// ---------------------------------------------------------------------------

extern "C" {
    pub fn XCompositeNameWindowPixmap(dpy: *mut Display, window: Window) -> x11::xlib::Pixmap;
}

/// The compositor's redirected pixmap for `client`'s frame window, or the
/// client itself when it is not wrapped. `None` means the client window has
/// no usable frame. Callers must hold an error trap around the whole capture
/// sequence, mirroring the C implementation.
pub fn frame_target(client: Window) -> Window {
    unsafe {
        let display = xdisplay();
        let mut root: Window = 0;
        let mut parent: Window = 0;
        let mut children: *mut Window = std::ptr::null_mut();
        let mut n: std::os::raw::c_uint = 0;
        let ok = x11::xlib::XQueryTree(
            display,
            client,
            &mut root,
            &mut parent,
            &mut children,
            &mut n,
        );
        if !children.is_null() {
            x11::xlib::XFree(children as *mut c_void);
        }
        if ok != 0 && parent != root && parent != 0 {
            parent
        } else {
            client
        }
    }
}

pub struct WindowAttrs {
    pub visual: *mut x11::xlib::Visual,
    pub width: i32,
    pub height: i32,
    pub viewable: bool,
}

/// `XGetWindowAttributes` for a window; `None` when the window is gone.
/// Must run inside an error trap.
pub unsafe fn get_window_attributes(display: *mut Display, xid: Window) -> Option<WindowAttrs> {
    let mut attr: x11::xlib::XWindowAttributes = std::mem::zeroed();
    if x11::xlib::XGetWindowAttributes(display, xid, &mut attr) != 0 {
        Some(WindowAttrs {
            visual: attr.visual,
            width: attr.width,
            height: attr.height,
            viewable: attr.map_state == xlib::IsViewable,
        })
    } else {
        None
    }
}

pub fn free_pixmap(pix: x11::xlib::Pixmap) {
    unsafe {
        error_trap_push();
        x11::xlib::XFreePixmap(xdisplay(), pix);
        error_trap_pop_ignored();
    }
}

// ---------------------------------------------------------------------------
// GDK current event time
// ---------------------------------------------------------------------------

pub fn server_time() -> u32 {
    unsafe {
        let root = default_root_window();
        gdk_x11_get_server_time(glib::translate::ToGlibPtr::to_glib_none(&root).0) as u32
    }
}
