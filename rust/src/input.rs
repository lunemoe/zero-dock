//! XInput2 raw scroll handling and speaker hit testing. Ported from
//! `src/input.c`.
//!
//! The X connection itself (open, event selection, translation) lives in
//! [`crate::ffi_x11::XInputConnection`]; this module owns the GLib IO watch and
//! the dock-side dispatch, so it stays safe Rust.

use crate::dock::Dock;
use crate::ffi_x11::{InputEvent, XInputConnection};
use crate::util::DockWeak;
use glib::IOCondition;
use gtk::prelude::*;

pub struct Input {
    connection: Option<XInputConnection>,
    source: Option<glib::SourceId>,
}

impl Drop for Input {
    fn drop(&mut self) {
        // Detach the fd source before the connection is closed with it.
        if let Some(source) = self.source.take() {
            source.remove();
        }
    }
}

impl Input {
    pub fn new(weak: DockWeak) -> Option<Input> {
        let mut connection = XInputConnection::open()?;
        connection.select_events();
        let fd = connection.connection_number();
        let weak_fd = weak.clone();
        let source = glib::unix_fd_add_local(
            fd,
            IOCondition::IN | IOCondition::HUP | IOCondition::ERR,
            move |_fd, condition| {
                if condition.contains(IOCondition::HUP) || condition.contains(IOCondition::ERR) {
                    // Connection died: detach the source without removing it
                    // from inside its own callback.
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
        Some(Input {
            connection: Some(connection),
            source: Some(source),
        })
    }
}

impl Dock {
    fn pump_input(&mut self) {
        if self.disposing {
            return;
        }
        // Temporarily take the input handler out so draining can borrow the
        // whole dock (the fd source only fires on the main loop).
        if let Some(mut input) = self.input.take() {
            if let Some(connection) = input.connection.as_mut() {
                connection.drain(|event| match event {
                    InputEvent::Scroll {
                        x,
                        y,
                        delta_x,
                        delta_y,
                        time,
                    } => self.input_delta(x, y, delta_x, delta_y, time),
                    // Device list changed; cached axis mappings were dropped.
                    InputEvent::DevicesChanged => {}
                });
            }
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
