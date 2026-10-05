//! Manual window-class → desktop-file associations. Ported from
//! `src/associations.c`.
//!
//! Manual associations use a SHA-256 of length-prefixed, case-normalized
//! class identifiers, preserving the entire class combination rather than
//! fuzzy names.

use crate::dock::Dock;
use crate::settings::{is_absolute_desktop_path, valid_key};
use crate::util::t;
use glib::KeyFile;
use gtk::prelude::*;

/// SHA-256 of `len:class;` pairs over the window's class ids.
pub fn association_key(window: &glib::Object) -> Option<String> {
    let ids = crate::ffi_xfce::Window::new(window).class_ids();
    let mut text = String::new();
    for id in &ids {
        let lower = id.to_lowercase();
        text.push_str(&format!("{}:{};", lower.len(), lower));
    }
    if text.is_empty() {
        None
    } else {
        Some(
            glib::compute_checksum_for_data(glib::ChecksumType::Sha256, text.as_bytes())
                .map(|s| s.to_string())
                .unwrap_or_default(),
        )
    }
}

impl Dock {
    pub fn load_associations(&mut self, file: &KeyFile) {
        self.associations.clear();
        let keys = match file.keys("Associations") {
            Ok(keys) => keys,
            Err(_) => return,
        };
        for key in keys.iter().take(512) {
            let path = file
                .string("Associations", key)
                .map(|p| p.to_string())
                .unwrap_or_default();
            if path.is_empty() {
                continue;
            }
            if valid_key(key) && is_absolute_desktop_path(&path) {
                self.associations.insert(key.to_string(), path);
            }
        }
    }

    pub fn save_associations(&self, file: &KeyFile) {
        if file.has_group("Associations") {
            let _ = file.remove_group("Associations");
        }
        for (key, path) in &self.associations {
            file.set_string("Associations", key, path);
        }
    }

    pub fn associate(
        &mut self,
        window: &glib::Object,
        path: Option<&str>,
    ) -> Result<(), glib::Error> {
        let key = association_key(window);
        if let Some(path) = path {
            let usable = key.is_some()
                && is_absolute_desktop_path(path)
                && gio::DesktopAppInfo::from_filename(path).is_some();
            if !usable {
                return Err(glib::Error::new(
                    gio::IOErrorEnum::InvalidArgument,
                    &t("窗口缺少可保存的窗口类，或所选桌面文件无效。"),
                ));
            }
        }
        match (&key, path) {
            (Some(key), Some(path)) => {
                self.associations.insert(key.clone(), path.to_string());
            }
            (Some(key), None) => {
                self.associations.remove(key);
            }
            _ => {}
        }
        self.app_generation += 1;
        self.save();
        self.queue_refresh();
        Ok(())
    }

    pub fn association_clear(&mut self, button_id: u64) {
        if let Some(window) = self.button(button_id).and_then(|b| b.window.clone()) {
            let _ = self.associate(&window, None);
        }
    }

    pub fn association_choose(&mut self, button_id: u64) {
        let Some(window) = self.button(button_id).and_then(|b| b.window.clone()) else {
            return;
        };
        if let Some(old) = self.association_dialog.take() {
            unsafe {
                old.destroy();
            }
        }
        let dialog = gtk::FileChooserDialog::with_buttons(
            Some(&t("选择关联应用的桌面文件")),
            None::<&gtk::Window>,
            gtk::FileChooserAction::Open,
            &[
                (&t("取消"), gtk::ResponseType::Cancel),
                (&t("关联"), gtk::ResponseType::Accept),
            ],
        );
        let filter = gtk::FileFilter::new();
        filter.set_name(Some(&t("应用启动器 (*.desktop)")));
        filter.add_pattern("*.desktop");
        dialog.add_filter(filter);
        let _ = dialog.set_current_folder("/usr/share/applications");

        let scope = gtk::Label::new(Some(&t(
            "此规则适用于相同窗口类的所有窗口。\n窗口类完全相同的多配置应用仍需要应用自身提供不同身份。",
        )));
        dialog.set_extra_widget(&scope);
        scope.show();
        dialog.set_skip_taskbar_hint(true);
        if let Some(plugin) = &self.plugin {
            plugin.take_window(&dialog);
        }

        // The dialog holds the window object for as long as it lives, so the
        // response handler can check it against the live window table.
        {
            let weak = self.weak();
            dialog.connect_response(move |dlg, response| {
                if response == gtk::ResponseType::Accept {
                    let path = dlg.filename();
                    if let Some(rc) = weak.upgrade() {
                        crate::util::with_dock(&rc, |d| {
                            let live = d.window_index.contains_key(&(window.as_ptr() as usize));
                            if !live {
                                d.show_error(&t("窗口已关闭，请为仍在运行的窗口设置关联。"));
                            } else {
                                let path = path.as_ref().map(|p| p.to_string_lossy().into_owned());
                                if let Err(e) = d.associate(&window, path.as_deref()) {
                                    d.show_error(e.message());
                                }
                            }
                        });
                    }
                }
                unsafe {
                    dlg.destroy();
                }
            });
        }
        {
            let weak = self.weak();
            dialog.connect_destroy(move |_| {
                if let Some(rc) = weak.upgrade() {
                    crate::util::with_dock(&rc, |d| d.association_dialog = None);
                }
            });
        }
        dialog.show();
        self.association_dialog = Some(dialog);
    }
}
