//! Thin unsafe layer over Xlib / XComposite / XInput2 and the GDK X11 glue.
//!
//! The `x11` crate supplies the core declarations; this module adds the few
//! missing ones (XComposite, the GDK X11 helpers, cairo-xlib) and exposes the
//! exact call shapes the dock needs as safe functions. Every X11 call that can
//! raise an asynchronous X error goes through a GDK error trap, exactly like
//! the C implementation did.
//!
//! This is one of the four modules allowed to contain `unsafe` (see the crate
//! root): the pointer plumbing is contained here, and the safe surface is what
//! the rest of the crate uses.

#![allow(unsafe_code)]

use gdk::ffi as gdk_ffi;
use glib::translate::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::os::raw::{c_int, c_ulong, c_void};
use x11::xlib;
use x11::xinput2;

pub use x11::xlib::{Atom, Display, Window, XA_CARDINAL};

// ---------------------------------------------------------------------------
// GDK X11 glue (not wrapped by the gdk crate for GTK3)
// ---------------------------------------------------------------------------

extern "C" {
    fn gdk_x11_display_get_xdisplay(display: *mut gdk_ffi::GdkDisplay) -> *mut Display;
    fn gdk_x11_display_error_trap_push(display: *mut gdk_ffi::GdkDisplay);
    fn gdk_x11_display_error_trap_pop(display: *mut gdk_ffi::GdkDisplay) -> c_int;
    fn gdk_x11_display_error_trap_pop_ignored(display: *mut gdk_ffi::GdkDisplay);
    fn gdk_x11_window_foreign_new_for_display(
        display: *mut gdk_ffi::GdkDisplay,
        window: Window,
    ) -> *mut gdk_ffi::GdkWindow;
    fn gdk_x11_get_server_time(window: *mut gdk_ffi::GdkWindow) -> c_ulong;
    fn gdk_x11_display_get_type() -> glib::ffi::GType;

    fn gdk_window_add_filter(
        window: *mut gdk_ffi::GdkWindow,
        func: Option<GdkFilterFunc>,
        data: *mut c_void,
    );
    fn gdk_window_remove_filter(
        window: *mut gdk_ffi::GdkWindow,
        func: Option<GdkFilterFunc>,
        data: *mut c_void,
    );

    fn cairo_xlib_surface_create(
        dpy: *mut Display,
        drawable: c_ulong,
        visual: *mut xlib::Visual,
        width: c_int,
        height: c_int,
    ) -> *mut cairo::ffi::cairo_surface_t;

    fn XCompositeNameWindowPixmap(dpy: *mut Display, window: Window) -> xlib::Pixmap;
}

type GdkFilterFunc = unsafe extern "C" fn(
    xevent: *mut c_void,
    event: *mut gdk_ffi::GdkEvent,
    data: *mut c_void,
) -> c_int;

const GDK_FILTER_CONTINUE: c_int = 0;

/// The Xlib `Display*` behind the default GDK display.
pub fn xdisplay() -> *mut Display {
    unsafe { gdk_x11_display_get_xdisplay(gdk_ffi::gdk_display_get_default()) }
}

/// The X window id behind a GDK window (used by the test harness).
#[cfg(feature = "test-harness")]
pub fn window_xid(window: &gdk::Window) -> u64 {
    extern "C" {
        fn gdk_x11_window_get_xid(window: *mut gdk_ffi::GdkWindow) -> c_ulong;
    }
    unsafe { gdk_x11_window_get_xid(window.to_glib_none().0) as u64 }
}

pub fn intern_atom(name: &str) -> Atom {
    let cname = std::ffi::CString::new(name).unwrap_or_default();
    unsafe { xlib::XInternAtom(xdisplay(), cname.as_ptr(), xlib::False) }
}

/// GDK-equivalent of `GDK_IS_X11_DISPLAY(gdk_display_get_default())`.
pub fn is_x11() -> bool {
    unsafe {
        let display = gdk_ffi::gdk_display_get_default();
        !display.is_null()
            && glib::gobject_ffi::g_type_check_instance_is_a(
                display as *mut glib::gobject_ffi::GTypeInstance,
                gdk_x11_display_get_type(),
            ) != glib::ffi::GFALSE
    }
}

// --- error traps -----------------------------------------------------------

fn error_trap_push() {
    unsafe { gdk_x11_display_error_trap_push(gdk_ffi::gdk_display_get_default()) }
}

fn error_trap_pop() -> bool {
    unsafe { gdk_x11_display_error_trap_pop(gdk_ffi::gdk_display_get_default()) != 0 }
}

fn error_trap_pop_ignored() {
    unsafe { gdk_x11_display_error_trap_pop_ignored(gdk_ffi::gdk_display_get_default()) }
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
        let ok = xlib::XGetWindowProperty(
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
            xlib::XFree(data as *mut c_void);
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
        let atom = xlib::XInternAtom(display, cname.as_ptr(), xlib::False);
        let mut rtype: Atom = 0;
        let mut format = 0;
        let mut nitems: c_ulong = 0;
        let mut rest: c_ulong = 0;
        let mut data: *mut std::os::raw::c_uchar = std::ptr::null_mut();
        let ok = xlib::XGetWindowProperty(
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
            xlib::XFree(data as *mut c_void);
        }
        if error_trap_pop() {
            None
        } else {
            value
        }
    }
}

// ---------------------------------------------------------------------------
// Identity property watch (ports the GdkWindow X event filter)
// ---------------------------------------------------------------------------

struct PropertyWatchData {
    atoms: Vec<Atom>,
    notify: RefCell<Box<dyn FnMut()>>,
}

unsafe extern "C" fn property_filter(
    xevent: *mut c_void,
    _event: *mut gdk_ffi::GdkEvent,
    data: *mut c_void,
) -> c_int {
    // GDK filters are plain C callbacks: a panic here would unwind into GDK,
    // so contain it and keep filtering.
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if data.is_null() || xevent.is_null() {
            return;
        }
        let watch = &*(data as *const PropertyWatchData);
        let event = &*(xevent as *const xlib::XEvent);
        if event.get_type() != xlib::PropertyNotify {
            return;
        }
        if !watch.atoms.contains(&event.property.atom) {
            return;
        }
        if let Ok(mut notify) = watch.notify.try_borrow_mut() {
            notify();
        }
    }));
    GDK_FILTER_CONTINUE
}

/// A live property-change filter on a GDK window; removed on drop.
pub struct PropertyWatch {
    window: gdk::Window,
    data: *mut PropertyWatchData,
}

impl PropertyWatch {
    /// Call `notify` whenever one of `atoms` changes on `window`.
    pub fn new(window: &gdk::Window, atoms: &[Atom], notify: impl FnMut() + 'static) -> Self {
        let data = Box::into_raw(Box::new(PropertyWatchData {
            atoms: atoms.to_vec(),
            notify: RefCell::new(Box::new(notify)),
        }));
        unsafe {
            gdk_window_add_filter(
                window.to_glib_none().0,
                Some(property_filter),
                data as *mut c_void,
            );
        }
        PropertyWatch {
            window: window.clone(),
            data,
        }
    }
}

impl Drop for PropertyWatch {
    fn drop(&mut self) {
        // Remove the filter first: afterwards the callback can no longer run
        // and the boxed state is ours to free.
        unsafe {
            gdk_window_remove_filter(
                self.window.to_glib_none().0,
                Some(property_filter),
                self.data as *mut c_void,
            );
            drop(Box::from_raw(self.data));
        }
    }
}

/// Wrap a foreign X window in a GDK window and watch property changes on it.
/// `None` when the window has no usable foreign wrapper.
pub fn watch_window_properties(xid: Window) -> Option<gdk::Window> {
    if xid == 0 {
        return None;
    }
    unsafe {
        error_trap_push();
        let foreign = gdk_x11_window_foreign_new_for_display(
            gdk_ffi::gdk_display_get_default(),
            xid,
        );
        let window = if foreign.is_null() {
            None
        } else {
            let window: gdk::Window = from_glib_full(foreign);
            let events = window.events() | gdk::EventMask::PROPERTY_CHANGE_MASK;
            window.set_events(events);
            Some(window)
        };
        error_trap_pop_ignored();
        window
    }
}

// ---------------------------------------------------------------------------
// XComposite capture
// ---------------------------------------------------------------------------

/// The compositor's redirected pixmap for `client`'s frame window, or the
/// client itself when it is not wrapped. Callers must hold an error trap
/// around the whole capture sequence.
fn frame_target(client: Window) -> Window {
    unsafe {
        let display = xdisplay();
        let mut root: Window = 0;
        let mut parent: Window = 0;
        let mut children: *mut Window = std::ptr::null_mut();
        let mut n: std::os::raw::c_uint = 0;
        let ok = xlib::XQueryTree(
            display,
            client,
            &mut root,
            &mut parent,
            &mut children,
            &mut n,
        );
        if !children.is_null() {
            xlib::XFree(children as *mut c_void);
        }
        if ok != 0 && parent != root && parent != 0 {
            parent
        } else {
            client
        }
    }
}

struct WindowAttrs {
    visual: *mut xlib::Visual,
    width: i32,
    height: i32,
    viewable: bool,
}

/// `XGetWindowAttributes` for a window; `None` when the window is gone.
fn window_attributes(display: *mut Display, xid: Window) -> Option<WindowAttrs> {
    unsafe {
        let mut attr: xlib::XWindowAttributes = std::mem::zeroed();
        if xlib::XGetWindowAttributes(display, xid, &mut attr) != 0 {
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
}

fn free_pixmap(pix: xlib::Pixmap) {
    unsafe {
        error_trap_push();
        xlib::XFreePixmap(xdisplay(), pix);
        error_trap_pop_ignored();
    }
}

unsafe fn render_scaled(
    x: *mut Display,
    pix: xlib::Pixmap,
    visual: *mut xlib::Visual,
    src_w: i32,
    src_h: i32,
    dst_w: i32,
    dst_h: i32,
) -> Option<gdk_pixbuf::Pixbuf> {
    let src =
        cairo_xlib_surface_create(x, pix as c_ulong, visual, src_w, src_h);
    if src.is_null() {
        return None;
    }
    // from_raw_full takes ownership of the reference returned by
    // cairo_xlib_surface_create; from_raw_none adds a second one and leaks
    // the surface on every capture.
    let src = cairo::Surface::from_raw_full(src).ok()?;
    let dst = cairo::ImageSurface::create(cairo::Format::ARgb32, dst_w, dst_h).ok()?;
    let cr = cairo::Context::new(&dst).ok()?;
    cr.scale(dst_w as f64 / src_w as f64, dst_h as f64 / src_h as f64);
    let pattern = cairo::SurfacePattern::create(&src);
    pattern.set_filter(cairo::Filter::Bilinear);
    let _ = cr.set_source(&pattern);
    let _ = cr.paint();
    dst.flush();
    if dst.status().is_err() {
        return None;
    }
    gdk::pixbuf_get_from_surface(&dst, 0, 0, dst_w, dst_h)
}

/// Capture the window's current compositor frame, scaled to fit inside
/// `max_width` × `max_height` with the aspect ratio preserved.
///
/// `client` is an X window id; a stale or foreign id simply yields `None`.
pub fn capture_frame(
    client: Window,
    max_width: i32,
    max_height: i32,
) -> Option<gdk_pixbuf::Pixbuf> {
    if client == 0 {
        return None;
    }
    unsafe {
        let x = xdisplay();
        error_trap_push();
        let target = frame_target(client);
        let Some(attr) = window_attributes(x, target) else {
            error_trap_pop_ignored();
            return None;
        };
        if !attr.viewable || attr.width <= 0 || attr.height <= 0 {
            error_trap_pop_ignored();
            return None;
        }
        let pix = XCompositeNameWindowPixmap(x, target);
        xlib::XSync(x, xlib::False);
        if pix == 0 || error_trap_pop() {
            if pix != 0 {
                free_pixmap(pix);
            }
            return None;
        }

        // Bounded thumbnail size, aspect preserved.
        let max_height = max_height.max(1);
        let mut width = max_width.max(1);
        let mut height =
            1.max(((attr.height as f64) * width as f64 / attr.width as f64).round() as i32);
        if height > max_height {
            width = 1.max((width as f64 * max_height as f64 / height as f64).round() as i32);
            height = max_height;
        }

        error_trap_push();
        let result = render_scaled(x, pix, attr.visual, attr.width, attr.height, width, height);
        xlib::XSync(x, xlib::False);
        xlib::XFreePixmap(x, pix);
        if error_trap_pop() {
            return None;
        }
        result
    }
}

// ---------------------------------------------------------------------------
// Private XInput2 connection (raw scroll deltas)
// ---------------------------------------------------------------------------

/// One scroll device's axis mapping, resolved from XInput2.
#[derive(Clone, Copy)]
struct ScrollAxes {
    x_axis: i32,
    y_axis: i32,
    x_increment: f64,
    y_increment: f64,
}

impl Default for ScrollAxes {
    fn default() -> Self {
        ScrollAxes {
            x_axis: -1,
            y_axis: -1,
            x_increment: 0.0,
            y_increment: 0.0,
        }
    }
}

/// What the private connection reports to the dock.
pub enum InputEvent {
    /// A raw scroll delta with the pointer position in root coordinates.
    Scroll {
        x: i32,
        y: i32,
        delta_x: f64,
        delta_y: f64,
        time: u32,
    },
    /// The device list changed; cached axis mappings are stale.
    DevicesChanged,
}

// XIMaskLen from XInput2.h: bytes needed to hold a bit for each event type.
const fn ximask_len(event: i32) -> usize {
    ((event + 7) >> 3) as usize
}

/// A dedicated X connection for XInput2 raw events, independent of GTK's own
/// connection: its event selections never touch GDK.
pub struct XInputConnection {
    display: *mut Display,
    root: Window,
    opcode: c_int,
    axes: HashMap<i32, ScrollAxes>,
}

impl XInputConnection {
    /// Open the connection and negotiate XInput2. `None` when XInput2 is not
    /// available; the connection is closed again in that case.
    pub fn open() -> Option<XInputConnection> {
        unsafe {
            let display = xlib::XOpenDisplay(std::ptr::null());
            if display.is_null() {
                return None;
            }
            let mut opcode: c_int = 0;
            let mut event_code: c_int = 0;
            let mut error: c_int = 0;
            let mut major: c_int = 2;
            let mut minor: c_int = 1;
            let name = b"XInputExtension\0";
            if xlib::XQueryExtension(
                display,
                name.as_ptr() as *const std::os::raw::c_char,
                &mut opcode,
                &mut event_code,
                &mut error,
            ) == xlib::False
                || xinput2::XIQueryVersion(display, &mut major, &mut minor) != 0
                || (major == 2 && minor < 1)
            {
                xlib::XCloseDisplay(display);
                return None;
            }
            Some(XInputConnection {
                display,
                root: xlib::XDefaultRootWindow(display),
                opcode,
                axes: HashMap::new(),
            })
        }
    }

    /// The connection's file descriptor, for a GLib IO watch.
    pub fn connection_number(&self) -> c_int {
        unsafe { xlib::XConnectionNumber(self.display) }
    }

    /// Subscribe to raw motion plus device add/remove on the root window.
    pub fn select_events(&mut self) {
        unsafe {
            let mut raw = [0u8; ximask_len(xinput2::XI_LASTEVENT)];
            let mut change = [0u8; ximask_len(xinput2::XI_LASTEVENT)];
            xinput2::XISetMask(&mut raw, xinput2::XI_RawMotion);
            xinput2::XISetMask(&mut change, xinput2::XI_HierarchyChanged);
            xinput2::XISetMask(&mut change, xinput2::XI_DeviceChanged);
            let mut masks = [
                xinput2::XIEventMask {
                    deviceid: xinput2::XIAllMasterDevices,
                    mask_len: raw.len() as c_int,
                    mask: raw.as_mut_ptr(),
                },
                xinput2::XIEventMask {
                    deviceid: xinput2::XIAllDevices,
                    mask_len: change.len() as c_int,
                    mask: change.as_mut_ptr(),
                },
            ];
            xinput2::XISelectEvents(
                self.display,
                self.root,
                masks.as_mut_ptr(),
                masks.len() as c_int,
            );
            xlib::XFlush(self.display);
        }
    }

    /// Read and translate every pending event, in order.
    pub fn drain(&mut self, mut on_event: impl FnMut(InputEvent)) {
        unsafe {
            while xlib::XPending(self.display) > 0 {
                let mut event: xlib::XEvent = std::mem::zeroed();
                xlib::XNextEvent(self.display, &mut event);
                if event.type_ != xlib::GenericEvent {
                    continue;
                }
                if event.generic_event_cookie.extension != self.opcode {
                    continue;
                }
                if xlib::XGetEventData(self.display, &mut event.generic_event_cookie) == 0 {
                    continue;
                }
                let data = event.generic_event_cookie.data as *const xinput2::XIRawEvent;
                if !data.is_null() {
                    match (*data).evtype {
                        xinput2::XI_RawMotion => {
                            if let Some(scroll) = self.raw_motion(&*data) {
                                on_event(scroll);
                            }
                        }
                        xinput2::XI_HierarchyChanged | xinput2::XI_DeviceChanged => {
                            self.axes.clear();
                            on_event(InputEvent::DevicesChanged);
                        }
                        _ => {}
                    }
                }
                xlib::XFreeEventData(self.display, &mut event.generic_event_cookie);
            }
        }
    }

    fn scroll_axes(&mut self, source: i32) -> ScrollAxes {
        if let Some(axes) = self.axes.get(&source) {
            return *axes;
        }
        let mut axes = ScrollAxes::default();
        unsafe {
            let mut count: c_int = 0;
            let devices = xinput2::XIQueryDevice(self.display, source, &mut count);
            if !devices.is_null() {
                for j in 0..count as usize {
                    let device = &*devices.add(j);
                    for i in 0..device.num_classes as usize {
                        let class = *device.classes.add(i);
                        if (*class)._type != xinput2::XIScrollClass {
                            continue;
                        }
                        let scroll = &*(class as *const xinput2::XIScrollClassInfo);
                        if scroll.increment == 0.0 {
                            continue;
                        }
                        if scroll.scroll_type == xinput2::XIScrollTypeVertical {
                            axes.y_axis = scroll.number;
                            axes.y_increment = scroll.increment;
                        } else {
                            axes.x_axis = scroll.number;
                            axes.x_increment = scroll.increment;
                        }
                    }
                }
                xinput2::XIFreeDeviceInfo(devices);
            }
        }
        self.axes.insert(source, axes);
        axes
    }

    /// Translate a raw motion event into a scroll event over the pointer.
    fn raw_motion(&mut self, event: &xinput2::XIRawEvent) -> Option<InputEvent> {
        unsafe {
            let source = if event.sourceid != 0 {
                event.sourceid
            } else {
                event.deviceid
            };
            let axes = self.scroll_axes(source);
            let mut dx = 0.0f64;
            let mut dy = 0.0f64;
            let mut value = event.valuators.values;
            let bits = event.valuators.mask_len * 8;
            let mask =
                std::slice::from_raw_parts(event.valuators.mask, event.valuators.mask_len as usize);
            for i in 0..bits {
                if !xinput2::XIMaskIsSet(mask, i) {
                    continue;
                }
                if i == axes.x_axis && axes.x_increment != 0.0 {
                    dx = *value / axes.x_increment;
                }
                if i == axes.y_axis && axes.y_increment != 0.0 {
                    dy = *value / axes.y_increment;
                }
                value = value.add(1);
            }
            if dx == 0.0 && dy == 0.0 {
                return None;
            }
            let mut root: Window = 0;
            let mut child: Window = 0;
            let mut x = 0;
            let mut y = 0;
            let mut wx = 0;
            let mut wy = 0;
            let mut mask: std::os::raw::c_uint = 0;
            if xlib::XQueryPointer(
                self.display,
                self.root,
                &mut root,
                &mut child,
                &mut x,
                &mut y,
                &mut wx,
                &mut wy,
                &mut mask,
            ) == 0
            {
                return None;
            }
            Some(InputEvent::Scroll {
                x,
                y,
                delta_x: dx,
                delta_y: dy,
                time: event.time as u32,
            })
        }
    }
}

impl Drop for XInputConnection {
    fn drop(&mut self) {
        unsafe { xlib::XCloseDisplay(self.display) };
    }
}

// ---------------------------------------------------------------------------
// GDK current event time
// ---------------------------------------------------------------------------

/// Server-side time for the current event, or 0 when there is no display.
pub fn server_time() -> u32 {
    let Some(root) = gdk::Screen::default().and_then(|screen| screen.root_window()) else {
        return 0;
    };
    unsafe { gdk_x11_get_server_time(root.to_glib_none().0) as u32 }
}
