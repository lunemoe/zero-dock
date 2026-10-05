//! XComposite capture, frame cache, clickable previews and volume bubbles.
//! Ported from `src/preview.c`.

use crate::button::ScrollInfo;
use crate::dock::Dock;
use crate::ffi_x11;
use crate::ffi_xfce;
use crate::util::{monotonic_us, t};
use gdk::prelude::*;
use gtk::prelude::*;

const FRAME_BUDGET_BYTES: usize = 32 * 1024 * 1024;

/// Workarea of the monitor under a button, with the C code's fallbacks.
pub fn monitor_bounds(d: &Dock, button_id: u64) -> gdk::Rectangle {
    let mut bounds = gdk::Rectangle::new(0, 0, 1024, 768);
    let display = gdk::Display::default().unwrap();
    let mut monitor: Option<gdk::Monitor> = None;
    let Some(button) = d.button(button_id) else {
        return bounds;
    };
    let top = button.main.toplevel();
    if let Some(top) = top.as_ref() {
        if top.is_realized() {
            if let (Some((x, y)), Some(top_window)) =
                (button.main.translate_coordinates(top, 0, 0), top.window())
            {
                // gdk_window_get_origin binds as (screen-number, x, y).
                let (_, ox, oy) = top_window.origin();
                let a = button.main.allocation();
                monitor = display.monitor_at_point(ox + x + a.width() / 2, oy + y + a.height() / 2);
            }
        }
    }
    if monitor.is_none() && display.n_monitors() > 0 {
        monitor = display.monitor(0);
    }
    if let Some(monitor) = monitor {
        bounds = monitor.workarea();
    }
    bounds
}

impl Dock {
    /// Evict old cached frames beyond the per-instance budget, keeping
    /// `keep_id`'s most recent frame.
    pub fn trim_frames(&mut self, keep_id: u64) {
        let mut total: usize = self
            .buttons
            .iter()
            .map(|b| b.thumbnail.as_ref().map_or(0, |p| p.byte_length()))
            .sum();
        while total > FRAME_BUDGET_BYTES {
            let mut oldest: Option<(u64, i64, usize)> = None;
            for b in &self.buttons {
                if b.id == keep_id {
                    continue;
                }
                let Some(thumbnail) = b.thumbnail.as_ref() else {
                    continue;
                };
                let newer = oldest
                    .as_ref()
                    .is_none_or(|(_, time, _)| b.thumbnail_time < *time);
                if newer {
                    oldest = Some((b.id, b.thumbnail_time, thumbnail.byte_length()));
                }
            }
            let Some((id, _, size)) = oldest else { break };
            if let Some(b) = self.button_mut(id) {
                b.thumbnail = None;
                b.thumbnail_time = 0;
            }
            total = total.saturating_sub(size);
        }
    }

    /// Capture the window's compositor frame into the button's cache and
    /// return a reference copy, or the cached frame for minimized windows.
    pub fn capture(&mut self, button_id: u64) -> Option<gdk_pixbuf::Pixbuf> {
        let (window, minimized) = self.button(button_id).and_then(|b| {
            b.window
                .as_ref()
                .map(|w| (w.clone(), ffi_xfce::Window::new(w).is_minimized()))
        })?;
        if !minimized {
            let result = self.capture_live(button_id, &window);
            if let Some(pixbuf) = result {
                if let Some(b) = self.button_mut(button_id) {
                    b.thumbnail = Some(pixbuf);
                    b.thumbnail_time = monotonic_us();
                }
                self.trim_frames(button_id);
            }
        }
        self.button(button_id).and_then(|b| b.thumbnail.clone())
    }

    fn capture_live(
        &mut self,
        button_id: u64,
        window: &glib::Object,
    ) -> Option<gdk_pixbuf::Pixbuf> {
        unsafe {
            let x = ffi_x11::xdisplay();
            let client = ffi_xfce::Window::new(window).xid();

            ffi_x11::error_trap_push();
            let target = ffi_x11::frame_target(client);
            let Some(attr) = ffi_x11::get_window_attributes(x, target) else {
                ffi_x11::error_trap_pop_ignored();
                return None;
            };
            if !attr.viewable || attr.width <= 0 || attr.height <= 0 {
                ffi_x11::error_trap_pop_ignored();
                return None;
            }
            let pix = ffi_x11::XCompositeNameWindowPixmap(x, target);
            x11::xlib::XSync(x, x11::xlib::False);
            if pix == 0 || ffi_x11::error_trap_pop() {
                if pix != 0 {
                    ffi_x11::free_pixmap(pix);
                }
                return None;
            }

            // Compute the bounded thumbnail size (aspect preserved).
            let bounds = monitor_bounds(self, button_id);
            let preview_width = self.settings.preview_width;
            let mut width = (preview_width as i32).min((bounds.width() - 48).max(1));
            let mut height =
                1.max(((attr.height as f64) * width as f64 / attr.width as f64).round() as i32);
            let max_height = 1.max(400.min(bounds.height() - 160));
            if height > max_height {
                width = 1.max((width as f64 * max_height as f64 / height as f64).round() as i32);
                height = max_height;
            }

            ffi_x11::error_trap_push();
            let result = render_scaled(x, pix, attr.visual, attr.width, attr.height, width, height);
            x11::xlib::XSync(x, x11::xlib::False);
            x11::xlib::XFreePixmap(x, pix);
            if ffi_x11::error_trap_pop() {
                return None;
            }
            result
        }
    }
}

unsafe fn render_scaled(
    x: *mut ffi_x11::Display,
    pix: x11::xlib::Pixmap,
    visual: *mut x11::xlib::Visual,
    src_w: i32,
    src_h: i32,
    dst_w: i32,
    dst_h: i32,
) -> Option<gdk_pixbuf::Pixbuf> {
    let src =
        ffi_x11::cairo_xlib_surface_create(x, pix as std::os::raw::c_ulong, visual, src_w, src_h);
    if src.is_null() {
        return None;
    }
    let src = cairo::Surface::from_raw_none(src);
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

// ---------------------------------------------------------------------------
// Preview popup
// ---------------------------------------------------------------------------

impl Dock {
    fn unblock_autohide(&mut self) {
        if self.autohide_blocked {
            if let Some(plugin) = &self.plugin {
                plugin.block_autohide(false);
            }
            self.autohide_blocked = false;
        }
    }

    pub fn hide_preview(&mut self) {
        self.hover_id.clear();
        self.preview_tick.clear();
        self.leave_id.clear();
        if let Some(preview) = self.preview.as_ref() {
            preview.hide();
        }
        self.hover_button = None;
        self.unblock_autohide();
    }

    pub fn maybe_leave_preview(&mut self) {
        if !self.leave_id.is_set() {
            let weak = self.weak();
            self.leave_id.set_timeout(30, move || {
                let Some(rc) = weak.upgrade() else {
                    return glib::ControlFlow::Break;
                };
                crate::util::with_dock(&rc, |d| d.check_leave());
                glib::ControlFlow::Break
            });
        }
    }

    fn check_leave(&mut self) {
        self.leave_id.take();
        let Some(hover) = self.hover_button else {
            return;
        };
        let Some((x, y)) = pointer_position() else {
            return;
        };
        let main_anchor = self
            .button(hover)
            .map(|b| b.main.clone().upcast::<gtk::Widget>());
        let in_main = main_anchor.is_some_and(|w| popup_contains(&w, x, y, 4));
        let in_preview = self
            .preview
            .as_ref()
            .is_some_and(|p| popup_contains(p.upcast_ref(), x, y, 4));
        if !in_main && !in_preview {
            self.hide_preview();
        }
    }

    pub fn schedule_preview(&mut self, button_id: u64) {
        if self.hover_button == Some(button_id) {
            if self.leave_id.is_set() {
                self.leave_id.clear();
            }
            return;
        }
        self.hide_preview();
        if self.button(button_id).is_none_or(|b| b.window.is_none())
            || !self.settings.previews
            || self.dragging
        {
            return;
        }
        self.hover_button = Some(button_id);
        let weak = self.weak();
        self.hover_id
            .set_timeout(self.settings.preview_delay as u64, move || {
                let Some(rc) = weak.upgrade() else {
                    return glib::ControlFlow::Break;
                };
                crate::util::with_dock(&rc, |d| d.show_preview());
                glib::ControlFlow::Break
            });
    }

    fn show_preview(&mut self) {
        self.hover_id.take();
        if self.hover_button.is_none()
            || !self.settings.previews
            || self.dragging
            || self.screen.as_ref().is_some_and(|s| s.show_desktop())
        {
            return;
        }
        if self.preview.is_none() {
            self.preview = Some(self.popup_new());
            let preview = self.preview.as_ref().unwrap();
            let box_ = gtk::Box::new(gtk::Orientation::Vertical, 6);
            box_.set_border_width(6);
            let image_box = gtk::EventBox::new();
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            preview.set_child(Some(&box_));
            let image = gtk::Image::new();
            let hover = self.hover_button;
            let bounds = hover
                .map(|id| monitor_bounds(self, id))
                .unwrap_or_else(|| gdk::Rectangle::new(0, 0, 1024, 768));
            image_box.set_size_request(
                (self.settings.preview_width as i32).min((bounds.width() - 48).max(1)),
                140.min((bounds.height() - 160).max(1)),
            );
            image_box.set_child(Some(&image));
            box_.pack_start(&image_box, false, false, 0);
            let title = gtk::Label::new(None);
            title.set_ellipsize(pango::EllipsizeMode::End);
            title.set_max_width_chars(35);
            title.set_xalign(0.0);
            row.pack_start(&title, true, true, 0);
            let sound = gtk::Button::new();
            let close =
                gtk::Button::from_icon_name(Some("window-close-symbolic"), gtk::IconSize::Button);
            close.set_tooltip_text(Some(&t("关闭窗口")));
            if let Some(accessible) = close.accessible() {
                accessible.set_name(t("关闭窗口").as_str());
            }
            {
                let weak = self.weak();
                close.connect_clicked(move |_| {
                    if let Some(rc) = weak.upgrade() {
                        crate::util::with_dock(&rc, |d| {
                            if let Some(hover) = d.hover_button {
                                d.close_window(hover);
                            }
                        });
                    }
                });
            }
            row.pack_end(&close, false, false, 0);
            row.pack_end(&sound, false, false, 0);
            box_.pack_start(&row, false, false, 0);
            let workspace = gtk::Label::new(None);
            box_.pack_start(&workspace, false, false, 0);
            let status = gtk::Label::new(None);
            box_.pack_start(&status, false, false, 0);
            image_box.add_events(gdk::EventMask::BUTTON_PRESS_MASK);
            {
                let weak = self.weak();
                image_box.connect_button_press_event(move |_w, e| {
                    if let Some(rc) = weak.upgrade() {
                        crate::util::with_dock(&rc, |d| {
                            let Some(hover) = d.hover_button else { return };
                            if e.button() == 1 {
                                d.activate(hover);
                                d.hide_preview();
                            } else if e.button() == 2 {
                                d.launch(hover);
                            }
                        });
                    }
                    glib::Propagation::Proceed
                });
            }
            {
                let weak = self.weak();
                sound.connect_clicked(move |_| {
                    if let Some(rc) = weak.upgrade() {
                        crate::util::with_dock(&rc, |d| {
                            if let Some(hover) = d.hover_button {
                                d.audio_mute(hover);
                                d.volume_bubble(hover);
                            }
                        });
                    }
                });
            }
            sound.add_events(gdk::EventMask::SCROLL_MASK | gdk::EventMask::SMOOTH_SCROLL_MASK);
            {
                let weak = self.weak();
                sound.connect_scroll_event(move |_w, e| {
                    if let Some(rc) = weak.upgrade() {
                        crate::util::with_dock(&rc, |d| {
                            if let Some(hover) = d.hover_button {
                                let info = ScrollInfo::from_event(e);
                                d.audio_scroll(hover, &info);
                            }
                        });
                    }
                    glib::Propagation::Stop
                });
            }
            {
                let weak = self.weak();
                preview.connect_enter_notify_event(move |_w, _e| {
                    if let Some(rc) = weak.upgrade() {
                        crate::util::with_dock(&rc, |d| d.leave_id.clear());
                    }
                    glib::Propagation::Proceed
                });
            }
            {
                let weak = self.weak();
                preview.connect_leave_notify_event(move |_w, e| {
                    if let Some(rc) = weak.upgrade() {
                        crate::util::with_dock(&rc, |d| {
                            if e.detail() != gdk::NotifyType::Inferior {
                                d.maybe_leave_preview();
                            }
                        });
                    }
                    glib::Propagation::Proceed
                });
            }
            self.preview_image = Some(image);
            self.preview_title = Some(title);
            self.preview_sound = Some(sound);
            self.preview_close = Some(close);
            self.preview_status = Some(status);
            self.preview_workspace = Some(workspace);
        }
        self.update_preview();
        self.preview.as_ref().unwrap().show_all();
        if let Some(hover) = self.hover_button {
            let anchor = self.button(hover).map(|b| b.main.clone());
            if let Some(anchor) = anchor {
                self.position_popup(self.preview.as_ref().unwrap(), anchor.upcast_ref());
            }
        }
        if let Some(plugin) = &self.plugin {
            plugin.block_autohide(true);
        }
        self.autohide_blocked = true;
        let weak = self.weak();
        let interval = self.settings.preview_interval as u64;
        self.preview_tick.set_timeout(interval, move || {
            let Some(rc) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            match crate::util::with_dock(&rc, |d| d.preview_tick_step()) {
                Some(flow) => flow,
                None => glib::ControlFlow::Break,
            }
        });
    }

    fn preview_tick_step(&mut self) -> glib::ControlFlow {
        if self.hover_button.is_none() {
            self.preview_tick.take();
            return glib::ControlFlow::Break;
        }
        self.update_preview();
        glib::ControlFlow::Continue
    }

    pub fn preview_update_audio(&mut self) {
        let Some(hover) = self.hover_button else {
            return;
        };
        let Some(sound) = self.preview_sound.clone() else {
            return;
        };
        let status = self.audio_status(hover);
        sound.set_sensitive(status.present);
        sound.set_image(Some(&gtk::Image::from_icon_name(
            Some(if status.muted {
                "audio-volume-muted-symbolic"
            } else {
                "audio-volume-high-symbolic"
            }),
            gtk::IconSize::Button,
        )));
        sound.set_tooltip_text(Some(
            if status.muted {
                t("取消应用静音")
            } else {
                t("应用静音")
            }
            .as_str(),
        ));
    }

    fn update_preview(&mut self) {
        let Some(hover) = self.hover_button else {
            return;
        };
        let Some((window, icon)) = self
            .button(hover)
            .map(|b| (b.window.clone(), b.icon.clone()))
        else {
            return;
        };
        let Some(window) = window else { return };
        let w = ffi_xfce::Window::new(&window);
        let bounds = monitor_bounds(self, hover);
        let max_width = (self.settings.preview_width as i32).min((bounds.width() - 48).max(1));
        let max_height = 1.max(400.min(bounds.height() - 160));
        if let Some(image) = self.preview_image.as_ref() {
            if let Some(parent) = image.parent() {
                parent.set_size_request(max_width, 140.min(max_height));
            }
        }
        let mut pixbuf = self.capture(hover);
        if let Some(p) = pixbuf.as_ref() {
            if p.width() > max_width || p.height() > max_height {
                let ratio = (max_width as f64 / p.width() as f64)
                    .min(max_height as f64 / p.height() as f64);
                pixbuf = p.scale_simple(
                    1.max((p.width() as f64 * ratio) as i32),
                    1.max((p.height() as f64 * ratio) as i32),
                    gdk_pixbuf::InterpType::Bilinear,
                );
            }
        }
        if let Some(image) = self.preview_image.as_ref() {
            match pixbuf.as_ref() {
                Some(p) => image.set_from_pixbuf(Some(p)),
                None => image.set_from_pixbuf(icon.as_ref()),
            }
        }
        if let Some(status) = self.preview_status.as_ref() {
            let text = if pixbuf.is_some() {
                if w.is_minimized() {
                    t("已最小化 · 最后一帧")
                } else {
                    t("点击预览切换窗口")
                }
            } else {
                t("暂无可用画面 · 点击恢复窗口")
            };
            status.set_text(&text);
        }
        let name = w.name().unwrap_or_default();
        if let Some(title) = self.preview_title.as_ref() {
            title.set_text(&name);
            title.set_tooltip_text(Some(&name));
        }
        let workspace = w
            .workspace()
            .and_then(|ws| ffi_xfce::Workspace::name(&ws))
            .unwrap_or_else(|| t("所有工作区").to_string());
        if let Some(label) = self.preview_workspace.as_ref() {
            label.set_text(&workspace);
        }
        self.preview_update_audio();
        if self.preview.as_ref().is_some_and(|p| p.is_visible()) {
            if let Some(anchor) = self.button(hover).map(|b| b.main.clone()) {
                let preview = self.preview.as_ref().unwrap();
                self.position_popup(preview, anchor.upcast_ref());
            }
        }
    }

    // -- volume bubble -------------------------------------------------------

    pub fn volume_bubble(&mut self, button_id: u64) {
        if self.button(button_id).is_none() {
            return;
        }
        if self.bubble.is_none() {
            self.bubble = Some(self.popup_new());
            let bubble = self.bubble.as_ref().unwrap();
            let box_ = gtk::Box::new(gtk::Orientation::Vertical, 5);
            box_.set_border_width(8);
            box_.set_size_request(150, -1);
            bubble.set_child(Some(&box_));
            let text = gtk::Label::new(None);
            let bar = gtk::ProgressBar::new();
            box_.pack_start(&text, false, false, 0);
            box_.pack_start(&bar, false, false, 0);
            self.bubble_text = Some(text);
            self.bubble_bar = Some(bar);
        }
        let status = self.audio_status(button_id);
        let text = if status.muted {
            t("应用已静音 · %u%%")
                .replace("%u", &status.percent.to_string())
                .replace("%%", "%")
        } else {
            t("应用音量 %u%%")
                .replace("%u", &status.percent.to_string())
                .replace("%%", "%")
        };
        if let Some(label) = self.bubble_text.as_ref() {
            label.set_text(&text);
        }
        if let Some(bar) = self.bubble_bar.as_ref() {
            bar.set_fraction((status.percent as f64 / 200.0).min(1.0));
        }
        if let Some(bubble) = self.bubble.as_ref() {
            bubble.show_all();
        }
        if let (Some(bubble), Some(anchor)) = (
            self.bubble.clone(),
            self.button(button_id).map(|b| b.main.clone()),
        ) {
            self.position_popup(&bubble, anchor.upcast_ref());
        }
        let weak = self.weak();
        self.bubble_id.set_timeout(1200, move || {
            if let Some(rc) = weak.upgrade() {
                crate::util::with_dock(&rc, |d| {
                    if let Some(bubble) = d.bubble.as_ref() {
                        bubble.hide();
                    }
                });
            }
            glib::ControlFlow::Break
        });
    }

    // -- popup infrastructure -------------------------------------------------

    fn popup_new(&self) -> gtk::Window {
        let popup = gtk::Window::new(gtk::WindowType::Popup);
        popup.set_accept_focus(false);
        popup.set_resizable(false);
        popup.set_type_hint(gdk::WindowTypeHint::Tooltip);
        popup.style_context().add_class("zd-popup");
        popup
            .style_context()
            .add_provider(&self.css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
        if let Some(plugin) = &self.plugin {
            plugin.take_window(&popup);
        }
        popup
    }

    /// Place a popup beside `anchor`, clamped to the anchor monitor's
    /// workarea (port of `position_popup`).
    pub fn position_popup(&self, popup: &gtk::Window, anchor: &gtk::Widget) {
        let Some(top) = anchor.toplevel() else { return };
        let Some((ax, ay)) = anchor.translate_coordinates(&top, 0, 0) else {
            return;
        };
        let Some(top_window) = top.window() else {
            return;
        };
        // gdk_window_get_origin binds as (screen-number, x, y).
        let (_, ox, oy) = top_window.origin();
        let ax = ax + ox;
        let ay = ay + oy;
        let allocation = anchor.allocation();
        let (_, natural) = popup.preferred_size();
        let width = natural.width;
        let height = natural.height;
        let display = gdk::Display::default().unwrap();
        let monitor =
            display.monitor_at_point(ax + allocation.width() / 2, ay + allocation.height() / 2);
        let mut bounds = gdk::Rectangle::new(0, 0, 1024, 768);
        if let Some(monitor) = monitor {
            bounds = monitor.workarea();
        }
        let (x, y);
        if self.orientation == gtk::Orientation::Horizontal {
            let mut px = ax + (allocation.width() - width) / 2;
            let mut py = ay + allocation.height() + 3;
            if ay + allocation.height() / 2 > bounds.y() + bounds.height() / 2 {
                py = ay - height - 3;
            }
            px = px.clamp(
                bounds.x() + 3,
                (bounds.x() + bounds.width() - width - 3).max(bounds.x() + 3),
            );
            py = py.clamp(
                bounds.y() + 3,
                (bounds.y() + bounds.height() - height - 3).max(bounds.y() + 3),
            );
            x = px;
            y = py;
        } else {
            let mut px = ax + allocation.width() + 3;
            let mut py = ay + (allocation.height() - height) / 2;
            if ax + allocation.width() / 2 > bounds.x() + bounds.width() / 2 {
                px = ax - width - 3;
            }
            px = px.clamp(
                bounds.x() + 3,
                (bounds.x() + bounds.width() - width - 3).max(bounds.x() + 3),
            );
            py = py.clamp(
                bounds.y() + 3,
                (bounds.y() + bounds.height() - height - 3).max(bounds.y() + 3),
            );
            x = px;
            y = py;
        }
        popup.resize(width, height);
        popup.move_(x, y);
    }

    pub fn popups_dispose(&mut self) {
        self.hide_preview();
        self.bubble_id.clear();
        if let Some(preview) = self.preview.take() {
            unsafe {
                preview.destroy();
            }
        }
        if let Some(bubble) = self.bubble.take() {
            unsafe {
                bubble.destroy();
            }
        }
        self.preview_image = None;
        self.preview_title = None;
        self.preview_sound = None;
        self.preview_close = None;
        self.preview_status = None;
        self.preview_workspace = None;
        self.bubble_text = None;
        self.bubble_bar = None;
    }
}

/// Pointer position in root coordinates.
fn pointer_position() -> Option<(i32, i32)> {
    let display = gdk::Display::default()?;
    let seat = display.default_seat()?;
    let pointer = seat.pointer()?;
    let (_, px, py) = pointer.position();
    Some((px, py))
}

/// Root-coordinate rectangle test with padding (port of `contains`).
fn popup_contains(widget: &gtk::Widget, x: i32, y: i32, pad: i32) -> bool {
    if !widget.is_visible() || !widget.is_realized() {
        return false;
    }
    let Some(window) = widget.window() else {
        return false;
    };
    // gdk_window_get_origin binds as (screen-number, x, y).
    let (_, mut ox, mut oy) = window.origin();
    let a = widget.allocation();
    if !widget.has_window() {
        ox += a.x();
        oy += a.y();
    }
    x >= ox - pad && x < ox + a.width() + pad && y >= oy - pad && y < oy + a.height() + pad
}
