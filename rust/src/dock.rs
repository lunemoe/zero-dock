//! Instance lifecycle, pin persistence, window reconciliation and panel
//! geometry. Ported from `src/plugin.c` plus `entry.c` handoff.
//!
//! Ownership model:
//! * the plugin object owns the single strong `Rc<RefCell<Dock>>` via
//!   `g_object_set_data_full`; `free-data` runs the orderly dispose and the
//!   data destroy-notify drops the `Rc`;
//! * every signal handler (GTK, Xfw, GLib sources) captures only a `Weak`,
//!   so no callback can keep the dock alive or touch freed state;
//! * button state is addressed by stable ids, and menus resolve keys against
//!   the live list — no raw pointers cross callback boundaries.

use crate::button::Button;
use crate::ffi_x11::Atom;
use crate::ffi_xfce::{Screen, XfcePanelPlugin};
use crate::input::Input;
use crate::settings::Settings;
use crate::util::{t, DockWeak, Timer};
use gio::prelude::*;
use gtk::prelude::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

pub struct Dock {
    pub plugin: Option<XfcePanelPlugin>,
    pub screen: Option<Screen>,
    pub weak: DockWeak,

    // Widgets
    pub container: gtk::Box,
    pub box_: gtk::Box,
    pub overflow: gtk::Button,
    pub association_dialog: Option<gtk::FileChooserDialog>,
    pub settings_dialog: Option<gtk::Dialog>,
    pub menu: Option<gtk::Menu>,
    pub show_desktop_item: Option<gtk::CheckMenuItem>,
    pub minimize_all_item: Option<gtk::MenuItem>,
    pub error_dialog: Option<gtk::MessageDialog>,
    pub about_dialog: Option<gtk::AboutDialog>,

    // Model
    pub buttons: Vec<Button>,
    pub apps: Vec<gio::AppInfo>,
    pub associations: HashMap<String, String>,
    pub app_monitor: Option<gio::AppInfoMonitor>,
    pub icon_theme: gtk::IconTheme,
    pub css: gtk::CssProvider,

    // Signal connections on process-global / longer-lived objects. These must
    // be disconnected explicitly when an instance is disposed; otherwise each
    // plugin reload leaves another dead Weak callback attached to the object.
    pub screen_handlers: Vec<glib::SignalHandlerId>,
    pub workspace_handlers: Vec<(glib::Object, glib::SignalHandlerId)>,
    pub app_monitor_handler: Option<glib::SignalHandlerId>,
    pub theme_handler: Option<glib::SignalHandlerId>,
    pub audio: Option<crate::audio::AudioRef>,
    pub input: Option<Input>,

    // Geometry / settings
    pub orientation: gtk::Orientation,
    pub unit: u32,
    pub icon_size: u32,
    pub settings: Settings,
    pub app_generation: u32,
    pub rc_path: Option<String>,

    // Flags
    pub disposing: bool,
    pub dragging: bool,
    pub save_blocked: bool,
    pub autohide_blocked: bool,

    // Timers
    pub hover_id: Timer,
    pub refresh_id: Timer,
    pub active_frame_id: Timer,
    pub launch_tick: Timer,

    // Drag / hover state
    pub identity_atoms: [Atom; 4],
    pub drag_button: Option<u64>,
    pub insert_button: Option<u64>,
    pub insert_after: bool,
    pub hover_button: Option<u64>,

    // Preview popup
    pub preview: Option<gtk::Window>,
    pub preview_image: Option<gtk::Image>,
    pub preview_title: Option<gtk::Label>,
    pub preview_sound: Option<gtk::Button>,
    pub preview_status: Option<gtk::Label>,
    pub preview_workspace: Option<gtk::Label>,
    pub preview_close: Option<gtk::Button>,
    pub preview_tick: Timer,
    pub leave_id: Timer,

    // Volume bubble
    pub bubble: Option<gtk::Window>,
    pub bubble_text: Option<gtk::Label>,
    pub bubble_bar: Option<gtk::ProgressBar>,
    pub bubble_id: Timer,

    // Internals
    pub next_button_id: u64,
    pub window_index: HashMap<usize, u64>,
}

impl Drop for Dock {
    fn drop(&mut self) {
        if !self.disposing {
            self.dispose();
        }
    }
}

impl Dock {
    pub fn weak(&self) -> DockWeak {
        self.weak.clone()
    }

    pub fn rc_path(&self) -> String {
        self.rc_path.clone().unwrap_or_default()
    }

    pub fn timestamp(&self) -> u32 {
        let t = gtk::current_event_time();
        if t != 0 {
            t
        } else {
            crate::ffi_x11::server_time()
        }
    }

    pub fn save_pins_to_keyfile(&self, file: &glib::KeyFile) {
        let pins: Vec<String> = self
            .buttons
            .iter()
            .filter(|b| b.pinned)
            .filter_map(|b| b.desktop.clone())
            .collect();
        crate::settings::write_pins_to_keyfile(file, &pins);
    }

    pub fn save(&mut self) -> bool {
        if self.disposing || self.rc_path.is_none() || self.save_blocked {
            return false;
        }
        let file = glib::KeyFile::new();
        let _ = file.load_from_file(self.rc_path(), glib::KeyFileFlags::KEEP_COMMENTS);
        self.save_pins_to_keyfile(&file);
        self.settings.save(&file);
        self.save_associations(&file);
        let data = file.to_data();
        crate::settings::write_private_file(&self.rc_path(), data.as_bytes()).is_ok()
    }

    pub fn queue_refresh(&mut self) {
        if self.disposing || self.refresh_id.is_set() {
            return;
        }
        let weak = self.weak();
        self.refresh_id.set_timeout(35, move || {
            let Some(rc) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            crate::util::with_dock(&rc, |d| {
                d.refresh_id.take();
                d.refresh();
            });
            glib::ControlFlow::Break
        });
    }

    /// Periodically refresh the active window's cached frame so minimize
    /// animations and previews stay current (port of `frame_timer`).
    fn refresh_active_frame(&mut self) {
        let Some(active) = self.screen.as_ref().and_then(|s| s.active_window()) else {
            return;
        };
        if let Some(id) = self.window_index.get(&(active.as_ptr() as usize)).copied() {
            self.capture(id);
        }
    }

    // -- reconciliation ------------------------------------------------------

    /// Port of `zd_refresh`: prune dead windows, revalidate matches, adopt
    /// new windows and transfer windows between pins.
    pub fn refresh(&mut self) {
        if self.disposing {
            return;
        }
        let live: Vec<glib::Object> = self
            .screen
            .as_ref()
            .map(|s| s.windows())
            .unwrap_or_default();
        let is_live = |window: &glib::Object| live.iter().any(|w| w.as_ptr() == window.as_ptr());

        // Pass 1: closed windows and match revalidation.
        let mut i = 0;
        while i < self.buttons.len() {
            let b = &self.buttons[i];
            let Some(window) = b.window.clone() else {
                i += 1;
                continue;
            };
            let gone =
                !is_live(&window) || crate::ffi_xfce::Window::new(&window).is_skip_tasklist();
            if gone {
                let id = b.id;
                let pinned = b.pinned;
                if pinned {
                    self.release_window(id);
                    i += 1;
                } else {
                    self.free_button(id);
                }
                continue;
            }
            if b.match_dirty || b.match_generation != self.app_generation {
                let id = b.id;
                let pinned = b.pinned;
                let app = self.match_app(&window);
                if pinned {
                    let changed = !crate::apps::app_equal(b.app.as_ref(), app.as_ref());
                    if changed {
                        self.release_window(id);
                    }
                } else {
                    let keep = app.is_some() || b.app.is_none() || {
                        let ids = crate::ffi_xfce::Window::new(&window).class_ids();
                        b.app
                            .as_ref()
                            .is_some_and(|a| crate::apps::app_match_score(a, &ids) == 0)
                    };
                    if keep {
                        let changed = !crate::apps::app_equal(b.app.as_ref(), app.as_ref());
                        if changed {
                            let b = &mut self.buttons[i];
                            b.app = app.clone();
                            b.icon_dirty = true;
                        }
                    }
                }
                let b = &mut self.buttons[i];
                b.match_dirty = false;
                b.match_generation = self.app_generation;
            }
            i += 1;
        }

        // Pass 2: adopt new windows.
        for window in &live {
            let w = crate::ffi_xfce::Window::new(window);
            let window_type = w.window_type();
            if !w.is_skip_tasklist()
                && window_type != crate::ffi_xfce::XFW_WINDOW_TYPE_DESKTOP
                && window_type != crate::ffi_xfce::XFW_WINDOW_TYPE_DOCK
                && !self.window_index.contains_key(&(window.as_ptr() as usize))
            {
                self.add_window(window);
            }
        }

        // Pass 3: when a pin's window closes, reuse the pin for another
        // existing window of the same app, moving references, signal
        // handlers and cached frames along.
        let pin_ids: Vec<u64> = self
            .buttons
            .iter()
            .filter(|pin| {
                pin.pinned
                    && pin.app.is_some()
                    && pin.window.as_ref().is_none_or(|w| {
                        !(self.settings.all_workspaces || self.window_in_workspace(w))
                    })
            })
            .map(|pin| pin.id)
            .collect();
        for pin_id in pin_ids {
            let Some(pin) = self.button(pin_id) else {
                continue;
            };
            let Some(pin_app) = pin.app.clone() else {
                continue;
            };
            let mut candidate: Option<u64> = None;
            for b in &self.buttons {
                if b.pinned || b.window.is_none() {
                    continue;
                }
                if !crate::apps::app_equal(Some(&pin_app), b.app.as_ref()) {
                    continue;
                }
                if candidate.is_none() {
                    candidate = Some(b.id);
                }
                if !self.settings.all_workspaces
                    && b.window
                        .as_ref()
                        .is_some_and(|w| self.window_in_workspace(w))
                {
                    candidate = Some(b.id);
                    break;
                }
            }
            let Some(candidate_id) = candidate else {
                continue;
            };
            let candidate_window = {
                let pin = self.button(pin_id).unwrap();
                let Some(candidate) = self.button(candidate_id) else {
                    continue;
                };
                let Some(candidate_window) = candidate.window.clone() else {
                    continue;
                };
                if pin.window.is_some() && !self.window_in_workspace(&candidate_window) {
                    continue;
                }
                candidate_window
            };
            if let (Some(pin), Some(candidate)) = (self.button(pin_id), self.button(candidate_id)) {
                // Same content check to avoid a spurious transfer.
                if pin.id == candidate.id {
                    continue;
                }
            }
            let (candidate_frame, candidate_time) = {
                let candidate = self.button(candidate_id).unwrap();
                (candidate.thumbnail.clone(), candidate.thumbnail_time)
            };
            let (old_window, old_frame, old_time) = {
                let pin = self.button(pin_id).unwrap();
                (
                    pin.window.clone(),
                    pin.thumbnail.clone(),
                    pin.thumbnail_time,
                )
            };
            self.release_window(candidate_id);
            match old_window {
                Some(old) => {
                    self.release_window(pin_id);
                    self.attach_window(candidate_id, &old);
                    if let Some(candidate) = self.button_mut(candidate_id) {
                        candidate.thumbnail = old_frame;
                        candidate.thumbnail_time = old_time;
                    }
                }
                None => {
                    // The candidate was released above; drop the launcher.
                    if let Some(mut removed) = self.remove_candidate(candidate_id) {
                        removed.destroy_widgets();
                    }
                }
            }
            self.attach_window(pin_id, &candidate_window);
            if let Some(pin) = self.button_mut(pin_id) {
                pin.thumbnail = candidate_frame;
                pin.thumbnail_time = candidate_time;
            }
        }

        // Pass 4: mirror the button order into the box.
        for (position, b) in self.buttons.iter().enumerate() {
            self.box_.reorder_child(&b.widget, position as i32);
        }
        self.update_buttons();
        self.menu_sync();
    }

    fn remove_candidate(&mut self, button_id: u64) -> Option<Button> {
        let index = self.button_index(button_id)?;
        Some(self.buttons.remove(index))
    }

    // -- disposal ------------------------------------------------------------

    /// Port of `dispose` from plugin.c. Idempotent; also runs from `Drop`.
    pub fn dispose(&mut self) {
        self.save();
        self.disposing = true;

        self.input = None; // removes the fd source, then closes the display
        self.refresh_id.clear();
        self.active_frame_id.clear();
        self.launch_tick.clear();

        self.popups_dispose();
        if let Some(dialog) = self.association_dialog.take() {
            unsafe {
                dialog.destroy();
            }
        }
        if let Some(dialog) = self.settings_dialog.take() {
            unsafe {
                dialog.destroy();
            }
        }
        if let Some(dialog) = self.error_dialog.take() {
            unsafe {
                dialog.destroy();
            }
        }
        if let Some(dialog) = self.about_dialog.take() {
            unsafe {
                dialog.destroy();
            }
        }
        if let Some(menu) = self.menu.take() {
            unsafe {
                menu.destroy();
            }
        }
        self.audio = None; // Drop impl: timers cleared, context disconnected

        // Explicitly tear down every live XfwWindow before dropping the
        // buttons. SignalHandlerId does not disconnect on Drop, and the GDK
        // property filter owns a raw IdentityFilter allocation, so simply
        // taking the button vector would leak both registrations across
        // plugin reloads.
        let button_ids: Vec<u64> = self.buttons.iter().map(|button| button.id).collect();
        for id in button_ids {
            self.release_window(id);
        }
        let buttons: Vec<Button> = std::mem::take(&mut self.buttons);
        for mut button in buttons {
            button.destroy_widgets();
        }
        if let Some(screen) = self.screen.as_ref() {
            for handler in self.screen_handlers.drain(..) {
                screen.obj.disconnect(handler);
            }
        } else {
            self.screen_handlers.clear();
        }
        for (object, handler) in self.workspace_handlers.drain(..) {
            object.disconnect(handler);
        }
        if let Some(handler) = self.app_monitor_handler.take() {
            if let Some(monitor) = self.app_monitor.as_ref() {
                monitor.disconnect(handler);
            }
        }
        if let Some(handler) = self.theme_handler.take() {
            self.icon_theme.disconnect(handler);
        }

        self.apps.clear();
        self.associations.clear();
        self.app_monitor = None;
        self.icon_theme = gtk::IconTheme::new();
        self.screen = None;
        self.window_index.clear();
        self.rc_path = None;
    }

    // -- plugin signal handlers ----------------------------------------------

    fn size_changed(&mut self, size: i32) -> bool {
        if let Some(plugin) = &self.plugin {
            let rows = plugin.nrows().max(1);
            self.unit = (size / rows).max(28) as u32;
            let requested = plugin.icon_size();
            self.icon_size = if requested > 0 {
                (requested.max(16)).min(self.unit.saturating_sub(8) as i32)
            } else {
                self.unit.saturating_sub(8) as i32
            }
            .max(1) as u32;
        }
        self.update_buttons();
        true
    }

    fn theme_changed(&mut self) {
        for b in &mut self.buttons {
            b.icon_dirty = true;
        }
        self.update_buttons();
    }

    fn scale_changed(&mut self) {
        self.theme_changed();
        for b in &mut self.buttons {
            b.thumbnail = None;
        }
        self.hide_preview();
    }

    fn orientation_changed(&mut self, orientation: gtk::Orientation) {
        self.orientation = orientation;
        self.box_.set_orientation(orientation);
        self.container.set_orientation(orientation);
        self.hide_preview();
        self.update_buttons();
    }

    fn monitors_changed(&mut self) {
        self.hide_preview();
        if let Some(bubble) = self.bubble.as_ref() {
            bubble.hide();
        }
        self.queue_refresh();
    }

    fn show_about(&mut self) {
        if let Some(dialog) = self.about_dialog.as_ref() {
            dialog.present();
            return;
        }
        let dialog = gtk::AboutDialog::new();
        dialog.set_program_name(&t("Zero Dock 窗口停靠"));
        dialog.set_version(Some(crate::util::VERSION));
        dialog.set_comments(Some(
            t("XFCE 原生面板插件 · C / GTK3 / X11\n独立窗口、固定应用、预览与应用音量控制")
                .as_str(),
        ));
        dialog.set_license_type(gtk::License::MitX11);
        dialog.set_skip_taskbar_hint(true);
        if let Some(plugin) = &self.plugin {
            plugin.take_window(&dialog);
        }
        {
            let weak = self.weak();
            dialog.connect_destroy(move |_| {
                if let Some(rc) = weak.upgrade() {
                    crate::util::with_dock(&rc, |d| d.about_dialog = None);
                }
            });
        }
        {
            let dialog_clone = dialog.clone();
            dialog.connect_response(move |_, _| unsafe {
                dialog_clone.destroy();
            });
        }
        dialog.show();
        self.about_dialog = Some(dialog);
    }
}

// ---------------------------------------------------------------------------
// Construction
// ---------------------------------------------------------------------------

const IDENTITY_PROPERTIES: [&str; 4] = [
    "_GTK_APPLICATION_ID",
    "_KDE_NET_WM_DESKTOP_FILE",
    "_NET_STARTUP_ID",
    "_NET_WM_PID",
];

pub fn construct(plugin: XfcePanelPlugin) -> Option<DockRef> {
    if !crate::ffi_x11::is_x11() {
        let label = gtk::Label::new(Some(&t("Zero Dock 需要 X11")));
        let plugin_widget = plugin.as_object();
        if let Some(c) = plugin_widget.downcast_ref::<gtk::Container>() {
            c.add(&label)
        }
        label.show();
        return None;
    }

    let screen = Screen::default_screen();
    let orientation = plugin.orientation();
    let unit = 32.max(plugin.size()) as u32;

    let container = gtk::Box::new(orientation, 2);
    container.set_widget_name("zero-dock");
    let box_ = gtk::Box::new(orientation, 2);
    box_.set_widget_name("zero-dock");
    box_.set_size_request(12, 12);
    container.pack_start(&box_, false, false, 0);
    let overflow = gtk::Button::with_label("…");
    overflow.set_no_show_all(true);

    let css = gtk::CssProvider::new();
    css.load_from_data(
        b"#zero-dock button {padding:0; margin:0; border:0; background:none; box-shadow:none;} #zero-dock button:hover {background:alpha(@theme_selected_bg_color,0.18); border-radius:8px;} #zero-dock .zd-sound {background:@theme_bg_color;color:@theme_fg_color;border-radius:8px; min-width:16px; min-height:16px;} .zd-popup {background:@theme_bg_color; color:@theme_fg_color; border:1px solid shade(@theme_bg_color,0.65); border-radius:8px; padding:8px;} #zero-dock button:focus {box-shadow:inset 0 0 0 2px @theme_selected_bg_color;} .zd-popup button {padding:4px;}",
    )
    .ok();
    box_.style_context()
        .add_provider(&css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
    overflow
        .style_context()
        .add_provider(&css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);

    let rc_path = unsafe {
        let test_rc =
            glib::gobject_ffi::g_object_get_data(plugin.0, c"test-rc".as_ptr() as *const _);
        if !test_rc.is_null() {
            Some(
                std::ffi::CStr::from_ptr(test_rc as *const _)
                    .to_string_lossy()
                    .into_owned(),
            )
        } else {
            plugin.save_location(true)
        }
    };

    let identity_atoms: [Atom; 4] = IDENTITY_PROPERTIES
        .iter()
        .map(|name| crate::ffi_x11::intern_atom(name))
        .collect::<Vec<_>>()
        .try_into()
        .unwrap();

    let dock = Rc::new_cyclic(|weak: &DockWeak| {
        std::cell::RefCell::new(Dock {
            plugin: Some(plugin),
            screen,
            weak: weak.clone(),
            container: container.clone(),
            box_: box_.clone(),
            overflow: overflow.clone(),
            association_dialog: None,
            settings_dialog: None,
            menu: None,
            show_desktop_item: None,
            minimize_all_item: None,
            error_dialog: None,
            about_dialog: None,
            buttons: Vec::new(),
            apps: gio::AppInfo::all(),
            associations: HashMap::new(),
            app_monitor: None,
            icon_theme: gtk::IconTheme::new(),
            css: css.clone(),
            screen_handlers: Vec::new(),
            workspace_handlers: Vec::new(),
            app_monitor_handler: None,
            theme_handler: None,
            audio: None,
            input: None,
            orientation,
            unit,
            icon_size: 32,
            settings: Settings::default(),
            app_generation: 1,
            rc_path: rc_path.clone(),
            disposing: false,
            dragging: false,
            save_blocked: false,
            autohide_blocked: false,
            hover_id: Timer::default(),
            refresh_id: Timer::default(),
            active_frame_id: Timer::default(),
            launch_tick: Timer::default(),
            identity_atoms,
            drag_button: None,
            insert_button: None,
            insert_after: false,
            hover_button: None,
            preview: None,
            preview_image: None,
            preview_title: None,
            preview_sound: None,
            preview_status: None,
            preview_workspace: None,
            preview_close: None,
            preview_tick: Timer::default(),
            leave_id: Timer::default(),
            bubble: None,
            bubble_text: None,
            bubble_bar: None,
            bubble_id: Timer::default(),
            next_button_id: 0,
            window_index: HashMap::new(),
        })
    });
    let dock_rc = dock;

    // Overflow button wiring
    {
        let weak = Rc::downgrade(&dock_rc);
        overflow.connect_clicked(move |_| {
            if let Some(rc) = weak.upgrade() {
                crate::util::with_dock(&rc, |d| d.overflow_menu(None));
            }
        });
    }
    {
        let weak = Rc::downgrade(&dock_rc);
        overflow.connect_key_press_event(move |w, e| {
            let Some(rc) = weak.upgrade() else {
                return glib::Propagation::Proceed;
            };
            let mut handled = glib::Propagation::Proceed;
            crate::util::with_dock(&rc, |d| {
                if d.focus_key(w.upcast_ref(), e) {
                    handled = glib::Propagation::Stop;
                }
            });
            handled
        });
    }
    box_.connect_notify_local(Some("scale-factor"), {
        let weak = Rc::downgrade(&dock_rc);
        move |_w, _pspec| {
            if let Some(rc) = weak.upgrade() {
                crate::util::with_dock(&rc, |d| d.scale_changed());
            }
        }
    });

    let plugin_ptr = plugin;
    let plugin_obj = plugin_ptr.as_object();

    // Panel plugin signals
    plugin_obj.connect_local("free-data", false, {
        let weak = Rc::downgrade(&dock_rc);
        move |_| {
            if let Some(rc) = weak.upgrade() {
                crate::util::with_dock(&rc, |d| d.dispose());
            }
            None
        }
    });
    plugin_obj.connect_local("save", false, {
        let weak = Rc::downgrade(&dock_rc);
        move |_| {
            if let Some(rc) = weak.upgrade() {
                crate::util::with_dock(&rc, |d| {
                    d.save();
                });
            }
            None
        }
    });
    plugin_obj.connect_local("configure-plugin", false, {
        let weak = Rc::downgrade(&dock_rc);
        move |_| {
            if let Some(rc) = weak.upgrade() {
                crate::util::with_dock(&rc, |d| d.configure());
            }
            None
        }
    });
    plugin_obj.connect_local("about", false, {
        let weak = Rc::downgrade(&dock_rc);
        move |_| {
            if let Some(rc) = weak.upgrade() {
                crate::util::with_dock(&rc, |d| d.show_about());
            }
            None
        }
    });
    plugin_obj.connect_local("size-changed", false, {
        let weak = Rc::downgrade(&dock_rc);
        move |values| {
            let size = values.get(1).and_then(|v| v.get::<i32>().ok()).unwrap_or(0);
            let handled = if let Some(rc) = weak.upgrade() {
                crate::util::with_dock(&rc, |d| d.size_changed(size)).unwrap_or(true)
            } else {
                true
            };
            Some(handled.to_value())
        }
    });
    for property in ["icon-size", "nrows"] {
        plugin_obj.connect_notify_local(Some(property), {
            let weak = Rc::downgrade(&dock_rc);
            move |_obj, _pspec| {
                if let Some(rc) = weak.upgrade() {
                    crate::util::with_dock(&rc, |d| {
                        let size = d.plugin.as_ref().map(|p| p.size()).unwrap_or(0);
                        d.size_changed(size);
                    });
                }
            }
        });
    }
    plugin_obj.connect_local("orientation-changed", false, {
        let weak = Rc::downgrade(&dock_rc);
        move |values| {
            let orientation = values
                .get(1)
                .and_then(|v| v.get::<gtk::Orientation>().ok())
                .unwrap_or(gtk::Orientation::Horizontal);
            if let Some(rc) = weak.upgrade() {
                crate::util::with_dock(&rc, |d| d.orientation_changed(orientation));
            }
            None
        }
    });

    // Screen signals. Materialize the clone first so the RefCell read borrow
    // ends before handler ids are recorded with borrow_mut() below.
    let screen = { dock_rc.borrow().screen.clone() };
    if let Some(screen) = screen {
        for signal in [
            "window-opened",
            "window-closed",
            "active-window-changed",
            "window-stacking-changed",
        ] {
            let weak = Rc::downgrade(&dock_rc);
            let handler = screen.obj.connect_local(signal, false, move |_| {
                if let Some(rc) = weak.upgrade() {
                    crate::util::with_dock(&rc, |d| d.queue_refresh());
                }
                None
            });
            dock_rc.borrow_mut().screen_handlers.push(handler);
        }
        let handler = screen.obj.connect_notify_local(Some("show-desktop"), {
            let weak = Rc::downgrade(&dock_rc);
            move |_obj, _pspec| {
                if let Some(rc) = weak.upgrade() {
                    crate::util::with_dock(&rc, |d| d.queue_refresh());
                }
            }
        });
        dock_rc.borrow_mut().screen_handlers.push(handler);
        let handler = screen.obj.connect_local("monitors-changed", false, {
            let weak = Rc::downgrade(&dock_rc);
            move |_| {
                if let Some(rc) = weak.upgrade() {
                    crate::util::with_dock(&rc, |d| d.monitors_changed());
                }
                None
            }
        });
        dock_rc.borrow_mut().screen_handlers.push(handler);

        // Workspace group changes
        let manager = screen.workspace_manager();
        let groups = unsafe {
            crate::util::glist_borrow_objects(
                crate::ffi_xfce::xfw_workspace_manager_list_workspace_groups(
                    glib::translate::ToGlibPtr::to_glib_none(&manager).0,
                ),
            )
        };
        for group in groups {
            let weak = Rc::downgrade(&dock_rc);
            let handler = group.connect_local("active-workspace-changed", false, move |_| {
                if let Some(rc) = weak.upgrade() {
                    crate::util::with_dock(&rc, |d| d.queue_refresh());
                }
                None
            });
            dock_rc
                .borrow_mut()
                .workspace_handlers
                .push((group, handler));
        }
    }

    // Application index monitor
    {
        let monitor = gio::AppInfoMonitor::get();
        let weak = Rc::downgrade(&dock_rc);
        let handler = monitor.connect_changed(move |_| {
            if let Some(rc) = weak.upgrade() {
                crate::util::with_dock(&rc, |d| d.reload_apps());
            }
        });
        let mut dock = dock_rc.borrow_mut();
        dock.app_monitor_handler = Some(handler);
        dock.app_monitor = Some(monitor);
    }

    // Theme changed
    {
        let theme = dock_rc.borrow().icon_theme.clone();
        let weak = Rc::downgrade(&dock_rc);
        let handler = theme.connect_local("changed", false, move |_| {
            if let Some(rc) = weak.upgrade() {
                crate::util::with_dock(&rc, |d| d.theme_changed());
            }
            None
        });
        dock_rc.borrow_mut().theme_handler = Some(handler);
    }

    // Audio + raw input
    {
        let audio = crate::audio::Audio::new(Rc::downgrade(&dock_rc));
        dock_rc.borrow_mut().audio = audio;
    }
    {
        let input = Input::new(Rc::downgrade(&dock_rc));
        dock_rc.borrow_mut().input = input;
    }

    // Active-frame cache timer
    {
        let weak = Rc::downgrade(&dock_rc);
        let mut timer = Timer::default();
        timer.set_timeout_seconds(5, move || {
            let Some(rc) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            crate::util::with_dock(&rc, |d| d.refresh_active_frame());
            glib::ControlFlow::Continue
        });
        dock_rc.borrow_mut().active_frame_id = timer;
    }

    // Plugin presentation
    plugin_ptr.set_expand(false);
    plugin_ptr.set_shrink(true);
    plugin_ptr.menu_show_configure();
    plugin_ptr.menu_show_about();
    plugin_ptr.add_action_widget(&box_);
    plugin_ptr.add_action_widget(&overflow);

    // Drag destination for .desktop imports on the container
    {
        let targets = [gtk::TargetEntry::new(
            "text/uri-list",
            gtk::TargetFlags::empty(),
            2,
        )];
        box_.drag_dest_set(gtk::DestDefaults::ALL, &targets, gdk::DragAction::COPY);
        let weak = Rc::downgrade(&dock_rc);
        box_.connect_drag_data_received(move |_w, ctx, _x, _y, data, info, time| {
            if let Some(rc) = weak.upgrade() {
                crate::util::with_dock(&rc, |d| {
                    let ok = info == 2 && crate::button::import_uris(d, data);
                    ctx.drag_finish(ok, false, time);
                });
            }
        });
    }

    // Initial settings load
    {
        let file = glib::KeyFile::new();
        let mut initial = dock_rc.borrow().rc_path.clone();
        if let Some(path) = initial.as_ref() {
            if !std::path::Path::new(path).exists() {
                for argument in plugin_ptr.arguments() {
                    if let Some(value) = argument.strip_prefix("--import-config=") {
                        initial = Some(value.to_string());
                    }
                }
            }
        }
        let loaded = initial.as_ref().is_some_and(|path| {
            with_dock_ref(&dock_rc, |d| d.settings_read(path, &file)).unwrap_or(false)
        });
        if loaded {
            with_dock_ref(&dock_rc, |d| {
                d.settings.load(&file);
                d.load_associations(&file);
            });
            let pins: Vec<String> = file
                .string_list("Dock", "Pinned")
                .unwrap_or_default()
                .iter()
                .map(|s| s.to_string())
                .collect();
            for pin in &pins {
                with_dock_ref(&dock_rc, |d| {
                    d.add_pin(pin);
                });
            }
        }
    }

    // Overlay wiring for the overflow button
    container.pack_end(&overflow, false, false, 0);
    crate::ffi_xfce::XfcePanelPlugin::add_action_widget(&plugin_ptr, &container);

    // Show the plugin and run the first refresh.
    {
        let plugin_widget = plugin_ptr.as_object();
        let _ = plugin_widget;
    }
    if let Some(c) = plugin_ptr.as_object().downcast_ref::<gtk::Container>() {
        c.add(&container)
    }

    with_dock_ref(&dock_rc, |d| {
        d.menu_install();
        d.size_changed(d.plugin.as_ref().map(|p| p.size()).unwrap_or(48));
        d.refresh();
        d.box_.show();
        d.container.show();
        d.save();
    });

    // Hand ownership to the plugin object; free-data/finalize drops it.
    let handle = dock_rc.clone();
    unsafe {
        let boxed: Box<DockRef> = Box::new(dock_rc);
        glib::gobject_ffi::g_object_set_data_full(
            plugin_ptr.0,
            c"zero-dock".as_ptr() as *const _,
            Box::into_raw(boxed) as *mut _,
            Some(destroy_dock_data),
        );
    }

    Some(handle)
}

unsafe extern "C" fn destroy_dock_data(data: *mut std::os::raw::c_void) {
    if data.is_null() {
        return;
    }
    let dock = Box::from_raw(data as *mut DockRef);
    crate::util::with_dock(&dock, |d| {
        if !d.disposing {
            d.dispose();
        }
    });
    drop(dock);
}

fn with_dock_ref<T>(rc: &DockRef, f: impl FnOnce(&mut Dock) -> T) -> Option<T> {
    crate::util::with_dock(rc, f)
}

pub type DockRef = Rc<RefCell<Dock>>;
