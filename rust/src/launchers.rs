//! Native application selection and persistent launcher/rule management.

use crate::dock::Dock;
use crate::ffi_gtk;
use crate::util::t;
use gio::prelude::*;
use gtk::prelude::*;

impl Dock {
    pub fn close_launcher_dialogs(&mut self) {
        if let Some(dialog) = self.application_dialog.take() {
            ffi_gtk::destroy(&dialog);
        }
        if let Some(dialog) = self.launcher_dialog.take() {
            ffi_gtk::destroy(&dialog);
        }
        self.launcher_list = None;
        self.association_list = None;
    }

    /// GTK's application chooser supplies the installed-app list and search;
    /// custom desktop files remain available through a native file chooser.
    pub fn choose_application(&mut self, replace: Option<u64>) {
        if let Some(dialog) = self.application_dialog.as_ref() {
            dialog.present();
            return;
        }
        let parent = self
            .launcher_dialog
            .as_ref()
            .or(self.settings_dialog.as_ref());
        let dialog = gtk::AppChooserDialog::for_content_type(
            parent,
            gtk::DialogFlags::DESTROY_WITH_PARENT,
            "application/octet-stream",
        );
        dialog.set_title(&if replace.is_some() {
            t("更换固定应用")
        } else {
            t("添加固定应用")
        });
        dialog.set_heading(&t("选择应用，或选择本机的桌面文件。"));
        if let Ok(chooser) = dialog.widget().downcast::<gtk::AppChooserWidget>() {
            chooser.set_show_all(true);
            chooser.set_show_default(false);
            chooser.set_show_recommended(false);
            chooser.set_show_fallback(false);
        }
        dialog.add_button(&t("选择桌面文件…"), gtk::ResponseType::Other(1));
        if let Some(button) = dialog
            .widget_for_response(gtk::ResponseType::Ok)
            .and_then(|widget| widget.downcast::<gtk::Button>().ok())
        {
            button.set_label(&if replace.is_some() {
                t("更换")
            } else {
                t("固定")
            });
        }
        show_application_search(dialog.upcast_ref());
        dialog.set_skip_taskbar_hint(true);
        if let Some(plugin) = &self.plugin {
            plugin.take_window(&dialog);
        }
        let weak = self.weak();
        dialog.connect_response(move |dlg, response| {
            let app = dlg
                .app_info()
                .and_then(|app| app.downcast::<gio::DesktopAppInfo>().ok());
            // Release the chooser before constructing its optional replacement.
            ffi_gtk::destroy(dlg);
            if let Some(rc) = weak.upgrade() {
                crate::util::with_dock(&rc, |d| {
                    if response == gtk::ResponseType::Other(1) {
                        d.choose_launcher_file(replace);
                    } else if response == gtk::ResponseType::Ok
                        || response == gtk::ResponseType::Accept
                    {
                        if let Some(app) = app {
                            d.apply_launcher_selection(replace, &app);
                        }
                    }
                });
            }
        });
        self.track_application_dialog(&dialog.clone().upcast());
        dialog.show();
    }

    fn track_application_dialog(&mut self, dialog: &gtk::Dialog) {
        let weak = self.weak();
        dialog.connect_destroy(move |dlg| {
            if let Some(rc) = weak.upgrade() {
                crate::util::with_dock(&rc, |d| {
                    if d.application_dialog
                        .as_ref()
                        .is_some_and(|current| current == dlg)
                    {
                        d.application_dialog = None;
                    }
                });
            }
        });
        self.application_dialog = Some(dialog.clone());
    }

    fn choose_launcher_file(&mut self, replace: Option<u64>) {
        let parent = self
            .launcher_dialog
            .as_ref()
            .or(self.settings_dialog.as_ref());
        let dialog = gtk::FileChooserDialog::with_buttons(
            Some(&t("选择应用的桌面文件")),
            parent,
            gtk::FileChooserAction::Open,
            &[
                (&t("取消"), gtk::ResponseType::Cancel),
                (&t("选择"), gtk::ResponseType::Accept),
            ],
        );
        let filter = gtk::FileFilter::new();
        filter.add_pattern("*.desktop");
        filter.set_name(Some(&t("应用启动器 (*.desktop)")));
        dialog.add_filter(filter);
        dialog.set_skip_taskbar_hint(true);
        if let Some(plugin) = &self.plugin {
            plugin.take_window(&dialog);
        }
        let weak = self.weak();
        dialog.connect_response(move |dlg, response| {
            if response == gtk::ResponseType::Accept {
                let app = dlg.filename().and_then(gio::DesktopAppInfo::from_filename);
                if let Some(rc) = weak.upgrade() {
                    crate::util::with_dock(&rc, |d| match app {
                        Some(app) => d.apply_launcher_selection(replace, &app),
                        None => d.show_error(&t("所选桌面文件无效。")),
                    });
                }
            }
            ffi_gtk::destroy(dlg);
        });
        self.track_application_dialog(&dialog.clone().upcast());
        dialog.show();
    }

    fn apply_launcher_selection(&mut self, replace: Option<u64>, app: &gio::DesktopAppInfo) {
        if let Some(id) = replace {
            if let Err(error) = self.replace_pin(id, app) {
                self.show_error(error.message());
            }
        } else {
            self.pin_app(app);
        }
        self.refresh_launcher_manager();
    }

    pub fn replace_pin(&mut self, id: u64, app: &gio::DesktopAppInfo) -> Result<(), glib::Error> {
        let Some(path) = app
            .filename()
            .map(|path| path.to_string_lossy().into_owned())
        else {
            return Err(glib::Error::new(
                gio::IOErrorEnum::InvalidArgument,
                &t("所选桌面文件无效。"),
            ));
        };
        if self.buttons.iter().any(|button| {
            button.id != id && button.pinned && button.desktop.as_deref() == Some(&path)
        }) {
            return Err(glib::Error::new(
                gio::IOErrorEnum::Exists,
                &t("此应用已固定，请选择其他应用。"),
            ));
        }
        let Some(button) = self.button_mut(id).filter(|button| button.pinned) else {
            return Err(glib::Error::new(
                gio::IOErrorEnum::NotFound,
                &t("固定应用已移除。"),
            ));
        };
        button.desktop = Some(path.clone());
        button.app = Some(app.clone());
        button.key = format!("pin:{path}");
        button.launching = false;
        button.launch_until = 0;
        button.launch_error = None;
        button.icon_dirty = true;
        self.app_generation += 1;
        self.audio_dirty = true;
        self.refresh();
        self.refresh_launcher_manager();
        if self.save() {
            Ok(())
        } else {
            Err(self.save_error.clone().unwrap_or_else(|| {
                glib::Error::new(gio::IOErrorEnum::Failed, &t("配置尚未保存。"))
            }))
        }
    }

    pub fn manage_launchers(&mut self) {
        if let Some(dialog) = self.launcher_dialog.as_ref() {
            dialog.present();
            return;
        }
        let dialog = gtk::Dialog::with_buttons(
            Some(&t("固定应用与关联")),
            self.settings_dialog.as_ref(),
            gtk::DialogFlags::DESTROY_WITH_PARENT,
            &[(&t("关闭"), gtk::ResponseType::Close)],
        );
        dialog.set_skip_taskbar_hint(true);
        if let Some(plugin) = &self.plugin {
            plugin.take_window(&dialog);
        }
        let area = self.monitor_area();
        dialog.set_default_size(
            600.min((area.width() - 48).max(200)),
            500.min((area.height() - 48).max(200)),
        );
        let content = dialog.content_area();
        content.set_border_width(12);
        content.set_spacing(12);
        let add = gtk::Button::with_label(&t("添加固定应用…"));
        let weak = self.weak();
        add.connect_clicked(move |_| {
            if let Some(rc) = weak.upgrade() {
                crate::util::with_dock(&rc, |d| d.choose_application(None));
            }
        });
        content.pack_start(&add, false, false, 0);
        let notebook = gtk::Notebook::new();
        let pins = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let rules = gtk::Box::new(gtk::Orientation::Vertical, 8);
        for (title, list) in [(t("固定应用"), &pins), (t("手动关联"), &rules)] {
            let scroller = gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Never)
                .vscrollbar_policy(gtk::PolicyType::Automatic)
                .build();
            scroller.set_child(Some(list));
            notebook.append_page(&scroller, Some(&gtk::Label::new(Some(&title))));
        }
        content.pack_start(&notebook, true, true, 0);
        self.launcher_list = Some(pins);
        self.association_list = Some(rules);
        self.launcher_dialog = Some(dialog.clone());
        self.refresh_launcher_manager();
        dialog.connect_response(|dlg, _| ffi_gtk::destroy(dlg));
        let weak = self.weak();
        dialog.connect_destroy(move |dlg| {
            if let Some(rc) = weak.upgrade() {
                crate::util::with_dock(&rc, |d| {
                    if d.launcher_dialog
                        .as_ref()
                        .is_some_and(|current| current == dlg)
                    {
                        d.launcher_dialog = None;
                        d.launcher_list = None;
                        d.association_list = None;
                    }
                });
            }
        });
        dialog.show_all();
    }

    pub fn refresh_launcher_manager(&self) {
        if let Some(list) = self.launcher_list.as_ref() {
            for child in list.children() {
                ffi_gtk::destroy(&child);
            }
            let pins: Vec<_> = self.buttons.iter().filter(|button| button.pinned).collect();
            if pins.is_empty() {
                list.add(&gtk::Label::new(Some(&t("暂无固定应用"))));
            }
            for (index, button) in pins.iter().enumerate() {
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                let text = gtk::Box::new(gtk::Orientation::Vertical, 3);
                let name = button
                    .app
                    .as_ref()
                    .map(|app| app.display_name().to_string())
                    .unwrap_or_else(|| t("失效的固定应用").to_string());
                let title = gtk::Label::new(Some(&name));
                title.set_xalign(0.0);
                title.set_ellipsize(pango::EllipsizeMode::End);
                title.set_max_width_chars(40);
                text.pack_start(&title, false, false, 0);
                let path = gtk::Label::new(button.desktop.as_deref());
                path.set_xalign(0.0);
                path.set_ellipsize(pango::EllipsizeMode::Middle);
                path.set_max_width_chars(40);
                path.set_tooltip_text(button.desktop.as_deref());
                text.pack_start(&path, false, false, 0);
                row.pack_start(&text, true, true, 0);
                let id = button.id;
                for (symbol, neighbor, after) in [
                    (
                        "go-up-symbolic",
                        index.checked_sub(1).map(|i| pins[i].id),
                        false,
                    ),
                    (
                        "go-down-symbolic",
                        pins.get(index + 1).map(|button| button.id),
                        true,
                    ),
                ] {
                    let move_ = gtk::Button::from_icon_name(Some(symbol), gtk::IconSize::Button);
                    move_.set_sensitive(neighbor.is_some());
                    move_.set_tooltip_text(Some(&if after {
                        t("向后移动")
                    } else {
                        t("向前移动")
                    }));
                    let weak = self.weak();
                    move_.connect_clicked(move |_| {
                        if let (Some(rc), Some(neighbor)) = (weak.upgrade(), neighbor) {
                            crate::util::with_dock(&rc, |d| d.move_button(id, neighbor, after));
                        }
                    });
                    row.pack_start(&move_, false, false, 0);
                }
                let replace = gtk::Button::with_label(&t("更换…"));
                let weak = self.weak();
                replace.connect_clicked(move |_| {
                    if let Some(rc) = weak.upgrade() {
                        crate::util::with_dock(&rc, |d| d.choose_application(Some(id)));
                    }
                });
                row.pack_start(&replace, false, false, 0);
                let remove = gtk::Button::with_label(&t("取消固定"));
                let weak = self.weak();
                remove.connect_clicked(move |_| {
                    if let Some(rc) = weak.upgrade() {
                        crate::util::with_dock(&rc, |d| d.unpin(id));
                    }
                });
                row.pack_start(&remove, false, false, 0);
                list.add(&row);
            }
            list.show_all();
        }
        if let Some(list) = self.association_list.as_ref() {
            for child in list.children() {
                ffi_gtk::destroy(&child);
            }
            let mut rules: Vec<_> = self.associations.iter().collect();
            rules.sort_by(|a, b| a.1.cmp(b.1).then_with(|| a.0.cmp(b.0)));
            if rules.is_empty() {
                list.add(&gtk::Label::new(Some(&t("暂无手动关联"))));
            }
            for (key, path) in rules {
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                let name = gio::DesktopAppInfo::from_filename(path)
                    .map(|app| app.display_name().to_string())
                    .unwrap_or_else(|| t("失效的关联应用").to_string());
                let classes = self
                    .buttons
                    .iter()
                    .filter_map(|button| button.window.as_ref())
                    .find(|window| {
                        crate::associations::association_key(window).as_ref() == Some(key)
                    })
                    .map(|window| crate::ffi_xfce::Window::new(window).class_ids().join(" / "))
                    .unwrap_or_else(|| t("当前没有匹配窗口").to_string());
                let label = gtk::Label::new(Some(&format!("{name}\n{classes}")));
                label.set_xalign(0.0);
                label.set_ellipsize(pango::EllipsizeMode::Middle);
                label.set_max_width_chars(40);
                label.set_tooltip_text(Some(path));
                row.pack_start(&label, true, true, 0);
                let remove = gtk::Button::with_label(&t("删除关联"));
                let key = key.clone();
                let weak = self.weak();
                remove.connect_clicked(move |_| {
                    if let Some(rc) = weak.upgrade() {
                        crate::util::with_dock(&rc, |d| {
                            d.associations.remove(&key);
                            d.app_generation += 1;
                            d.save();
                            d.queue_refresh();
                            d.refresh_launcher_manager();
                        });
                    }
                });
                row.pack_start(&remove, false, false, 0);
                list.add(&row);
            }
            list.show_all();
        }
    }
}

fn show_application_search(widget: &gtk::Widget) {
    if let Some(bar) = widget.downcast_ref::<gtk::SearchBar>() {
        bar.set_search_mode(true);
    }
    if let Some(container) = widget.downcast_ref::<gtk::Container>() {
        for child in container.children() {
            show_application_search(&child);
        }
    }
}
