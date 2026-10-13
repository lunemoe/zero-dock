//! Bounded icon layout, overflow menu, shared popup events and focus
//! navigation. Ported from `src/layout.c`.

use crate::dock::Dock;
use crate::util::t;
use glib::subclass::prelude::*;
use gtk::prelude::*;

mod adaptive {
    use super::*;
    use gtk::subclass::prelude::*;
    use std::cell::{Cell, RefCell};

    type AllocateCallback = Box<dyn Fn(i32)>;

    #[derive(Default)]
    pub struct AdaptiveBox {
        pub unit: Cell<i32>,
        pub natural: Cell<i32>,
        pub allocate: RefCell<Option<AllocateCallback>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for AdaptiveBox {
        const NAME: &'static str = "ZeroDockAdaptiveBox";
        type Type = super::AdaptiveBox;
        type ParentType = gtk::Box;
    }

    impl ObjectImpl for AdaptiveBox {}
    impl ContainerImpl for AdaptiveBox {}
    impl BoxImpl for AdaptiveBox {}

    impl WidgetImpl for AdaptiveBox {
        fn request_mode(&self) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::ConstantSize
        }

        fn preferred_width(&self) -> (i32, i32) {
            if self.obj().orientation() == gtk::Orientation::Horizontal {
                (self.unit.get().max(1), self.natural.get().max(1))
            } else {
                self.parent_preferred_width()
            }
        }

        fn preferred_width_for_height(&self, _height: i32) -> (i32, i32) {
            self.preferred_width()
        }

        fn preferred_height(&self) -> (i32, i32) {
            if self.obj().orientation() == gtk::Orientation::Vertical {
                (self.unit.get().max(1), self.natural.get().max(1))
            } else {
                self.parent_preferred_height()
            }
        }

        fn preferred_height_for_width(&self, _width: i32) -> (i32, i32) {
            self.preferred_height()
        }

        fn size_allocate(&self, allocation: &gtk::Allocation) {
            let length = if self.obj().orientation() == gtk::Orientation::Horizontal {
                allocation.width()
            } else {
                allocation.height()
            };
            if let Some(callback) = self.allocate.borrow().as_ref() {
                callback(length);
            }
            // Update visibility before GtkBox allocates children; its default
            // minimum sums every child and cannot itself implement overflow.
            self.parent_size_allocate(allocation);
        }
    }
}

glib::wrapper! {
    pub struct AdaptiveBox(ObjectSubclass<adaptive::AdaptiveBox>)
        @extends gtk::Box, gtk::Container, gtk::Widget,
        @implements gtk::Orientable, gtk::Buildable;
}

impl AdaptiveBox {
    pub fn new(orientation: gtk::Orientation) -> Self {
        glib::Object::builder()
            .property("orientation", orientation)
            .build()
    }

    pub fn measure_contents(&self, unit: u32, positions: u32) {
        let imp = self.imp();
        let unit = unit as i32;
        let natural = content_length(unit, positions.max(1));
        let old_unit = imp.unit.replace(unit);
        let old_natural = imp.natural.replace(natural);
        if old_unit != unit || old_natural != natural {
            self.queue_resize();
        }
        self.set_halign(if self.orientation() == gtk::Orientation::Horizontal {
            gtk::Align::Start
        } else {
            gtk::Align::Fill
        });
        self.set_valign(if self.orientation() == gtk::Orientation::Vertical {
            gtk::Align::Start
        } else {
            gtk::Align::Fill
        });
    }

    pub fn on_allocate(&self, callback: impl Fn(i32) + 'static) {
        self.imp().allocate.replace(Some(Box::new(callback)));
    }
}

pub fn content_length(unit: i32, positions: u32) -> i32 {
    let positions = positions.max(1) as u64;
    (positions * (unit.max(1) as u64 + 2) - 2).min(i32::MAX as u64) as i32
}

pub fn visible_capacity(length: i32, unit: u32) -> u32 {
    ((length.max(1) as u64 + 2) / (unit.max(1) as u64 + 2)).max(1) as u32
}

impl Dock {
    /// Keep the natural request tied to all eligible content, even while some
    /// buttons are hidden, so gaining panel space can expand the dock again.
    pub fn layout_update(&mut self) {
        let allocation = self.container.allocation();
        let length = if self.orientation == gtk::Orientation::Horizontal {
            allocation.width()
        } else {
            allocation.height()
        };
        self.layout_update_for_length(length);
    }

    pub fn layout_update_for_length(&mut self, length: i32) {
        let mut eligible = 0u32;
        let ids: Vec<u64> = self.buttons.iter().map(|b| b.id).collect();
        for id in &ids {
            let Some(index) = self.button_index(*id) else {
                continue;
            };
            let is_eligible = {
                let b = &self.buttons[index];
                b.window.is_none()
                    || b.pinned
                    || self.settings.all_workspaces
                    || b.window
                        .as_ref()
                        .is_some_and(|w| self.window_in_workspace(w))
            };
            self.buttons[index].eligible = is_eligible;
            if is_eligible {
                eligible += 1;
            }
        }
        self.container.measure_contents(self.unit, eligible);
        let capacity = if self.container.is_realized() {
            visible_capacity(length, self.unit)
        } else {
            eligible.max(1)
        };
        self.layout_length = length;
        let overflow = eligible > capacity;
        let limit = if overflow { capacity - 1 } else { eligible };
        let mut shown = 0u32;
        for id in &ids {
            let Some(index) = self.button_index(*id) else {
                continue;
            };
            let visible = self.buttons[index].eligible && shown < limit;
            if visible {
                shown += 1;
            }
            if !visible && self.hover_button == Some(*id) {
                self.hide_preview();
            }
            self.buttons[index].widget.set_visible(visible);
        }
        self.overflow.set_visible(overflow);
        self.overflow
            .set_size_request(self.unit as i32, self.unit as i32);
        let tip = t("更多窗口与启动器（%u）").replace("%u", &(eligible - shown).to_string());
        self.overflow.set_tooltip_text(Some(&tip));
        if let Some(accessible) = self.overflow.accessible() {
            accessible.set_name(&tip);
        }
        self.empty_launcher.set_visible(eligible == 0);
        self.empty_launcher
            .set_size_request(self.unit as i32, self.unit as i32);
        self.box_
            .reorder_child(&self.overflow, self.buttons.len() as i32);
        self.box_
            .reorder_child(&self.empty_launcher, self.buttons.len() as i32 + 1);
        self.update_overflow_status();
    }

    /// Pop up a panel menu at the anchor.
    ///
    /// Xfce's popup helper expects a real trigger event and GTK emits a warning
    /// when it receives NULL. Button/keyboard activation has no GDK event, so
    /// use GTK's anchored popup in that case instead of manufacturing an event.
    pub fn popup_menu_at(
        &self,
        menu: &gtk::Menu,
        anchor: Option<&gtk::Widget>,
        event: Option<&gdk::Event>,
    ) {
        menu.show_all();
        match (&self.plugin, event) {
            (Some(plugin), Some(event)) => plugin.popup_menu(menu, anchor, Some(event)),
            // No plugin (X11-less fallback) or no GDK event: gtk_menu_popup_at_*
            // with a NULL event emits "no trigger event for menu popup" on
            // GTK 3, so use the legacy popup form. Register the menu explicitly
            // first: that is what keeps the panel from auto-hiding while the
            // menu is up.
            _ => {
                if let Some(plugin) = &self.plugin {
                    plugin.register_menu(menu);
                }
                menu.popup_easy(0, self.timestamp());
            }
        }
    }

    /// Port of zd_overflow_menu.
    pub fn overflow_menu(&mut self, event: Option<&gdk::Event>) {
        self.hide_preview();
        let menu = self.new_menu();
        let mut targets = Vec::new();
        for b in &self.buttons {
            if !b.eligible || b.widget.is_visible() {
                continue;
            }
            let name = b.display_name();
            let workspace = b.window.as_ref().and_then(ffi_workspace_name);
            let label = match workspace {
                Some(ws) => format!("[{}] {}", ws, name),
                None => name,
            };
            let item = gtk::CheckMenuItem::new();
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            let icon = b
                .app
                .as_ref()
                .and_then(|app| {
                    use gio::prelude::*;
                    app.icon()
                })
                .map(|icon| gtk::Image::from_gicon(&icon, gtk::IconSize::Menu))
                .unwrap_or_else(|| {
                    gtk::Image::from_icon_name(
                        Some(if b.pinned && b.app.is_none() {
                            "dialog-warning-symbolic"
                        } else {
                            "application-x-executable-symbolic"
                        }),
                        gtk::IconSize::Menu,
                    )
                });
            row.pack_start(&icon, false, false, 0);
            let text = gtk::Label::new(Some(&label));
            text.set_xalign(0.0);
            text.set_ellipsize(pango::EllipsizeMode::End);
            text.set_max_width_chars(48);
            row.pack_start(&text, true, true, 0);
            if let Some(window) = b.window.as_ref() {
                let window = crate::ffi_xfce::Window::new(window);
                if window.is_minimized() {
                    row.pack_end(&gtk::Label::new(Some(&t("已最小化"))), false, false, 0);
                }
                if window.is_urgent() {
                    row.pack_end(
                        &gtk::Image::from_icon_name(
                            Some("dialog-warning-symbolic"),
                            gtk::IconSize::Menu,
                        ),
                        false,
                        false,
                        0,
                    );
                }
            }
            if b.audio.present && (b.audio.playing || b.audio.muted) {
                row.pack_end(
                    &gtk::Image::from_icon_name(
                        Some(if b.audio.muted {
                            "audio-volume-muted-symbolic"
                        } else {
                            "audio-volume-high-symbolic"
                        }),
                        gtk::IconSize::Menu,
                    ),
                    false,
                    false,
                    0,
                );
            }
            item.set_child(Some(&row));
            item.set_tooltip_text(Some(&t("点击切换或启动；右键或 Shift+F10 打开操作。")));
            if let Some(accessible) = item.accessible() {
                accessible.set_name(&label);
            }
            let active = b
                .window
                .as_ref()
                .is_some_and(|w| crate::ffi_xfce::Window::new(w).is_active());
            item.set_active(active);
            let weak = self.weak();
            let item_key = b.key.clone();
            item.connect_activate(move |_| {
                if let Some(rc) = weak.upgrade() {
                    crate::util::with_dock(&rc, |d| {
                        let target = d.buttons.iter().find(|b| b.key == item_key).map(|b| b.id);
                        if let Some(target) = target {
                            if d.button(target).is_some_and(|b| b.window.is_some()) {
                                d.activate(target);
                            } else {
                                d.toggle(target);
                            }
                        }
                    });
                }
            });
            let weak = self.weak();
            let item_key = b.key.clone();
            item.connect_button_press_event(move |_, event| {
                if event.button() != 2 && event.button() != 3 {
                    return glib::Propagation::Proceed;
                }
                if let Some(rc) = weak.upgrade() {
                    crate::util::with_dock(&rc, |d| {
                        let target = d.buttons.iter().find(|b| b.key == item_key).map(|b| b.id);
                        if let Some(target) = target {
                            if event.button() == 2 {
                                d.middle_click(target);
                            } else {
                                d.button_menu(target, Some(&**event));
                            }
                        }
                    });
                }
                glib::Propagation::Stop
            });
            targets.push((item.clone().upcast::<gtk::Widget>(), b.key.clone()));
            menu.append(&item);
        }
        let weak = self.weak();
        menu.connect_key_press_event(move |menu, event| {
            use gdk::keys::constants as Key;
            if event.keyval() != Key::Menu
                && !(event.keyval() == Key::F10
                    && event.state().contains(gdk::ModifierType::SHIFT_MASK))
            {
                return glib::Propagation::Proceed;
            }
            if let Some(selected) = menu.selected_item() {
                if let Some((_, key)) = targets.iter().find(|(widget, _)| *widget == selected) {
                    if let Some(rc) = weak.upgrade() {
                        crate::util::with_dock(&rc, |d| {
                            if let Some(target) =
                                d.buttons.iter().find(|b| &b.key == key).map(|b| b.id)
                            {
                                d.button_menu(target, Some(&**event));
                            }
                        });
                    }
                }
            }
            glib::Propagation::Stop
        });
        let anchor = self.overflow.clone().upcast::<gtk::Widget>();
        self.popup_menu_at(&menu, Some(&anchor), event);
    }

    pub fn update_overflow_status(&self) {
        let hidden: Vec<_> = self
            .buttons
            .iter()
            .filter(|b| b.eligible && !b.widget.is_visible())
            .collect();
        let active = hidden.iter().any(|b| {
            b.window
                .as_ref()
                .is_some_and(|w| crate::ffi_xfce::Window::new(w).is_active())
        });
        let urgent = hidden.iter().any(|b| {
            b.window
                .as_ref()
                .is_some_and(|w| crate::ffi_xfce::Window::new(w).is_urgent())
        });
        let audio = hidden
            .iter()
            .any(|b| b.audio.present && (b.audio.playing || b.audio.muted));
        let context = self.overflow.style_context();
        for (name, enabled) in [
            ("zd-overflow-urgent", urgent),
            ("zd-overflow-active", active),
        ] {
            if enabled {
                context.add_class(name);
            } else {
                context.remove_class(name);
            }
        }
        self.overflow.set_label(if audio { "… ♪" } else { "…" });
        let mut tip = t("更多窗口与启动器（%u）").replace("%u", &hidden.len().to_string());
        if active {
            tip.push_str(&format!("\n{}", t("活动窗口位于更多菜单中")));
        }
        if urgent {
            tip.push_str(&format!("\n{}", t("有窗口请求注意")));
        }
        if audio {
            tip.push_str(&format!("\n{}", t("有应用音频")));
        }
        self.overflow.set_tooltip_text(Some(&tip));
        if let Some(accessible) = self.overflow.accessible() {
            accessible.set_name(&tip);
        }
    }

    /// Port of `zd_focus_key`: arrow/Home/End navigation over visible
    /// buttons plus the overflow control.
    pub fn focus_key(&self, widget: &gtk::Widget, event: &gdk::EventKey) -> bool {
        use gdk::keys::constants as Key;
        let keyval = event.keyval();
        let delta: i32 = if keyval == Key::Right || keyval == Key::Down {
            1
        } else if keyval == Key::Left || keyval == Key::Up {
            -1
        } else {
            0
        };
        if delta == 0 && keyval != Key::Home && keyval != Key::End {
            return false;
        }
        let mut items: Vec<gtk::Widget> = Vec::new();
        for b in &self.buttons {
            if b.widget.is_visible() {
                items.push(b.main.clone().upcast());
            }
        }
        if self.overflow.is_visible() {
            items.push(self.overflow.clone().upcast());
        }
        let Some(position) = items.iter().position(|w| w.as_ptr() == widget.as_ptr()) else {
            return true;
        };
        let next = if keyval == Key::Home {
            0
        } else if keyval == Key::End {
            items.len() - 1
        } else {
            ((position as i32 + delta + items.len() as i32) % items.len() as i32) as usize
        };
        items[next].grab_focus();
        true
    }
}

fn ffi_workspace_name(window: &glib::Object) -> Option<String> {
    crate::ffi_xfce::Window::new(window)
        .workspace()
        .and_then(|ws| crate::ffi_xfce::Workspace::name(&ws))
}
