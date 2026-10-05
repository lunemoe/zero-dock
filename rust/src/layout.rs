//! Bounded icon layout, overflow menu, shared popup events and focus
//! navigation. Ported from `src/layout.c`.

use crate::dock::Dock;
use crate::util::t;
use gtk::prelude::*;

impl Dock {
    /// Port of `zd_layout_update`: eligibility, overflow visibility and
    /// reserved-slot sizing.
    pub fn layout_update(&mut self) {
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
        let overflow = self.settings.max_visible > 0 && eligible > self.settings.max_visible;
        let limit = if overflow {
            self.settings.max_visible - 1
        } else {
            eligible
        };
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
        let mut slots = self
            .settings
            .slots
            .max(shown + if overflow { 1 } else { 0 });
        if self.settings.max_visible > 0 {
            slots = slots.min(self.settings.max_visible);
        }
        let length = (slots * (self.unit + 2)).max(12);
        self.box_.set_size_request(12, 12);
        let (w, h) = if self.orientation == gtk::Orientation::Horizontal {
            (length as i32, 12)
        } else {
            (12, length as i32)
        };
        self.container.set_size_request(w, h);
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
        if let (Some(plugin), Some(event)) = (&self.plugin, event) {
            plugin.popup_menu(menu, anchor, Some(event));
        } else {
            // gtk_menu_popup_at_widget(NULL event) still emits
            // "no trigger event for menu popup" on GTK 3. Use the legacy
            // keyboard/programmatic popup form for event-less activation.
            menu.popup_easy(0, self.timestamp());
        }
    }

    /// Port of zd_overflow_menu.
    pub fn overflow_menu(&mut self, event: Option<&gdk::Event>) {
        self.hide_preview();
        let menu = self.new_menu();
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
            let item = gtk::CheckMenuItem::with_label(&label);
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
                        let key = Some(item_key.clone());
                        let Some(key) = key else { return };
                        let target = d.buttons.iter().find(|b| b.key == *key).map(|b| b.id);
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
            menu.append(&item);
        }
        let anchor = self.overflow.clone().upcast::<gtk::Widget>();
        self.popup_menu_at(&menu, Some(&anchor), event);
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
