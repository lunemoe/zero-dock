//! Buttons: identity, icon drawing, interaction, pinning, ordering, drag and
//! drop. Ported from `src/buttons.c` and the button parts of `plugin.c`.
//!
//! Every signal handler captures a `Weak` dock handle plus the button's id.
//! The id is resolved through the live button list on each event, so a freed
//! button simply makes the handler a no-op — the C code's `b->closed` checks
//! and "never dereference a payload pointer" rules are enforced by design
//! here, not by discipline.

use crate::dock::Dock;
use crate::ffi_gtk;
use crate::ffi_x11;
use crate::ffi_xfce;
use crate::settings::is_absolute_desktop_path;
use crate::util::{monotonic_us, t, DockWeak};
use gdk::keys::constants as Key;
use gdk::prelude::*;
use gtk::prelude::*;

/// A normalized scroll event: built from GDK events by handlers and from raw
/// coordinates by the XInput2 path, so audio code never touches GDK events.
#[derive(Clone, Copy, Debug)]
pub struct ScrollInfo {
    pub direction: gdk::ScrollDirection,
    pub delta_x: f64,
    pub delta_y: f64,
    pub time: u32,
    pub x: f64,
    pub y: f64,
    pub x_root: f64,
    pub y_root: f64,
}

impl ScrollInfo {
    pub fn from_event(e: &gdk::EventScroll) -> ScrollInfo {
        let (dx, dy) = e.delta();
        let (rx, ry) = e.root();
        ScrollInfo {
            direction: e.direction(),
            delta_x: dx,
            delta_y: dy,
            time: e.time(),
            x: e.position().0,
            y: e.position().1,
            x_root: rx,
            y_root: ry,
        }
    }
}

pub struct Button {
    pub id: u64,
    // Identity
    pub window: Option<glib::Object>, // XfwWindow
    pub window_handlers: Vec<glib::SignalHandlerId>,
    pub xwindow: Option<gdk::Window>,
    pub property_watch: Option<ffi_x11::PropertyWatch>,
    pub app: Option<gio::DesktopAppInfo>,
    pub desktop: Option<String>,
    pub key: String,
    // Widgets
    pub widget: gtk::Overlay,
    pub main: gtk::Button,
    pub drawing: gtk::DrawingArea,
    pub sound: gtk::Button,
    // Display state
    pub icon: Option<gdk_pixbuf::Pixbuf>,
    pub thumbnail: Option<gdk_pixbuf::Pixbuf>,
    pub audio: crate::audio::AudioStatus,
    pub pinned: bool,
    pub icon_dirty: bool,
    pub match_dirty: bool,
    pub eligible: bool,
    pub number: u32,
    pub icon_pixels: u32,
    pub icon_scale: u32,
    pub match_generation: u32,
    pub launching: bool,
    pub launch_until: i64,
    pub thumbnail_time: i64,
    pub launch_pid: i32,
    pub pid: i32,
    pub startup_id: Option<String>,
    pub launch_error: Option<String>,
    pub audio_scroll: f64,
    pub window_scroll: f64,
    pub last_audio_time: u32,
}

impl Button {
    fn new(id: u64, css: &gtk::CssProvider) -> Button {
        let widget = gtk::Overlay::new();
        let main = gtk::Button::new();
        let drawing = gtk::DrawingArea::new();
        main.set_child(Some(&drawing));
        widget.set_child(Some(&main));
        let sound = gtk::Button::new();
        sound.set_halign(gtk::Align::End);
        sound.set_valign(gtk::Align::Start);
        sound.set_size_request(18, 18);
        sound.style_context().add_class("zd-sound");
        widget.add_overlay(&sound);
        sound.set_no_show_all(true);
        main.style_context()
            .add_provider(css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
        sound
            .style_context()
            .add_provider(css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
        main.add_events(
            gdk::EventMask::SCROLL_MASK
                | gdk::EventMask::SMOOTH_SCROLL_MASK
                | gdk::EventMask::ENTER_NOTIFY_MASK
                | gdk::EventMask::LEAVE_NOTIFY_MASK,
        );
        sound.add_events(gdk::EventMask::SCROLL_MASK | gdk::EventMask::SMOOTH_SCROLL_MASK);
        widget.add_events(gdk::EventMask::SCROLL_MASK | gdk::EventMask::SMOOTH_SCROLL_MASK);
        main.set_can_focus(true);
        Button {
            id,
            window: None,
            window_handlers: Vec::new(),
            xwindow: None,
            property_watch: None,
            app: None,
            desktop: None,
            key: String::new(),
            widget,
            main,
            drawing,
            sound,
            icon: None,
            thumbnail: None,
            audio: crate::audio::AudioStatus::default(),
            pinned: false,
            icon_dirty: true,
            match_dirty: false,
            eligible: false,
            number: 0,
            icon_pixels: 0,
            icon_scale: 0,
            match_generation: 0,
            launching: false,
            launch_until: 0,
            thumbnail_time: 0,
            launch_pid: 0,
            pid: 0,
            startup_id: None,
            launch_error: None,
            audio_scroll: 0.0,
            window_scroll: 0.0,
            last_audio_time: 0,
        }
    }

    /// Destroy the button's widgets (equivalent of `zd_button_free` UI part).
    pub fn destroy_widgets(&mut self) {
        ffi_gtk::destroy(&self.widget);
    }

    pub fn window_xid(&self) -> ffi_x11::Window {
        self.window
            .as_ref()
            .map(|w| ffi_xfce::Window::new(w).xid())
            .unwrap_or(0)
    }

    pub fn display_name(&self) -> String {
        if let Some(window) = &self.window {
            return ffi_xfce::Window::new(window)
                .name()
                .unwrap_or_else(|| t("无标题窗口").to_string());
        }
        if let Some(app) = &self.app {
            return app.upcast_ref::<gio::AppInfo>().display_name().to_string();
        }
        t("启动器文件已失效").to_string()
    }
}

// ---------------------------------------------------------------------------
// Dock helpers around the button list
// ---------------------------------------------------------------------------

impl Dock {
    pub fn button(&self, id: u64) -> Option<&Button> {
        self.buttons.iter().find(|b| b.id == id)
    }

    pub fn button_mut(&mut self, id: u64) -> Option<&mut Button> {
        self.buttons.iter_mut().find(|b| b.id == id)
    }

    pub fn button_index(&self, id: u64) -> Option<usize> {
        self.buttons.iter().position(|b| b.id == id)
    }

    fn alloc_button_id(&mut self) -> u64 {
        self.next_button_id += 1;
        self.next_button_id
    }

    fn create_button(&mut self) -> u64 {
        let id = self.alloc_button_id();
        let mut button = Button::new(id, &self.css);
        let weak = self.weak();
        wire_button(&mut button, weak);
        self.box_.pack_start(&button.widget, false, false, 0);
        button.widget.show_all();
        button.widget.set_no_show_all(true);
        self.buttons.push(button);
        id
    }

    /// Add a pin button for an absolute .desktop path.
    pub fn add_pin(&mut self, path: &str) -> Option<u64> {
        if !is_absolute_desktop_path(path) {
            return None;
        }
        let app = gio::DesktopAppInfo::from_filename(path);
        for b in &self.buttons {
            if b.pinned && b.desktop.as_deref() == Some(path) {
                return Some(b.id);
            }
        }
        let id = self.create_button();
        if let Some(b) = self.button_mut(id) {
            b.pinned = true;
            b.app = app;
            b.desktop = Some(path.to_string());
            b.key = format!("pin:{}", path);
        }
        self.app_generation += 1;
        Some(id)
    }

    /// Bind a live window to a button, reusing a matching idle pin.
    pub fn add_window(&mut self, window: &glib::Object) -> Option<u64> {
        let app = self.match_app(window);
        let mut target: Option<u64> = None;
        for b in &self.buttons {
            if b.pinned
                && b.window.is_none()
                && crate::apps::app_equal(b.app.as_ref(), app.as_ref())
            {
                target = Some(b.id);
                break;
            }
        }
        let id = match target {
            Some(id) => id,
            None => {
                let id = self.create_button();
                if let Some(b) = self.button_mut(id) {
                    b.app = app;
                    b.key = format!("window:{}", ffi_xfce::Window::new(window).xid());
                }
                id
            }
        };
        self.attach_window(id, window);
        // A launching pin that created this window finishes its animation.
        let launcher_app = self.button(id).and_then(|b| b.app.clone());
        let launchers: Vec<u64> = self
            .buttons
            .iter()
            .filter(|q| {
                q.launching && crate::apps::app_equal(q.app.as_ref(), launcher_app.as_ref())
            })
            .map(|q| q.id)
            .collect();
        for q in launchers {
            self.launch_complete(q);
        }
        Some(id)
    }

    pub fn attach_window(&mut self, button_id: u64, window: &glib::Object) {
        let Some(index) = self.button_index(button_id) else {
            return;
        };
        {
            let b = &mut self.buttons[index];
            b.window = Some(window.clone());
            b.pid = crate::apps::window_pid(window);
            b.icon_dirty = true;
            b.match_dirty = false;
            b.match_generation = self.app_generation;
        }
        self.window_index
            .insert(window.as_ptr() as usize, button_id);

        let weak = self.weak();
        let mut handlers = Vec::new();
        for signal in [
            "state-changed",
            "name-changed",
            "workspace-changed",
            "capabilities-changed",
        ] {
            let weak = weak.clone();
            handlers.push(window.connect_local(signal, false, move |_| {
                if let Some(rc) = weak.upgrade() {
                    crate::util::with_dock(&rc, |d| d.queue_refresh());
                }
                None
            }));
        }
        {
            let weak = weak.clone();
            handlers.push(window.connect_local("class-changed", false, move |_| {
                if let Some(rc) = weak.upgrade() {
                    crate::util::with_dock(&rc, |d| d.mark_class_changed(button_id));
                }
                None
            }));
        }
        {
            let weak = weak.clone();
            handlers.push(window.connect_local("icon-changed", false, move |_| {
                if let Some(rc) = weak.upgrade() {
                    crate::util::with_dock(&rc, |d| d.mark_icon_changed(button_id));
                }
                None
            }));
        }
        self.buttons[index].window_handlers = handlers;

        // Watch the identity properties (ports the GdkWindow X event filter).
        let xid = ffi_xfce::Window::new(window).xid();
        let watch = ffi_x11::watch_window_properties(xid).map(|xwindow| {
            let weak = self.weak();
            let atoms = self.identity_atoms;
            let watch = ffi_x11::PropertyWatch::new(&xwindow, &atoms, move || {
                if let Some(rc) = weak.upgrade() {
                    crate::util::with_dock(&rc, |d| d.mark_class_changed(button_id));
                }
            });
            (xwindow, watch)
        });
        if let Some((xwindow, watch)) = watch {
            self.buttons[index].xwindow = Some(xwindow);
            self.buttons[index].property_watch = Some(watch);
        }
    }

    pub fn release_window(&mut self, button_id: u64) {
        let Some(index) = self.button_index(button_id) else {
            return;
        };
        if self.hover_button == Some(button_id) {
            self.hide_preview();
        }
        if self.drag_button == Some(button_id) {
            self.drag_button = None;
            self.dragging = false;
        }
        if self.insert_button == Some(button_id) {
            self.insert_button = None;
        }
        if let Some(menu) = self.menu.take() {
            ffi_gtk::destroy(&menu);
        }
        let button = &mut self.buttons[index];
        // Dropping the watch removes the GDK filter, so the callback can no
        // longer run for this button; the foreign window goes with it.
        button.property_watch = None;
        button.xwindow = None;
        if let Some(window) = button.window.take() {
            self.window_index.remove(&(window.as_ptr() as usize));
            for handler in button.window_handlers.drain(..) {
                window.disconnect(handler);
            }
        }
        let button = &mut self.buttons[index];
        button.thumbnail = None;
        button.audio = crate::audio::AudioStatus::default();
        button.number = 0;
        button.audio_scroll = 0.0;
        button.window_scroll = 0.0;
        button.last_audio_time = 0;
        button.pid = 0;
        button.icon_dirty = true;
    }

    pub fn free_button(&mut self, button_id: u64) {
        self.release_window(button_id);
        if let Some(mut button) = self.remove_button(button_id) {
            button.destroy_widgets();
        }
    }

    fn remove_button(&mut self, button_id: u64) -> Option<Button> {
        let index = self.button_index(button_id)?;
        Some(self.buttons.remove(index))
    }

    pub fn mark_class_changed(&mut self, button_id: u64) {
        if let Some(b) = self.button_mut(button_id) {
            b.match_dirty = true;
            b.pid = 0;
            b.thumbnail_time = 0;
        }
        self.queue_refresh();
    }

    pub fn mark_icon_changed(&mut self, button_id: u64) {
        if let Some(b) = self.button_mut(button_id) {
            b.icon_dirty = true;
        }
        self.queue_refresh();
    }

    // -- window actions ----------------------------------------------------

    pub fn activate(&mut self, button_id: u64) {
        let Some(window) = self.button(button_id).and_then(|b| b.window.clone()) else {
            return;
        };
        let w = ffi_xfce::Window::new(&window);
        if let Some(ws) = w.workspace() {
            if ffi_xfce::Workspace::state(&ws) & ffi_xfce::XFW_WORKSPACE_STATE_ACTIVE == 0 {
                ffi_xfce::Workspace::activate(&ws);
            }
        }
        if w.is_minimized() {
            w.set_minimized(false);
        }
        w.activate(self.timestamp());
    }

    pub fn minimize(&mut self, button_id: u64) {
        let Some(window) = self.button(button_id).and_then(|b| b.window.clone()) else {
            return;
        };
        // Keep the last frame before minimizing.
        self.capture(button_id);
        ffi_xfce::Window::new(&window).set_minimized(true);
    }

    pub fn toggle(&mut self, button_id: u64) {
        let (has_window, pinned, left_action) = match self.button(button_id) {
            Some(b) => (b.window.is_some(), b.pinned, self.settings.left_action),
            None => return,
        };
        if !has_window && pinned {
            if !self.button(button_id).is_some_and(|b| b.launching) {
                self.launch(button_id);
            }
            return;
        }
        let is_active = self
            .button(button_id)
            .and_then(|b| b.window.as_ref())
            .is_some_and(|w| ffi_xfce::Window::new(w).is_active());
        if left_action == 0 && is_active {
            self.minimize(button_id);
        } else {
            self.activate(button_id);
        }
    }

    pub fn close_window(&mut self, button_id: u64) {
        let Some(window) = self.button(button_id).and_then(|b| b.window.clone()) else {
            return;
        };
        let timestamp = self.timestamp();
        self.hide_preview();
        ffi_xfce::Window::new(&window).close(timestamp);
    }

    pub fn cycle(&mut self, delta: i32) {
        let mut candidates: Vec<u64> = Vec::new();
        let mut active_index: i32 = -1;
        for b in &self.buttons {
            let Some(window) = b.window.as_ref() else {
                continue;
            };
            if !b.eligible {
                continue;
            }
            if !self.settings.all_workspaces && !self.window_in_workspace(window) {
                continue;
            }
            if ffi_xfce::Window::new(window).is_active() {
                active_index = candidates.len() as i32;
            }
            candidates.push(b.id);
        }
        if candidates.is_empty() {
            return;
        }
        let len = candidates.len() as i32;
        let index = if active_index < 0 {
            if delta > 0 {
                0
            } else {
                len - 1
            }
        } else {
            (active_index + delta + len) % len
        };
        let id = candidates[index as usize];
        self.activate(id);
    }

    pub fn window_in_workspace(&self, window: &glib::Object) -> bool {
        let w = ffi_xfce::Window::new(window);
        if w.is_pinned() {
            return true;
        }
        match w.workspace() {
            None => true,
            Some(ws) => ffi_xfce::Workspace::state(&ws) & ffi_xfce::XFW_WORKSPACE_STATE_ACTIVE != 0,
        }
    }

    // -- pin management -----------------------------------------------------

    pub fn pin_app(&mut self, app: &gio::DesktopAppInfo) {
        let Some(path) = app.filename().map(|p| p.to_string_lossy().into_owned()) else {
            return;
        };
        for b in &self.buttons {
            if b.pinned && crate::apps::app_equal(b.app.as_ref(), Some(app)) {
                return;
            }
        }
        // Pin a running button in place, preserving position and preview.
        for b in &self.buttons {
            let Some(window) = b.window.as_ref() else {
                continue;
            };
            if b.pinned {
                continue;
            }
            let ids = ffi_xfce::Window::new(window).class_ids();
            let score = crate::apps::app_match_score(app, &ids);
            let own_score = b
                .app
                .as_ref()
                .map(|a| crate::apps::app_match_score(a, &ids))
                .unwrap_or(0);
            if crate::apps::app_equal(b.app.as_ref(), Some(app))
                || (score > 0 && (b.app.is_none() || score >= own_score))
            {
                let id = b.id;
                if let Some(b) = self.button_mut(id) {
                    b.pinned = true;
                    b.app = Some(app.clone());
                    b.desktop = Some(path.clone());
                    b.key = format!("pin:{}", path);
                }
                self.app_generation += 1;
                if let Some(b) = self.button_mut(id) {
                    b.icon_dirty = true;
                }
                self.save();
                self.refresh();
                return;
            }
        }
        if self.add_pin(&path).is_some() {
            self.save();
            self.refresh();
        }
    }

    pub fn unpin(&mut self, button_id: u64) {
        let pinned = self.button(button_id).is_some_and(|b| b.pinned);
        if !pinned {
            return;
        }
        self.app_generation += 1;
        let has_window = self.button(button_id).is_some_and(|b| b.window.is_some());
        if has_window {
            {
                let b = self.button_mut(button_id).unwrap();
                b.pinned = false;
                b.desktop = None;
                b.key = format!("window:{}", b.window_xid());
            }
            if let Some(menu) = self.menu.take() {
                ffi_gtk::destroy(&menu);
            }
        } else {
            self.free_button(button_id);
        }
        self.save();
        self.update_buttons();
    }

    pub fn move_button(&mut self, source: u64, target: u64, after: bool) {
        let (Some(si), Some(ti)) = (self.button_index(source), self.button_index(target)) else {
            return;
        };
        if si == ti {
            return;
        }
        let button = self.buttons.remove(si);
        let mut ti = self.button_index(target).unwrap_or(0);
        if after {
            ti += 1;
        }
        self.buttons.insert(ti, button);
        for (j, b) in self.buttons.iter().enumerate() {
            self.box_.reorder_child(&b.widget, j as i32);
        }
        self.save();
        self.update_buttons();
    }

    // -- audio scroll entry point (shared with preview popup) ---------------

    pub fn audio_scroll(&mut self, button_id: u64, event: &ScrollInfo) {
        if event.time != 0
            && self
                .button(button_id)
                .is_some_and(|b| b.last_audio_time == event.time)
        {
            return;
        }
        if let Some(b) = self.button_mut(button_id) {
            b.last_audio_time = event.time;
        }
        let steps = {
            let accumulator = self.button_mut(button_id).map(|b| &mut b.audio_scroll);
            scroll_steps(event, accumulator)
        };
        if steps != 0 {
            self.audio_volume(button_id, steps);
            self.volume_bubble(button_id);
        }
    }
}

// ---------------------------------------------------------------------------
// Scroll accumulation
// ---------------------------------------------------------------------------

/// Port of `scroll_steps`: direction events snap to ±1, smooth events
/// accumulate into `accumulator`.
pub fn scroll_steps(event: &ScrollInfo, accumulator: Option<&mut f64>) -> i32 {
    use gdk::ScrollDirection as D;
    match event.direction {
        D::Up | D::Left => {
            if let Some(acc) = accumulator {
                *acc = 0.0;
            }
            1
        }
        D::Down | D::Right => {
            if let Some(acc) = accumulator {
                *acc = 0.0;
            }
            -1
        }
        _ => {
            let Some(accumulator) = accumulator else {
                return 0;
            };
            let accumulator = &mut *accumulator;
            // gdk_event_get_scroll_deltas only supplies deltas for smooth
            // events; non-smooth events without a handled direction are ignored.
            let (dx, dy) = if event.direction == D::Smooth {
                (event.delta_x, event.delta_y)
            } else {
                return 0;
            };
            *accumulator -= if dy.abs() >= dx.abs() { dy } else { dx };
            let steps =
                ((accumulator.abs() + 1e-6).floor().copysign(*accumulator) as i32).clamp(-32, 32);
            *accumulator -= steps as f64;
            steps
        }
    }
}

// ---------------------------------------------------------------------------
// Widget wiring
// ---------------------------------------------------------------------------

fn wire_button(b: &mut Button, weak: DockWeak) {
    let button_id = b.id;

    // draw
    {
        let weak = weak.clone();
        b.drawing.connect_draw(move |_w, cr| {
            let Some(rc) = weak.upgrade() else {
                return glib::Propagation::Proceed;
            };
            crate::util::with_dock(&rc, |d| d.draw_button(button_id, _w.upcast_ref(), cr));
            glib::Propagation::Proceed
        });
    }

    // button-press-event
    {
        let weak = weak.clone();
        b.main.connect_button_press_event(move |_w, e| {
            let Some(rc) = weak.upgrade() else {
                return glib::Propagation::Proceed;
            };
            let mut inhibit = glib::Propagation::Proceed;
            crate::util::with_dock(&rc, |d| {
                if e.button() == 3 {
                    d.hide_preview();
                    if d.button(button_id).is_some_and(|b| b.window.is_some()) {
                        d.window_menu(button_id, Some(&**e));
                    } else {
                        d.pin_menu(button_id, Some(&**e));
                    }
                    inhibit = glib::Propagation::Stop;
                } else if e.button() == 2 {
                    match d.settings.middle_action {
                        0 => d.launch(button_id),
                        1 => d.close_window(button_id),
                        _ => {}
                    }
                    inhibit = glib::Propagation::Stop;
                }
            });
            inhibit
        });
    }

    // clicked
    {
        let weak = weak.clone();
        b.main.connect_clicked(move |_| {
            if let Some(rc) = weak.upgrade() {
                crate::util::with_dock(&rc, |d| {
                    if !d.dragging {
                        d.toggle(button_id);
                    }
                });
            }
        });
    }

    // key-press-event
    {
        let weak = weak.clone();
        b.main.connect_key_press_event(move |w, e| {
            let Some(rc) = weak.upgrade() else {
                return glib::Propagation::Proceed;
            };
            let mut handled = glib::Propagation::Proceed;
            crate::util::with_dock(&rc, |d| {
                let menu_key = e.keyval() == Key::Menu
                    || (e.keyval() == Key::F10
                        && e.state().contains(gdk::ModifierType::SHIFT_MASK));
                if menu_key {
                    if d.button(button_id).is_some_and(|b| b.window.is_some()) {
                        d.window_menu(button_id, Some(&**e));
                    } else {
                        d.pin_menu(button_id, Some(&**e));
                    }
                    handled = glib::Propagation::Stop;
                    return;
                }
                if (e.keyval() == Key::Return || e.keyval() == Key::KP_Enter)
                    && e.state().contains(gdk::ModifierType::CONTROL_MASK)
                {
                    d.launch(button_id);
                    handled = glib::Propagation::Stop;
                    return;
                }
                if d.focus_key(w.upcast_ref(), e) {
                    handled = glib::Propagation::Stop;
                }
            });
            handled
        });
    }

    // scroll on main + widget
    for widget in [
        b.main.clone().upcast::<gtk::Widget>(),
        b.widget.clone().upcast::<gtk::Widget>(),
    ] {
        let weak = weak.clone();
        widget.connect_scroll_event(move |w, e| {
            let Some(rc) = weak.upgrade() else {
                return glib::Propagation::Proceed;
            };
            let mut handled = glib::Propagation::Proceed;
            crate::util::with_dock(&rc, |d| {
                let info = ScrollInfo::from_event(e);
                if d.over_sound(button_id, w, &info) {
                    d.audio_scroll(button_id, &info);
                    handled = glib::Propagation::Stop;
                    return;
                }
                if !d.settings.scroll_windows {
                    return;
                }
                let acc = d.button_mut(button_id).map(|b| &mut b.window_scroll);
                let steps = crate::button::scroll_steps(&info, acc);
                if steps != 0 {
                    d.cycle(-steps);
                }
                handled = glib::Propagation::Stop;
            });
            handled
        });
    }

    // enter/leave
    {
        let weak = weak.clone();
        b.widget.connect_enter_notify_event(move |_w, e| {
            if let Some(rc) = weak.upgrade() {
                crate::util::with_dock(&rc, |d| {
                    if e.detail() != gdk::NotifyType::Inferior {
                        d.schedule_preview(button_id);
                    }
                });
            }
            glib::Propagation::Proceed
        });
    }
    {
        let weak = weak.clone();
        b.main.connect_enter_notify_event(move |_w, e| {
            if let Some(rc) = weak.upgrade() {
                crate::util::with_dock(&rc, |d| {
                    if e.detail() != gdk::NotifyType::Inferior {
                        d.schedule_preview(button_id);
                    }
                });
            }
            glib::Propagation::Proceed
        });
    }
    {
        let weak = weak.clone();
        b.main.connect_leave_notify_event(move |_w, e| {
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

    // sound widget
    {
        b.sound
            .connect_button_press_event(|_w, e| glib::Propagation::from(e.button() != 1));
    }
    {
        let weak = weak.clone();
        b.sound.connect_clicked(move |_| {
            if let Some(rc) = weak.upgrade() {
                crate::util::with_dock(&rc, |d| {
                    d.audio_mute(button_id);
                    d.volume_bubble(button_id);
                });
            }
        });
    }
    {
        let weak = weak.clone();
        b.sound.connect_scroll_event(move |_w, e| {
            if let Some(rc) = weak.upgrade() {
                crate::util::with_dock(&rc, |d| {
                    let info = ScrollInfo::from_event(e);
                    d.audio_scroll(button_id, &info);
                });
            }
            glib::Propagation::Stop
        });
    }

    // drag and drop
    let targets_button = [gtk::TargetEntry::new(
        "application/x-zero-dock-button",
        gtk::TargetFlags::empty(),
        1,
    )];
    let targets_all = [
        gtk::TargetEntry::new(
            "application/x-zero-dock-button",
            gtk::TargetFlags::empty(),
            1,
        ),
        gtk::TargetEntry::new("text/uri-list", gtk::TargetFlags::empty(), 2),
    ];
    b.main.drag_source_set(
        gdk::ModifierType::BUTTON1_MASK,
        &targets_button,
        gdk::DragAction::MOVE,
    );
    b.main.drag_dest_set(
        gtk::DestDefaults::ALL,
        &targets_all,
        gdk::DragAction::COPY | gdk::DragAction::MOVE,
    );
    {
        let weak = weak.clone();
        b.main.connect_drag_begin(move |_w, _ctx| {
            if let Some(rc) = weak.upgrade() {
                crate::util::with_dock(&rc, |d| {
                    d.dragging = true;
                    d.drag_button = Some(button_id);
                    d.hide_preview();
                });
            }
        });
    }
    {
        let weak = weak.clone();
        b.main.connect_drag_end(move |_w, _ctx| {
            if let Some(rc) = weak.upgrade() {
                crate::util::with_dock(&rc, |d| {
                    d.dragging = false;
                    d.drag_button = None;
                    d.insert_button = None;
                    d.update_buttons();
                });
            }
        });
    }
    {
        let weak = weak.clone();
        b.main
            .connect_drag_data_get(move |_w, _ctx, data, info, _time| {
                if let Some(rc) = weak.upgrade() {
                    crate::util::with_dock(&rc, |d| {
                        if info == 1 {
                            if let Some(b) = d.button(button_id) {
                                data.set(&data.target(), 8, b.key.as_bytes());
                            }
                        }
                    });
                }
            });
    }
    {
        let weak = weak.clone();
        b.main.connect_drag_motion(move |w, ctx, x, y, time| {
            let Some(rc) = weak.upgrade() else {
                return false;
            };
            let mut result = false;
            crate::util::with_dock(&rc, |d| {
                let a = w.allocation();
                d.insert_button = Some(button_id);
                d.insert_after = if d.orientation == gtk::Orientation::Horizontal {
                    x > a.width() / 2
                } else {
                    y > a.height() / 2
                };
                d.update_buttons();
                ctx.drag_status(
                    if d.drag_button.is_some() {
                        gdk::DragAction::MOVE
                    } else {
                        gdk::DragAction::COPY
                    },
                    time,
                );
                result = true;
            });
            result
        });
    }
    {
        let weak = weak.clone();
        b.main.connect_drag_leave(move |_w, _ctx, _time| {
            if let Some(rc) = weak.upgrade() {
                crate::util::with_dock(&rc, |d| {
                    d.insert_button = None;
                    d.update_buttons();
                });
            }
        });
    }
    {
        let weak = weak.clone();
        b.main
            .connect_drag_data_received(move |_w, ctx, _x, _y, data, info, time| {
                let Some(rc) = weak.upgrade() else { return };
                crate::util::with_dock(&rc, |d| {
                    let mut ok = false;
                    if info == 1 && data.length() > 0 {
                        if let Ok(key) = std::str::from_utf8(&data.data()).map(|s| s.to_string()) {
                            let src = d
                                .buttons
                                .iter()
                                .find(|src| src.key == key)
                                .map(|src| src.id);
                            if let Some(src) = src {
                                d.move_button(src, button_id, d.insert_after);
                                ok = true;
                            }
                        }
                    } else if info == 2 {
                        ok = import_uris(d, data);
                    }
                    d.insert_button = None;
                    ctx.drag_finish(ok, false, time);
                    d.update_buttons();
                });
            });
    }
}

/// Handle a drop of .desktop files on any dock surface.
pub fn import_uris(d: &mut Dock, data: &gtk::SelectionData) -> bool {
    let mut ok = false;
    for uri in data.uris() {
        let Ok(path) = glib::filename_from_uri(&uri) else {
            continue;
        };
        let path = path.0.to_string_lossy().into_owned();
        if path.ends_with(".desktop") {
            if let Some(app) = gio::DesktopAppInfo::from_filename(&path) {
                d.pin_app(&app);
                ok = true;
            }
        }
    }
    ok
}

// ---------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------

impl Dock {
    /// Port of the button `draw` handler.
    pub fn draw_button(&mut self, button_id: u64, widget: &gtk::Widget, cr: &cairo::Context) {
        let Some(index) = self.button_index(button_id) else {
            return;
        };
        let a = widget.allocation();
        let width = a.width() as f64;
        let height = a.height() as f64;
        let button = &self.buttons[index];
        let (active, min, urgent) = button
            .window
            .as_ref()
            .map(|w| {
                let w = ffi_xfce::Window::new(w);
                (w.is_active(), w.is_minimized(), w.is_urgent())
            })
            .unwrap_or((false, false, false));
        let mut accent = gdk::RGBA::new(0.69, 0.43, 1.0, 1.0);
        if let Some(color) = widget
            .style_context()
            .lookup_color("theme_selected_bg_color")
        {
            accent = color;
        }

        if active {
            cr.set_source_rgba(accent.red(), accent.green(), accent.blue(), 0.20);
            cr.rectangle(2.0, 2.0, width - 4.0, height - 4.0);
            let _ = cr.fill();
        }
        if let Some(icon) = button.icon.clone() {
            let scale = 1.0f64.max(button.icon_scale as f64);
            let x = (width - icon.width() as f64 / scale) / 2.0;
            let y = (height - icon.height() as f64 / scale) / 2.0;
            if cr.save().is_ok() {
                cr.translate(x, y);
                cr.scale(1.0 / scale, 1.0 / scale);
                cr.set_source_pixbuf(&icon, 0.0, 0.0);
                let _ = cr.paint_with_alpha(if min { 0.45 } else { 1.0 });
                let _ = cr.restore();
            }
        }
        if button.launching {
            let angle =
                (monotonic_us() % 1_000_000) as f64 / 1_000_000.0 * 2.0 * std::f64::consts::PI;
            cr.set_source_rgba(accent.red(), accent.green(), accent.blue(), 0.9);
            cr.set_line_width(2.0);
            cr.arc(
                width / 2.0,
                height / 2.0,
                width.min(height) / 2.0 - 3.0,
                angle,
                angle + std::f64::consts::PI * 1.3,
            );
            let _ = cr.stroke();
        } else if button.launch_error.is_some() || (button.pinned && button.app.is_none()) {
            cr.set_source_rgb(1.0, 0.65, 0.2);
            cr.arc(
                width - 8.0,
                height - 8.0,
                4.0,
                0.0,
                2.0 * std::f64::consts::PI,
            );
            let _ = cr.fill();
        }
        if active {
            cr.set_source_rgba(accent.red(), accent.green(), accent.blue(), accent.alpha());
            if self.orientation == gtk::Orientation::Horizontal {
                cr.rectangle(width * 0.25, height - 3.0, width * 0.5, 3.0);
            } else {
                cr.rectangle(width - 3.0, height * 0.25, 3.0, height * 0.5);
            }
            let _ = cr.fill();
        }
        if min {
            cr.set_source_rgb(0.7, 0.7, 0.75);
            cr.arc(
                width / 2.0,
                height - 4.0,
                2.0,
                0.0,
                2.0 * std::f64::consts::PI,
            );
            let _ = cr.fill();
        }
        if urgent {
            cr.set_source_rgb(1.0, 0.75, 0.18);
            cr.set_line_width(2.0);
            cr.rectangle(2.0, 2.0, width - 4.0, height - 4.0);
            let _ = cr.stroke();
        }
        if self.settings.numbers && button.number > 0 {
            cr.set_source_rgba(0.15, 0.15, 0.2, 0.95);
            cr.arc(9.0, 9.0, 8.0, 0.0, 2.0 * std::f64::consts::PI);
            let _ = cr.fill();
            let text = format!("{}", button.number);
            let layout = widget.create_pango_layout(Some(&text));
            let font = pango::FontDescription::from_string("Sans Bold 8");
            layout.set_font_description(Some(&font));
            let (tw, th) = layout.pixel_size();
            cr.set_source_rgb(1.0, 1.0, 1.0);
            cr.move_to(9.0 - tw as f64 / 2.0, 9.0 - th as f64 / 2.0);
            ffi_gtk::show_layout(cr, &layout);
        }
        if self.insert_button == Some(button_id) {
            cr.set_source_rgb(0.25, 0.65, 1.0);
            if self.orientation == gtk::Orientation::Horizontal {
                cr.rectangle(
                    if self.insert_after { width - 3.0 } else { 0.0 },
                    0.0,
                    3.0,
                    height,
                );
            } else {
                cr.rectangle(
                    0.0,
                    if self.insert_after { height - 3.0 } else { 0.0 },
                    width,
                    3.0,
                );
            }
            let _ = cr.fill();
        }
    }

    /// Port of `over_sound`: does this scroll event land on the button's
    /// speaker control?
    pub fn over_sound(&self, button_id: u64, w: &gtk::Widget, info: &ScrollInfo) -> bool {
        let Some(b) = self.button(button_id) else {
            return false;
        };
        if !b.sound.is_visible() {
            return false;
        }
        let a = b.sound.allocation();
        let top = b.sound.toplevel();
        if let Some(top) = top.as_ref() {
            if top.is_realized() {
                if let (Some((x, y)), Some(top_window)) =
                    (b.sound.translate_coordinates(top, 0, 0), top.window())
                {
                    // gdk_window_get_origin binds as (screen-number, x, y).
                    let (_, ox, oy) = top_window.origin();
                    if info.x_root != 0.0 || info.y_root != 0.0 {
                        return info.x_root >= (ox + x) as f64
                            && info.x_root < (ox + x + a.width()) as f64
                            && info.y_root >= (oy + y) as f64
                            && info.y_root < (oy + y + a.height()) as f64;
                    }
                }
            }
        }
        let Some((x, y)) = b.sound.translate_coordinates(w, 0, 0) else {
            return false;
        };
        info.x >= x as f64
            && info.x < (x + a.width()) as f64
            && info.y >= y as f64
            && info.y < (y + a.height()) as f64
    }

    // -- refresh cycle -------------------------------------------------------

    /// Port of `zd_update_buttons`: icons, numbering, tooltips, geometry.
    pub fn update_buttons(&mut self) {
        self.layout_update();
        let scale = self.box_.scale_factor().max(1) as u32;
        let pixels = self.icon_size * scale;
        let ids: Vec<u64> = self.buttons.iter().map(|b| b.id).collect();
        for id in ids {
            let Some(index) = self.button_index(id) else {
                continue;
            };
            if self.buttons[index].icon_dirty
                || self.buttons[index].icon_pixels != pixels
                || self.buttons[index].icon_scale != scale
            {
                {
                    let b = &mut self.buttons[index];
                    b.icon = None;
                    b.icon_dirty = false;
                    b.icon_pixels = pixels;
                    b.icon_scale = scale;
                    if let Some(window) = b.window.clone() {
                        if let Some(p) =
                            ffi_xfce::Window::new(&window).icon(self.icon_size as i32, scale as i32)
                        {
                            b.icon = p.scale_simple(
                                pixels as i32,
                                pixels as i32,
                                gdk_pixbuf::InterpType::Bilinear,
                            );
                        }
                    }
                    if b.icon.is_none() {
                        if let Some(app) = b.app.clone() {
                            let icon = app.upcast_ref::<gio::AppInfo>().icon();
                            if let Some(icon) = icon {
                                b.icon = self
                                    .icon_theme
                                    .lookup_by_gicon_for_scale(
                                        &icon,
                                        self.icon_size as i32,
                                        scale as i32,
                                        gtk::IconLookupFlags::FORCE_SIZE,
                                    )
                                    .and_then(|info| info.load_icon().ok());
                            }
                        }
                    }
                    if b.icon.is_none() {
                        let name = if b.pinned && b.app.is_none() {
                            "dialog-warning"
                        } else {
                            "application-x-executable"
                        };
                        b.icon = self
                            .icon_theme
                            .load_icon_for_scale(
                                name,
                                self.icon_size as i32,
                                scale as i32,
                                gtk::IconLookupFlags::FORCE_SIZE,
                            )
                            .ok()
                            .flatten();
                    }
                }
            }
            // Window numbering among same-app groups.
            {
                let b = &mut self.buttons[index];
                b.number = 0;
            }
            if self.buttons[index].window.is_some() {
                let mut n = 0u32;
                let mut order = 0u32;
                for q in &self.buttons {
                    if q.window.is_none() {
                        continue;
                    }
                    if !same_app(self, index, q.id) {
                        continue;
                    }
                    if !(self.settings.all_workspaces
                        || q.pinned
                        || q.window
                            .as_ref()
                            .is_some_and(|w| self.window_in_workspace(w)))
                    {
                        continue;
                    }
                    n += 1;
                    if q.id == id {
                        order = n;
                    }
                }
                if n > 1 {
                    self.buttons[index].number = order;
                }
            }
            {
                let b = &mut self.buttons[index];
                b.main.set_size_request(self.unit as i32, self.unit as i32);
                b.drawing
                    .set_size_request(self.unit as i32, self.unit as i32);
                let title = b.display_name();
                let tooltip = if b.launching {
                    Some(t("正在启动…").to_string())
                } else if let Some(error) = b.launch_error.clone() {
                    Some(error)
                } else if self.settings.previews && b.window.is_some() {
                    None
                } else {
                    Some(title.clone())
                };
                b.main.set_tooltip_text(tooltip.as_deref());
                if let Some(accessible) = b.main.accessible() {
                    accessible.set_name(&title);
                }
                b.drawing.queue_draw();
                if let (Some(window), Some(window_gdk)) = (b.window.clone(), b.main.window()) {
                    if b.main.is_realized() {
                        let a = b.main.allocation();
                        let rect = gdk::Rectangle::new(a.x(), a.y(), a.width(), a.height());
                        ffi_xfce::Window::new(&window).set_button_geometry(&window_gdk, &rect);
                    }
                }
            }
        }
        self.update_audio_buttons();
    }

    /// Port of `zd_update_audio_buttons`.
    pub fn update_audio_buttons(&mut self) {
        if self.disposing {
            return;
        }
        let ids: Vec<u64> = self.buttons.iter().map(|b| b.id).collect();
        for id in ids {
            let Some(index) = self.button_index(id) else {
                continue;
            };
            let old = self.buttons[index].audio;
            let status = self.audio_status(id);
            self.buttons[index].audio = status;
            let visible = status.present && (status.playing || status.muted);
            let b = &self.buttons[index];
            b.sound.set_visible(visible);
            let has_image = b.sound.image().is_some();
            if !has_image || old.muted != status.muted {
                let image = gtk::Image::from_icon_name(
                    Some(if status.muted {
                        "audio-volume-muted-symbolic"
                    } else {
                        "audio-volume-high-symbolic"
                    }),
                    gtk::IconSize::Menu,
                );
                b.sound.set_image(Some(&image));
                image.show();
                b.sound.set_tooltip_text(Some(
                    if status.muted {
                        t("取消应用静音 · 滚轮调音量")
                    } else {
                        t("应用静音 · 滚轮调音量")
                    }
                    .as_str(),
                ));
            }
        }
        self.preview_update_audio();
    }
}

fn same_app(dock: &Dock, index: usize, other: u64) -> bool {
    let Some(other_index) = dock.button_index(other) else {
        return false;
    };
    let a = &dock.buttons[index];
    let b = &dock.buttons[other_index];
    if let (Some(aa), Some(bb)) = (a.app.as_ref(), b.app.as_ref()) {
        return crate::apps::app_equal(Some(aa), Some(bb));
    }
    let (Some(wa), Some(wb)) = (a.window.as_ref(), b.window.as_ref()) else {
        return false;
    };
    let aa = ffi_xfce::Window::new(wa).class_ids();
    let bb = ffi_xfce::Window::new(wb).class_ids();
    match (aa.first(), bb.first()) {
        (Some(x), Some(y)) => x.eq_ignore_ascii_case(y),
        _ => false,
    }
}
