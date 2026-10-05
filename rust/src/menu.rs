//! Native window, workspace, launcher and panel menus. Ported from
//! `src/menu.c`.
//!
//! Menu items never retain button pointers: activate handlers capture the
//! dock weakly plus a button id (or window/workspace object), mirroring the
//! C code's "resolve keys against the current instance" rule.

use crate::dock::Dock;
use crate::util::t;
use gtk::prelude::*;

#[derive(Clone, Copy)]
enum Action {
    MinimizeRestore,
    Maximize,
    Fullscreen,
    Above,
    Mute,
    Launch,
    Pin,
    Close,
    Unpin,
    MoveLeft,
    MoveRight,
    Associate,
    ClearAssociation,
}

impl Dock {
    fn run_action(&mut self, action: Action, button_id: u64) {
        if self.button(button_id).is_none() {
            return; // button was freed: no-op (C checked b->closed)
        }
        let window = self.button(button_id).and_then(|b| b.window.clone());
        match action {
            Action::Associate => self.association_choose(button_id),
            Action::ClearAssociation => self.association_clear(button_id),
            Action::MinimizeRestore => {
                let Some(window) = window else { return };
                let w = crate::ffi_xfce::Window::new(&window);
                if w.is_minimized() {
                    self.activate(button_id);
                } else {
                    self.minimize(button_id);
                }
            }
            Action::Maximize => {
                let Some(window) = window else { return };
                let w = crate::ffi_xfce::Window::new(&window);
                w.set_maximized(!w.is_maximized());
            }
            Action::Fullscreen => {
                let Some(window) = window else { return };
                let w = crate::ffi_xfce::Window::new(&window);
                w.set_fullscreen(!w.is_fullscreen());
            }
            Action::Above => {
                let Some(window) = window else { return };
                let w = crate::ffi_xfce::Window::new(&window);
                w.set_above(!w.is_above());
            }
            Action::Mute => {
                self.audio_mute(button_id);
                self.volume_bubble(button_id);
            }
            Action::Launch => self.launch(button_id),
            Action::Pin => {
                if let Some(app) = self.button(button_id).and_then(|b| b.app.clone()) {
                    self.pin_app(&app);
                }
            }
            Action::Close => self.close_window(button_id),
            Action::Unpin => self.unpin(button_id),
            Action::MoveLeft | Action::MoveRight => {
                let Some(index) = self.button_index(button_id) else {
                    return;
                };
                let neighbor = match action {
                    Action::MoveLeft => index.checked_sub(1),
                    Action::MoveRight => {
                        if index + 1 < self.buttons.len() {
                            Some(index + 1)
                        } else {
                            None
                        }
                    }
                    _ => None,
                };
                if let Some(n) = neighbor {
                    let other = self.buttons[n].id;
                    self.move_button(button_id, other, matches!(action, Action::MoveRight));
                }
            }
        }
    }

    pub(crate) fn new_menu(&mut self) -> gtk::Menu {
        if let Some(old) = self.menu.take() {
            unsafe {
                old.destroy();
            }
        }
        let menu = gtk::Menu::new();
        self.menu = Some(menu.clone());
        {
            let weak = self.weak();
            menu.connect_destroy(move |w| {
                if let Some(rc) = weak.upgrade() {
                    crate::util::with_dock(&rc, |d| {
                        if d.menu.as_ref().is_some_and(|m| m.as_ptr() == w.as_ptr()) {
                            d.menu = None;
                        }
                    });
                }
            });
        }
        menu.connect_selection_done(|m| unsafe {
            m.destroy();
        });
        menu
    }

    fn add_item(
        &self,
        menu: &gtk::Menu,
        label: &str,
        action: Action,
        sensitive: bool,
        button_id: u64,
    ) {
        let item = gtk::MenuItem::with_label(label);
        item.set_sensitive(sensitive);
        let weak = self.weak();
        item.connect_activate(move |_| {
            if let Some(rc) = weak.upgrade() {
                crate::util::with_dock(&rc, |d| d.run_action(action, button_id));
            }
        });
        menu.append(&item);
    }

    fn add_window_list(&self, menu: &gtk::Menu, button_id: u64) {
        let Some(b) = self.button(button_id) else {
            return;
        };
        let b_app = b.app.clone();
        let b_application = b
            .window
            .as_ref()
            .and_then(|w| crate::ffi_xfce::Window::new(w).application());
        let sub = gtk::Menu::new();
        let mut count = 0;
        for q in &self.buttons {
            let Some(window) = q.window.as_ref() else {
                continue;
            };
            let same = crate::apps::app_equal(q.app.as_ref(), b_app.as_ref())
                || (q.app.is_none()
                    && b_app.is_none()
                    && q.window
                        .as_ref()
                        .and_then(|w| crate::ffi_xfce::Window::new(w).application())
                        == b_application);
            if !same {
                continue;
            }
            let name = crate::ffi_xfce::Window::new(window)
                .name()
                .unwrap_or_else(|| t("无标题窗口").to_string());
            let workspace = crate::ffi_xfce::Window::new(window)
                .workspace()
                .and_then(|ws| crate::ffi_xfce::Workspace::name(&ws));
            let label = match workspace {
                Some(ws) => format!("[{}] {}", ws, name),
                None => name,
            };
            let item = gtk::CheckMenuItem::with_label(&label);
            item.set_active(crate::ffi_xfce::Window::new(window).is_active());
            let window_obj = window.clone();
            let weak = self.weak();
            item.connect_activate(move |_| {
                if let Some(rc) = weak.upgrade() {
                    crate::util::with_dock(&rc, |d| {
                        // Resolve through the live window table (C used
                        // g_hash_table_lookup(d->windows, window)).
                        let target = d.window_index.get(&(window_obj.as_ptr() as usize)).copied();
                        if let Some(target) = target {
                            d.activate(target);
                        }
                    });
                }
            });
            sub.append(&item);
            count += 1;
        }
        if count > 1 {
            let item = gtk::MenuItem::with_label(&t("本应用的窗口"));
            item.set_submenu(Some(&sub));
            menu.append(&item);
        }
        // Dropping the submenu unrefs it; GtkMenu reffed it on set_submenu
        // only when attached, otherwise it is freed with the wrapper.
    }

    /// Port of `zd_window_menu`.
    pub fn window_menu(&mut self, button_id: u64, event: Option<&gdk::Event>) {
        let Some(window) = self.button(button_id).and_then(|b| b.window.clone()) else {
            return;
        };
        let menu = self.new_menu();
        self.add_window_list(&menu, button_id);
        let w = crate::ffi_xfce::Window::new(&window);
        let caps = w.capabilities();
        let (min, max, full, above) = (
            w.is_minimized(),
            w.is_maximized(),
            w.is_fullscreen(),
            w.is_above(),
        );
        let has = |bit: u32| caps & bit != 0;
        let min_label = if min {
            t("恢复窗口")
        } else {
            t("最小化")
        };
        self.add_item(
            &menu,
            &min_label,
            Action::MinimizeRestore,
            has(if min {
                crate::ffi_xfce::XFW_WINDOW_CAPABILITIES_CAN_UNMINIMIZE
            } else {
                crate::ffi_xfce::XFW_WINDOW_CAPABILITIES_CAN_MINIMIZE
            }),
            button_id,
        );
        let max_label = if max {
            t("还原大小")
        } else {
            t("最大化")
        };
        self.add_item(
            &menu,
            &max_label,
            Action::Maximize,
            has(if max {
                crate::ffi_xfce::XFW_WINDOW_CAPABILITIES_CAN_UNMAXIMIZE
            } else {
                crate::ffi_xfce::XFW_WINDOW_CAPABILITIES_CAN_MAXIMIZE
            }),
            button_id,
        );
        let full_label = if full { t("退出全屏") } else { t("全屏") };
        self.add_item(
            &menu,
            &full_label,
            Action::Fullscreen,
            has(if full {
                crate::ffi_xfce::XFW_WINDOW_CAPABILITIES_CAN_UNFULLSCREEN
            } else {
                crate::ffi_xfce::XFW_WINDOW_CAPABILITIES_CAN_FULLSCREEN
            }),
            button_id,
        );
        let above_label = if above {
            t("取消置顶")
        } else {
            t("置顶")
        };
        self.add_item(
            &menu,
            &above_label,
            Action::Above,
            has(if above {
                crate::ffi_xfce::XFW_WINDOW_CAPABILITIES_CAN_UNPLACE_ABOVE
            } else {
                crate::ffi_xfce::XFW_WINDOW_CAPABILITIES_CAN_PLACE_ABOVE
            }),
            button_id,
        );

        let move_item = gtk::MenuItem::with_label(&t("移到工作区"));
        let sub = gtk::Menu::new();
        move_item.set_submenu(Some(&sub));
        move_item.set_sensitive(has(
            crate::ffi_xfce::XFW_WINDOW_CAPABILITIES_CAN_CHANGE_WORKSPACE,
        ));
        menu.append(&move_item);
        if let Some(screen) = self.screen.as_ref() {
            let manager = screen.workspace_manager();
            let workspaces = unsafe {
                crate::ffi_xfce::xfw_workspace_manager_list_workspaces(
                    glib::translate::ToGlibPtr::to_glib_none(&manager).0,
                )
            };
            let workspaces = unsafe { crate::util::glist_borrow_objects(workspaces) };
            for ws in &workspaces {
                let name = crate::ffi_xfce::Workspace::name(ws)
                    .filter(|n| !n.is_empty())
                    .unwrap_or_else(|| {
                        t("工作区 %u").replace(
                            "%u",
                            &(crate::ffi_xfce::Workspace::number(ws) + 1).to_string(),
                        )
                    });
                let item = gtk::CheckMenuItem::with_label(&name);
                let is_current = w.workspace().is_some_and(|cur| cur.as_ptr() == ws.as_ptr());
                item.set_active(is_current);
                let ws_obj = ws.clone();
                let weak = self.weak();
                item.connect_activate(move |_| {
                    if let Some(rc) = weak.upgrade() {
                        crate::util::with_dock(&rc, |d| {
                            if let Some(b) = d.button(button_id) {
                                if let Some(window) = b.window.as_ref() {
                                    crate::ffi_xfce::Window::new(window).move_to_workspace(&ws_obj);
                                }
                            }
                        });
                    }
                });
                sub.append(&item);
            }
        }
        menu.append(&gtk::SeparatorMenuItem::new());
        let audio = self.audio_status(button_id);
        let mute_label = if audio.muted {
            t("取消应用静音")
        } else {
            t("应用静音")
        };
        self.add_item(&menu, &mute_label, Action::Mute, audio.present, button_id);
        let launch_label = t("启动新窗口");
        self.add_item(
            &menu,
            &launch_label,
            Action::Launch,
            self.button(button_id).is_some_and(|b| b.app.is_some()),
            button_id,
        );
        if self.button(button_id).is_some_and(|b| b.pinned) {
            self.add_desktop_actions(&menu, button_id);
        }
        let pinned = self.button(button_id).is_some_and(|b| b.pinned);
        let pin_label = if pinned {
            t("取消固定")
        } else {
            t("固定到启动器")
        };
        self.add_item(
            &menu,
            &pin_label,
            if pinned { Action::Unpin } else { Action::Pin },
            pinned || self.button(button_id).is_some_and(|b| b.app.is_some()),
            button_id,
        );
        menu.append(&gtk::SeparatorMenuItem::new());
        let associate_label = t("关联到应用…");
        self.add_item(&menu, &associate_label, Action::Associate, true, button_id);
        let has_rule = crate::associations::association_key(&window)
            .is_some_and(|rule| self.associations.contains_key(&rule));
        let clear_label = t("清除手动关联");
        self.add_item(
            &menu,
            &clear_label,
            Action::ClearAssociation,
            has_rule,
            button_id,
        );
        menu.append(&gtk::SeparatorMenuItem::new());
        let close_label = t("关闭窗口");
        self.add_item(&menu, &close_label, Action::Close, true, button_id);
        let anchor = self
            .button(button_id)
            .map(|b| b.main.clone().upcast::<gtk::Widget>());
        match anchor {
            Some(anchor) => self.popup_menu_at(&menu, Some(&anchor), event),
            None => self.popup_menu_at(&menu, None, event),
        }
    }

    fn add_desktop_actions(&self, menu: &gtk::Menu, button_id: u64) {
        let Some(app) = self.button(button_id).and_then(|b| b.app.clone()) else {
            return;
        };
        for action in app.list_actions() {
            let name = app.action_name(&action);
            let item = gtk::MenuItem::with_label(&name);
            let weak = self.weak();
            item.connect_activate(move |_| {
                if let Some(rc) = weak.upgrade() {
                    crate::util::with_dock(&rc, |d| {
                        let Some(b) = d.button(button_id) else { return };
                        let Some(app) = b.app.clone() else { return };
                        let context =
                            match gdk::Display::default().and_then(|d| d.app_launch_context()) {
                                Some(c) => c,
                                None => return,
                            };
                        context.set_timestamp(d.timestamp());
                        app.launch_action(&action, Some(&context));
                    });
                }
            });
            menu.append(&item);
        }
    }

    /// Port of `zd_pin_menu`.
    pub fn pin_menu(&mut self, button_id: u64, event: Option<&gdk::Event>) {
        let menu = self.new_menu();
        let launch_label = t("启动新窗口");
        self.add_item(
            &menu,
            &launch_label,
            Action::Launch,
            self.button(button_id).is_some_and(|b| b.app.is_some()),
            button_id,
        );
        self.add_desktop_actions(&menu, button_id);
        menu.append(&gtk::SeparatorMenuItem::new());
        let index = self.button_index(button_id);
        let horizontal = self.orientation == gtk::Orientation::Horizontal;
        let left_label = if horizontal {
            t("向左移动")
        } else {
            t("向上移动")
        };
        self.add_item(
            &menu,
            &left_label,
            Action::MoveLeft,
            index.is_some_and(|i| i > 0),
            button_id,
        );
        let right_label = if horizontal {
            t("向右移动")
        } else {
            t("向下移动")
        };
        self.add_item(
            &menu,
            &right_label,
            Action::MoveRight,
            index.is_some_and(|i| i + 1 < self.buttons.len()),
            button_id,
        );
        menu.append(&gtk::SeparatorMenuItem::new());
        let unpin_label = t("取消固定");
        self.add_item(&menu, &unpin_label, Action::Unpin, true, button_id);
        let anchor = self
            .button(button_id)
            .map(|b| b.main.clone().upcast::<gtk::Widget>());
        match anchor {
            Some(anchor) => self.popup_menu_at(&menu, Some(&anchor), event),
            None => self.popup_menu_at(&menu, None, event),
        }
    }

    /// Port of `zd_minimize_all`.
    pub fn minimize_all(&mut self) {
        let ids: Vec<u64> = self.buttons.iter().map(|b| b.id).collect();
        for id in ids {
            let Some(window) = self.button(id).and_then(|b| b.window.clone()) else {
                continue;
            };
            let w = crate::ffi_xfce::Window::new(&window);
            if !w.is_minimized()
                && w.capabilities() & crate::ffi_xfce::XFW_WINDOW_CAPABILITIES_CAN_MINIMIZE != 0
            {
                self.minimize(id);
            }
        }
    }

    // -- panel menu integration ----------------------------------------------

    /// Insert the show-desktop toggle and minimize-all entries into the
    /// panel's own context menu.
    pub fn menu_install(&mut self) {
        let show_desktop_item = gtk::CheckMenuItem::with_label(&t("显示桌面"));
        {
            let weak = self.weak();
            show_desktop_item.connect_activate(move |_| {
                if let Some(rc) = weak.upgrade() {
                    crate::util::with_dock(&rc, |d| {
                        d.hide_preview();
                        if let Some(screen) = d.screen.as_ref() {
                            let show = screen.show_desktop();
                            screen.set_show_desktop(!show);
                        }
                    });
                }
            });
        }
        {
            let weak = self.weak();
            show_desktop_item.connect_destroy(move |_| {
                if let Some(rc) = weak.upgrade() {
                    crate::util::with_dock(&rc, |d| d.show_desktop_item = None);
                }
            });
        }
        if let Some(plugin) = &self.plugin {
            plugin.menu_insert_item(&show_desktop_item);
        }
        show_desktop_item.show();
        self.show_desktop_item = Some(show_desktop_item);

        let minimize_all_item = gtk::MenuItem::with_label(&t("最小化所有窗口"));
        {
            let weak = self.weak();
            minimize_all_item.connect_activate(move |_| {
                if let Some(rc) = weak.upgrade() {
                    crate::util::with_dock(&rc, |d| d.minimize_all());
                }
            });
        }
        {
            let weak = self.weak();
            minimize_all_item.connect_destroy(move |_| {
                if let Some(rc) = weak.upgrade() {
                    crate::util::with_dock(&rc, |d| d.minimize_all_item = None);
                }
            });
        }
        if let Some(plugin) = &self.plugin {
            plugin.menu_insert_item(&minimize_all_item);
        }
        minimize_all_item.show();
        self.minimize_all_item = Some(minimize_all_item);
    }

    /// Keep the show-desktop toggle in sync; `set_active` does not emit the
    /// `activate` signal, so no handler blocking is required.
    pub fn menu_sync(&mut self) {
        let Some(item) = self.show_desktop_item.as_ref() else {
            return;
        };
        let show = self.screen.as_ref().is_some_and(|s| s.show_desktop());
        item.set_active(show);
    }
}
