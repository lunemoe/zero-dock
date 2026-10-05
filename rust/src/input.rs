//! Separate XInput2 connection, raw scroll device handling and speaker hit
//! testing. Ported from `src/input.c`.
//!
//! The connection is deliberately independent from GTK's X connection (its
//! event selections never touch GDK), and the fd source is removed by `Drop`
//! before the display is closed.

use crate::dock::Dock;
use crate::util::DockWeak;
use glib::IOCondition;
use gtk::prelude::*;
use std::collections::HashMap;
use std::os::raw::{c_char, c_int, c_uint};
use x11::xinput2;
use x11::xlib;

#[derive(Clone, Copy, Default, Debug)]
struct ScrollAxes {
    x_axis: i32,
    y_axis: i32,
    x_increment: f64,
    y_increment: f64,
}

pub struct Input {
    display: *mut xlib::Display,
    root: xlib::Window,
    opcode: c_int,
    source: Option<glib::SourceId>,
    axes: HashMap<i32, ScrollAxes>,
}

// XIMaskLen from XInput2.h: bytes needed to hold a bit for each event type.
const fn ximask_len(event: i32) -> usize {
    (((event) + 7) >> 3) as usize
}

impl Drop for Input {
    fn drop(&mut self) {
        if let Some(source) = self.source.take() {
            source.remove();
        }
        unsafe {
            xlib::XCloseDisplay(self.display);
        }
    }
}

impl Input {
    pub fn new(weak: DockWeak) -> Option<Input> {
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
                name.as_ptr() as *const c_char,
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
            let root = xlib::XDefaultRootWindow(display);

            let mut input = Input {
                display,
                root,
                opcode,
                source: None,
                axes: HashMap::new(),
            };

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
            xinput2::XISelectEvents(display, root, masks.as_mut_ptr(), masks.len() as c_int);
            xlib::XFlush(display);

            let fd = xlib::XConnectionNumber(display);
            let weak_fd = weak.clone();
            let source = glib::unix_fd_add_local(
                fd,
                IOCondition::IN | IOCondition::HUP | IOCondition::ERR,
                move |_fd, condition| {
                    if condition.contains(IOCondition::HUP) || condition.contains(IOCondition::ERR)
                    {
                        // Connection died: detach the source without removing
                        // it from inside its own callback.
                        if let Some(rc) = weak_fd.upgrade() {
                            crate::util::with_dock(&rc, |d| {
                                if let Some(input) = d.input.as_mut() {
                                    input.source = None;
                                }
                            });
                        }
                        return glib::ControlFlow::Break;
                    }
                    if let Some(rc) = weak_fd.upgrade() {
                        crate::util::with_dock(&rc, |d| d.pump_input());
                    }
                    glib::ControlFlow::Continue
                },
            );
            input.source = Some(source);
            Some(input)
        }
    }

    /// Read and dispatch pending X events on the private connection.
    pub fn drain(&mut self, dock: &mut Dock) {
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
                        xinput2::XI_RawMotion => self.raw_motion(dock, &*data),
                        xinput2::XI_HierarchyChanged | xinput2::XI_DeviceChanged => {
                            self.axes.clear();
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
        let mut axes = ScrollAxes {
            x_axis: -1,
            y_axis: -1,
            x_increment: 0.0,
            y_increment: 0.0,
        };
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

    unsafe fn raw_motion(&mut self, dock: &mut Dock, event: &xinput2::XIRawEvent) {
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
        for i in 0..bits {
            if !xinput2::XIMaskIsSet(
                std::slice::from_raw_parts(event.valuators.mask, event.valuators.mask_len as usize),
                i,
            ) {
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
            return;
        }
        let mut root: xlib::Window = 0;
        let mut child: xlib::Window = 0;
        let mut x = 0;
        let mut y = 0;
        let mut wx = 0;
        let mut wy = 0;
        let mut mask: c_uint = 0;
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
        ) != 0
        {
            dock.input_delta(x, y, dx, dy, event.time as u32);
        }
    }
}

impl Dock {
    fn pump_input(&mut self) {
        if self.disposing {
            return;
        }
        // Temporarily take the input handler out so `drain` can borrow the
        // whole dock (the fd source only fires on the main loop).
        if let Some(mut input) = self.input.take() {
            input.drain(self);
            self.input = Some(input);
        }
    }

    /// A raw scroll event over one of this instance's visible speaker
    /// controls becomes an audio action.
    pub fn input_delta(&mut self, x: i32, y: i32, dx: f64, dy: f64, time: u32) {
        if self.disposing
            || self.dragging
            || !dx.is_finite()
            || !dy.is_finite()
            || (dx == 0.0 && dy == 0.0)
        {
            return;
        }
        let mut target: Option<u64> = None;
        if let (Some(hover), Some(sound)) = (self.hover_button, self.preview_sound.as_ref()) {
            if sound.is_visible() && self.widget_contains_sound(sound, x, y) {
                target = Some(hover);
            }
        }
        if target.is_none() {
            for b in &self.buttons {
                if b.sound.is_visible() && self.widget_contains_sound(&b.sound, x, y) {
                    target = Some(b.id);
                    break;
                }
            }
        }
        let Some(target) = target else { return };
        let event = crate::button::ScrollInfo {
            direction: gdk::ScrollDirection::Smooth,
            delta_x: dx,
            delta_y: dy,
            time,
            x: 0.0,
            y: 0.0,
            x_root: x as f64,
            y_root: y as f64,
        };
        self.audio_scroll(target, &event);
    }

    /// Hit-test a sound button in root coordinates (port of `contains()`).
    fn widget_contains_sound(&self, sound: &gtk::Button, x: i32, y: i32) -> bool {
        if !sound.is_mapped() {
            return false;
        }
        let top = sound.toplevel();
        let Some(top) = top else { return false };
        let Some((wx, wy)) = sound.translate_coordinates(&top, 0, 0) else {
            return false;
        };
        let Some(top_window) = top.window() else {
            return false;
        };
        // gdk_window_get_origin binds as (screen-number, x, y).
        let (_, ox, oy) = top_window.origin();
        let a = sound.allocation();
        x >= ox + wx && x < ox + wx + a.width() && y >= oy + wy && y < oy + wy + a.height()
    }
}
