//! Validated preferences, native error UI, diagnostics and config
//! import/export. Ported from `src/settings.c`.

use crate::dock::Dock;
use crate::ffi_gtk;
use crate::util::{real_time_us, t};
use glib::KeyFile;
use gtk::prelude::*;

/// Typed dock preferences, stored in the `Dock` group of the rc file.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    pub previews: bool,
    pub numbers: bool,
    pub all_workspaces: bool,
    pub scroll_windows: bool,
    pub preview_width: u32,
    pub preview_delay: u32,
    pub preview_interval: u32,
    pub launch_timeout: u32,
    pub left_action: u32,
    pub middle_action: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            previews: true,
            numbers: true,
            all_workspaces: true,
            scroll_windows: true,
            preview_width: 300,
            preview_delay: 350,
            preview_interval: 650,
            launch_timeout: 10000,
            left_action: 0,
            middle_action: 0,
        }
    }
}

fn integer_setting(f: &KeyFile, key: &str, fallback: u32, low: i32, high: i32) -> u32 {
    match f.integer("Dock", key) {
        Ok(value) => value.clamp(low, high) as u32,
        Err(_) => fallback,
    }
}

fn boolean_setting(f: &KeyFile, key: &str, fallback: bool) -> bool {
    f.boolean("Dock", key).unwrap_or(fallback)
}

impl Settings {
    pub fn load(&mut self, f: &KeyFile) {
        self.previews = boolean_setting(f, "Previews", self.previews);
        self.numbers = boolean_setting(f, "Numbers", self.numbers);
        self.all_workspaces = boolean_setting(f, "AllWorkspaces", self.all_workspaces);
        self.preview_width = integer_setting(f, "PreviewWidth", self.preview_width, 180, 600);
        self.preview_delay = integer_setting(f, "PreviewDelay", self.preview_delay, 100, 2000);
        self.preview_interval =
            integer_setting(f, "PreviewInterval", self.preview_interval, 200, 2000);
        self.launch_timeout = integer_setting(f, "LaunchTimeout", self.launch_timeout, 2000, 60000);
        self.left_action = integer_setting(f, "LeftAction", self.left_action, 0, 1);
        self.middle_action = integer_setting(f, "MiddleAction", self.middle_action, 0, 2);
        self.scroll_windows = boolean_setting(f, "ScrollWindows", self.scroll_windows);
    }

    pub fn save(&self, f: &KeyFile) {
        f.set_boolean("Dock", "Previews", self.previews);
        f.set_boolean("Dock", "Numbers", self.numbers);
        f.set_boolean("Dock", "AllWorkspaces", self.all_workspaces);
        f.set_integer("Dock", "PreviewWidth", self.preview_width as i32);
        f.set_integer("Dock", "PreviewDelay", self.preview_delay as i32);
        f.set_integer("Dock", "PreviewInterval", self.preview_interval as i32);
        f.set_integer("Dock", "LaunchTimeout", self.launch_timeout as i32);
        // Migrate fixed-length layouts: neither old key controls allocation.
        let _ = f.remove_key("Dock", "Slots");
        let _ = f.remove_key("Dock", "MaxVisible");
        f.set_integer("Dock", "LeftAction", self.left_action as i32);
        f.set_integer("Dock", "MiddleAction", self.middle_action as i32);
        f.set_boolean("Dock", "ScrollWindows", self.scroll_windows);
    }
}

impl Dock {
    pub fn update_save_status(&self) {
        if let Some(row) = self.save_status.as_ref() {
            row.set_visible(self.save_error.is_some());
        }
        if let Some(label) = self.save_status_label.as_ref() {
            label.set_text(
                &self
                    .save_error
                    .as_ref()
                    .map(|error| t("尚未保存：%s").replace("%s", error.message()))
                    .unwrap_or_default(),
            );
        }
    }

    /// Load `path` into `file`; on a non-missing read error, keep a backup and
    /// block saving until the configuration is fixed (mirrors settings.c).
    pub fn settings_read(&mut self, path: &str, file: &KeyFile) -> bool {
        match file.load_from_file(path, glib::KeyFileFlags::KEEP_COMMENTS) {
            Ok(()) => true,
            Err(error) => {
                if !error.matches(glib::FileError::Noent) {
                    let backup = format!("{}.invalid-{}", self.rc_path(), real_time_us());
                    let saved = std::fs::read(path)
                        .ok()
                        .and_then(|contents| write_private_file(&backup, &contents).ok())
                        .is_some();
                    self.save_blocked = !saved;
                    let message = if saved {
                        t("配置文件无法读取，已保留原文件备份并使用默认设置。")
                    } else {
                        t("配置文件无法读取，设置暂不保存。请修复配置后重新加载插件。")
                    };
                    self.show_error(&message);
                }
                let _ = error;
                false
            }
        }
    }

    pub fn show_error(&mut self, message: &str) {
        if let Some(old) = self.error_dialog.take() {
            ffi_gtk::destroy(&old);
        }
        let parent = self
            .settings_dialog
            .clone()
            .map(|s| s.upcast::<gtk::Window>());
        let dialog = gtk::MessageDialog::new(
            parent.as_ref(),
            gtk::DialogFlags::DESTROY_WITH_PARENT,
            gtk::MessageType::Error,
            gtk::ButtonsType::Close,
            message,
        );
        dialog.set_title(&t("Zero Dock 提示"));
        dialog.set_skip_taskbar_hint(true);
        if let Some(plugin) = &self.plugin {
            plugin.take_window(&dialog);
        }
        {
            let weak = self.weak();
            dialog.connect_destroy(move |_| {
                if let Some(rc) = weak.upgrade() {
                    crate::util::with_dock(&rc, |d| {
                        d.error_dialog = None;
                    });
                }
            });
        }
        {
            let weak = self.weak();
            dialog.connect_response(move |dlg, _| {
                if let Some(rc) = weak.upgrade() {
                    crate::util::with_dock(&rc, |d| {
                        if d.error_dialog
                            .as_ref()
                            .is_some_and(|cur| cur.as_ptr() == dlg.as_ptr())
                        {
                            d.error_dialog = None;
                        }
                    });
                }
                ffi_gtk::destroy(dlg);
            });
        }
        dialog.show();
        self.error_dialog = Some(dialog);
    }

    pub fn diagnostics(&self) -> String {
        let pins = self.buttons.iter().filter(|b| b.pinned).count();
        let missing = self
            .buttons
            .iter()
            .filter(|b| b.pinned && b.app.is_none())
            .count();
        format!(
            "Zero Dock {}\nBackend: X11\nGTK: {}.{}.{}\nWindows: {}\nPinned: {}\nUnavailable launchers: {}\nOrientation: {}\nIcon size: {}\nScale: {}\nPreviews: {}\nPreview width: {}\nPreview delay: {} ms\nPreview interval: {} ms\nLaunch timeout: {} ms\nAll workspaces: {}\nLayout: adaptive\nAllocated length: {}\nHidden eligible buttons: {}\nManual associations: {}\nAudio connection: {}\nCompositor: {}\nUnsaved changes: {}\nUnrecognized windows: {}\nPreview captures: {}\nFailed captures: {}\n",
            crate::util::VERSION,
            gtk::major_version(),
            gtk::minor_version(),
            gtk::micro_version(),
            self.window_index.len(),
            pins,
            missing,
            if self.orientation == gtk::Orientation::Horizontal { "horizontal" } else { "vertical" },
            self.icon_size,
            self.box_.scale_factor(),
            self.settings.previews,
            self.settings.preview_width,
            self.settings.preview_delay,
            self.settings.preview_interval,
            self.settings.launch_timeout,
            self.settings.all_workspaces,
            self.layout_length,
            self.buttons.iter().filter(|b| b.eligible && !b.widget.is_visible()).count(),
            self.associations.len(),
            self.audio.as_ref().and_then(|audio| audio.try_borrow().ok())
                .map(|audio| audio.connection_state()).unwrap_or("unavailable"),
            gdk::Screen::default().is_some_and(|screen| screen.is_composited()),
            self.save_error.is_some(),
            self.buttons.iter().filter(|b| b.window.is_some() && b.app.is_none()).count(),
            self.capture_attempts,
            self.capture_failures,
        )
    }

    pub fn config_export(&mut self, path: &str) -> Result<(), glib::Error> {
        if self.save_blocked {
            return Err(glib::Error::new(
                gio::IOErrorEnum::Failed,
                &t("当前配置不可保存，请先修复配置文件。"),
            ));
        }
        if !self.save() {
            return Err(glib::Error::new(
                gio::IOErrorEnum::Failed,
                &t("无法保存当前配置，未创建备份。"),
            ));
        }
        let contents = std::fs::read(self.rc_path())
            .map_err(|e| glib::Error::new(glib::FileError::Io, &format!("{}", e)))?;
        write_private_file(path, &contents)
            .map_err(|e| glib::Error::new(glib::FileError::Io, &format!("{}", e)))
    }

    pub fn config_import(&mut self, path: &str) -> Result<(), glib::Error> {
        let meta = std::fs::metadata(path).map_err(|_| {
            glib::Error::new(
                gio::IOErrorEnum::InvalidData,
                &t("配置必须是小于 1 MiB 的普通文件。"),
            )
        })?;
        if !meta.is_file() || meta.len() > 1024 * 1024 {
            return Err(glib::Error::new(
                gio::IOErrorEnum::InvalidData,
                &t("配置必须是小于 1 MiB 的普通文件。"),
            ));
        }
        let contents = std::fs::read(path)
            .map_err(|e| glib::Error::new(glib::FileError::Io, &format!("{}", e)))?;
        let file = KeyFile::new();
        let text = std::str::from_utf8(&contents).map_err(|_| {
            glib::Error::new(
                gio::IOErrorEnum::InvalidData,
                &t("所选文件不是有效的 Zero Dock 配置。"),
            )
        })?;
        let valid = file
            .load_from_data(text, glib::KeyFileFlags::KEEP_COMMENTS)
            .is_ok();

        // Validate pins and association rules before touching anything.
        let mut pins: Vec<String> = Vec::new();
        let mut valid = valid
            && file.has_group("Dock")
            && match file.string_list("Dock", "Pinned") {
                Ok(list) if list.len() <= 512 => {
                    pins = list.iter().map(|s| s.to_string()).collect();
                    true
                }
                Ok(_) => false,
                Err(_) => !file.has_key("Dock", "Pinned").unwrap_or(false),
            };
        if valid {
            for pin in &pins {
                valid = is_absolute_desktop_path(pin);
                if !valid {
                    break;
                }
            }
        }
        if valid {
            if let Ok(keys) = file.keys("Associations") {
                valid = keys.len() <= 512;
                for key in &keys {
                    if !valid {
                        break;
                    }
                    let value = file.string("Associations", key).ok();
                    valid &= valid_key(key) && value.is_some();
                    if valid {
                        let value = value.unwrap().to_string();
                        valid = value.starts_with('/') && value.ends_with(".desktop");
                    }
                    if !valid {
                        break;
                    }
                }
                // A missing Associations group is fine.
            }
        }
        if !valid {
            return Err(glib::Error::new(
                gio::IOErrorEnum::InvalidData,
                &t("所选文件不是有效的 Zero Dock 配置。"),
            ));
        }

        let mut settings = Settings::default();
        settings.load(&file);
        settings.save(&file);
        let mut seen = std::collections::HashSet::new();
        pins.retain(|pin| seen.insert(pin.clone()));
        write_pins_to_keyfile(&file, &pins);
        let normalized = file.to_data();
        let backup = format!("{}.backup-{}", self.rc_path(), real_time_us());
        self.config_export(&backup)?;
        write_private_file(&self.rc_path(), normalized.as_bytes())
            .map_err(|e| glib::Error::new(glib::FileError::Io, &format!("{}", e)))?;

        self.hide_preview();
        if let Some(menu) = self.menu.take() {
            ffi_gtk::destroy(&menu);
        }
        // Running pinned buttons become ordinary live buttons temporarily.
        let mut i = 0;
        while i < self.buttons.len() {
            self.buttons[i].thumbnail = None;
            let b = &self.buttons[i];
            if b.pinned {
                if let Some(window) = b.window.clone() {
                    let button = &mut self.buttons[i];
                    button.pinned = false;
                    button.desktop = None;
                    button.key = format!("window:{}", crate::ffi_xfce::Window::new(&window).xid());
                }
            }
            i += 1;
        }
        self.buttons.retain_mut(|b| {
            if b.pinned && b.window.is_none() {
                b.destroy_widgets();
                false
            } else {
                true
            }
        });

        self.settings = settings;
        self.load_associations(&file);
        self.app_generation += 1;
        for pin in &pins {
            self.add_pin(pin);
        }
        self.refresh();
        self.refresh_launcher_manager();
        Ok(())
    }
}

pub fn is_absolute_desktop_path(path: &str) -> bool {
    path.starts_with('/') && path.ends_with(".desktop")
}

pub fn valid_key(key: &str) -> bool {
    key.len() == 64 && key.chars().all(|c| c.is_ascii_hexdigit())
}

/// Serialize the pinned-launcher list into a key file (shared by `Dock::save`
/// and the unit tests so the real serialization is what gets tested).
pub fn write_pins_to_keyfile(file: &KeyFile, pins: &[String]) {
    ffi_gtk::key_file_set_string_list(file, "Dock", "Pinned", pins);
}

/// Write `contents` atomically with 0600 permissions (replacement for
/// `g_file_set_contents_full(..., CONSISTENT, 0600, ...)`).
pub fn write_private_file(path: &str, contents: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let tmp = format!("{}.zd-tmp-{}", path, std::process::id());
    let open = || {
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)
    };
    // A crash between create and rename leaves the temp file behind; with a
    // recycled pid it would then block every future save, so drop it and
    // retry once.
    let mut file = match open() {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            let _ = std::fs::remove_file(&tmp);
            open()?
        }
        Err(e) => return Err(e),
    };
    file.write_all(contents)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&tmp, path)
}

// ---------------------------------------------------------------------------
// Settings dialog
// ---------------------------------------------------------------------------

/// Identifies a numeric setting bound to a spin button or combo box.
#[derive(Clone, Copy, PartialEq)]
enum IntField {
    PreviewWidth,
    PreviewDelay,
    PreviewInterval,
    LaunchTimeout,
    LeftAction,
    MiddleAction,
}

impl IntField {
    fn get(self, s: &Settings) -> u32 {
        match self {
            IntField::PreviewWidth => s.preview_width,
            IntField::PreviewDelay => s.preview_delay,
            IntField::PreviewInterval => s.preview_interval,
            IntField::LaunchTimeout => s.launch_timeout,
            IntField::LeftAction => s.left_action,
            IntField::MiddleAction => s.middle_action,
        }
    }
    fn set(self, s: &mut Settings, value: u32) {
        match self {
            IntField::PreviewWidth => s.preview_width = value,
            IntField::PreviewDelay => s.preview_delay = value,
            IntField::PreviewInterval => s.preview_interval = value,
            IntField::LaunchTimeout => s.launch_timeout = value,
            IntField::LeftAction => s.left_action = value,
            IntField::MiddleAction => s.middle_action = value,
        }
    }
}

impl Dock {
    pub fn configure(&mut self) {
        if let Some(dialog) = self.settings_dialog.as_ref() {
            dialog.present();
            return;
        }
        let dialog = gtk::Dialog::with_buttons(
            Some(t("Zero Dock 设置").as_str()),
            None::<&gtk::Window>,
            gtk::DialogFlags::DESTROY_WITH_PARENT,
            &[(t("关闭").as_str(), gtk::ResponseType::Close)],
        );
        dialog.set_skip_taskbar_hint(true);
        if let Some(plugin) = &self.plugin {
            plugin.take_window(&dialog);
        }
        let content = dialog.content_area();
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .build();
        let box_ = gtk::Box::new(gtk::Orientation::Vertical, 12);
        box_.set_border_width(16);
        scroller.set_child(Some(&box_));
        content.pack_start(&scroller, true, true, 0);

        let area = self.monitor_area();
        dialog.set_default_size(
            560.min((area.width() - 48).max(200)),
            600.min((area.height() - 48).max(200)),
        );

        let status = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        status.set_no_show_all(true);
        let status_label = gtk::Label::new(None);
        status_label.set_line_wrap(true);
        status_label.set_line_wrap_mode(pango::WrapMode::WordChar);
        status_label.set_max_width_chars(60);
        status_label.set_xalign(0.0);
        status_label.set_hexpand(true);
        let retry = gtk::Button::with_label(&t("重试保存"));
        let weak = self.weak();
        retry.connect_clicked(move |_| {
            if let Some(rc) = weak.upgrade() {
                crate::util::with_dock(&rc, |d| {
                    d.save();
                });
            }
        });
        status.pack_start(&status_label, true, true, 0);
        status.pack_end(&retry, false, false, 0);
        status_label.show();
        retry.show();
        box_.pack_start(&status, false, false, 0);
        self.save_status = Some(status);
        self.save_status_label = Some(status_label);
        self.update_save_status();

        // Boolean toggles
        let bools = [
            (t("显示可点击的窗口预览"), BoolField::Previews),
            (t("同应用多窗口显示序号"), BoolField::Numbers),
            (t("显示所有工作区的窗口"), BoolField::AllWorkspaces),
            (t("滚轮切换窗口"), BoolField::ScrollWindows),
        ];
        for (label, field) in bools {
            let check = gtk::CheckButton::with_label(&label);
            check.set_active(field.get(&self.settings));
            let weak = self.weak();
            check.connect_toggled(move |btn| {
                let value = btn.is_active();
                if let Some(rc) = weak.upgrade() {
                    crate::util::with_dock(&rc, |d| {
                        field.set(&mut d.settings, value);
                        d.hide_preview();
                        d.save();
                        d.refresh();
                    });
                }
            });
            box_.pack_start(&check, false, false, 0);
        }

        // Numeric rows: (label, field, low, high, step)
        let numbers = [
            (
                t("预览宽度（像素）"),
                IntField::PreviewWidth,
                180,
                600,
                50.0,
            ),
            (
                t("悬停延时（毫秒）"),
                IntField::PreviewDelay,
                100,
                2000,
                50.0,
            ),
            (
                t("预览刷新间隔（毫秒）"),
                IntField::PreviewInterval,
                200,
                2000,
                50.0,
            ),
            (
                t("启动等待时间（毫秒）"),
                IntField::LaunchTimeout,
                2000,
                60000,
                50.0,
            ),
        ];
        for (label, field, low, high, step) in numbers {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            let lbl = gtk::Label::new(Some(&label));
            row.pack_start(&lbl, true, true, 0);
            let spin = gtk::SpinButton::with_range(low as f64, high as f64, step);
            spin.set_value(field.get(&self.settings) as f64);
            let weak = self.weak();
            spin.connect_value_changed(move |sp| {
                let value = sp.value_as_int() as u32;
                if let Some(rc) = weak.upgrade() {
                    crate::util::with_dock(&rc, |d| {
                        field.set(&mut d.settings, value);
                        d.hide_preview();
                        if field == IntField::PreviewWidth {
                            for b in &mut d.buttons {
                                b.thumbnail = None;
                            }
                        }
                        d.save();
                        d.refresh();
                    });
                }
            });
            row.pack_end(&spin, false, false, 0);
            box_.pack_start(&row, false, false, 0);
        }

        // Click behavior combos
        let combos = [
            (
                t("左键行为"),
                IntField::LeftAction,
                vec![t("激活 / 最小化"), t("仅激活")],
            ),
            (
                t("中键行为"),
                IntField::MiddleAction,
                vec![t("启动新窗口"), t("关闭窗口"), t("不操作")],
            ),
        ];
        for (label, field, options) in combos {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            let lbl = gtk::Label::new(Some(&label));
            row.pack_start(&lbl, true, true, 0);
            let combo = gtk::ComboBoxText::new();
            for opt in &options {
                combo.append_text(opt);
            }
            combo.set_active(Some(field.get(&self.settings)));
            let weak = self.weak();
            combo.connect_changed(move |c| {
                let value = c.active();
                if let (Some(rc), Some(value)) = (weak.upgrade(), value) {
                    crate::util::with_dock(&rc, |d| {
                        field.set(&mut d.settings, value);
                        d.save();
                    });
                }
            });
            row.pack_end(&combo, false, false, 0);
            box_.pack_start(&row, false, false, 0);
        }

        // Backup / restore
        let backups = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let manage = gtk::Button::with_label(&t("管理固定应用与关联…"));
        let weak = self.weak();
        manage.connect_clicked(move |_| {
            if let Some(rc) = weak.upgrade() {
                crate::util::with_dock(&rc, |d| d.manage_launchers());
            }
        });
        box_.pack_start(&manage, false, false, 0);
        for (i, label) in [t("备份配置…"), t("恢复配置…")].into_iter().enumerate() {
            let button = gtk::Button::with_label(&label);
            let restore = i == 1;
            let weak = self.weak();
            button.connect_clicked(move |_| {
                if let Some(rc) = weak.upgrade() {
                    crate::util::with_dock(&rc, |d| d.config_chooser(restore));
                }
            });
            backups.pack_start(&button, false, false, 0);
        }
        box_.pack_start(&backups, false, false, 0);

        // Actions row
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let reset = gtk::Button::with_label(&t("恢复默认设置（保留固定应用）"));
        let export = gtk::Button::with_label(&t("导出诊断信息"));
        {
            let weak = self.weak();
            reset.connect_clicked(move |_| {
                if let Some(rc) = weak.upgrade() {
                    crate::util::with_dock(&rc, |d| d.reset_settings());
                }
            });
        }
        {
            let weak = self.weak();
            export.connect_clicked(move |_| {
                if let Some(rc) = weak.upgrade() {
                    crate::util::with_dock(&rc, |d| d.export_diagnostics());
                }
            });
        }
        actions.pack_start(&reset, false, false, 0);
        actions.pack_start(&export, false, false, 0);
        box_.pack_start(&actions, false, false, 0);

        let info = gtk::Label::new(Some(&t(
            "固定图标复用首个窗口，其他窗口独立显示。\n长度随内容和面板空间自动调整，空间不足时显示更多菜单。\n拖入 .desktop 文件或选择应用即可固定。\n图标和预览共用中键设置。\n喇叭滚轮调整应用音量，最高 200%。\n预览画面仅保存在内存中。",
        )));
        info.set_xalign(0.0);
        box_.pack_start(&info, false, false, 4);

        {
            let weak = self.weak();
            dialog.connect_response(move |dlg, _| {
                ffi_gtk::destroy(dlg);
                let _ = &weak;
            });
        }
        {
            let weak = self.weak();
            dialog.connect_destroy(move |_| {
                if let Some(rc) = weak.upgrade() {
                    crate::util::with_dock(&rc, |d| {
                        d.settings_dialog = None;
                        d.close_launcher_dialogs();
                        d.save_status = None;
                        d.save_status_label = None;
                    });
                }
            });
        }
        dialog.show_all();
        self.settings_dialog = Some(dialog);
    }

    pub(crate) fn monitor_area(&self) -> gdk::Rectangle {
        let Some(display) = gdk::Display::default() else {
            return gdk::Rectangle::new(0, 0, 1024, 768);
        };
        let plugin_widget: Option<gtk::Widget> = self
            .plugin
            .as_ref()
            .and_then(|p| p.as_object().downcast::<gtk::Widget>().ok());
        let monitor = plugin_widget.and_then(|w| {
            if !w.is_realized() {
                return None;
            }
            w.window().and_then(|win| display.monitor_at_window(&win))
        });
        let monitor = monitor.or_else(|| display.primary_monitor());
        let mut area = gdk::Rectangle::new(0, 0, 1024, 768);
        if let Some(monitor) = monitor {
            area = monitor.workarea();
        }
        area
    }

    fn reset_settings(&mut self) {
        self.close_launcher_dialogs();
        self.settings = Settings::default();
        self.hide_preview();
        for b in &mut self.buttons {
            b.thumbnail = None;
        }
        self.save();
        self.refresh();
        if let Some(dialog) = self.settings_dialog.take() {
            ffi_gtk::destroy(&dialog);
        }
        self.configure();
    }

    fn export_diagnostics(&mut self) {
        let settings = match self.settings_dialog.as_ref() {
            Some(s) => s.clone().upcast::<gtk::Window>(),
            None => return,
        };
        let dialog = gtk::FileChooserDialog::with_buttons(
            Some(t("导出诊断信息").as_str()),
            Some(&settings),
            gtk::FileChooserAction::Save,
            &[
                (t("取消").as_str(), gtk::ResponseType::Cancel),
                (t("保存").as_str(), gtk::ResponseType::Accept),
            ],
        );
        dialog.set_current_name("zero-dock-diagnostics.txt");
        dialog.set_do_overwrite_confirmation(true);
        dialog.set_destroy_with_parent(true);
        dialog.set_skip_taskbar_hint(true);
        if let Some(plugin) = &self.plugin {
            plugin.take_window(&dialog);
        }
        let weak = self.weak();
        dialog.connect_response(move |dlg, response| {
            if response == gtk::ResponseType::Accept {
                if let Some(path) = dlg.filename() {
                    if let Some(rc) = weak.upgrade() {
                        crate::util::with_dock(&rc, |d| {
                            let text = d.diagnostics();
                            if let Err(e) =
                                write_private_file(&path.to_string_lossy(), text.as_bytes())
                            {
                                d.show_error(&format!("{}", e));
                            }
                        });
                    }
                }
            }
            ffi_gtk::destroy(dlg);
        });
        dialog.show();
    }

    fn config_chooser(&mut self, restore: bool) {
        let settings = match self.settings_dialog.as_ref() {
            Some(s) => s.clone().upcast::<gtk::Window>(),
            None => return,
        };
        let (title, action, accept) = if restore {
            (
                t("恢复 Zero Dock 配置"),
                gtk::FileChooserAction::Open,
                t("恢复"),
            )
        } else {
            (
                t("备份 Zero Dock 配置"),
                gtk::FileChooserAction::Save,
                t("保存"),
            )
        };
        let dialog = gtk::FileChooserDialog::with_buttons(
            Some(&title),
            Some(&settings),
            action,
            &[
                (&t("取消"), gtk::ResponseType::Cancel),
                (&accept, gtk::ResponseType::Accept),
            ],
        );
        if !restore {
            dialog.set_current_name("zero-dock-backup.rc");
            dialog.set_do_overwrite_confirmation(true);
        }
        let description = gtk::Label::new(Some(&if restore {
            t("恢复固定应用、手动关联与设置；恢复前自动备份当前配置。")
        } else {
            t("备份包含固定启动器和手动关联的本机文件路径。")
        }));
        dialog.set_extra_widget(&description);
        description.show();
        dialog.set_destroy_with_parent(true);
        dialog.set_skip_taskbar_hint(true);
        if let Some(plugin) = &self.plugin {
            plugin.take_window(&dialog);
        }
        let weak = self.weak();
        dialog.connect_response(move |dlg, response| {
            let mut restored = false;
            if response == gtk::ResponseType::Accept {
                if let Some(path) = dlg.filename() {
                    let path = path.to_string_lossy().into_owned();
                    if let Some(rc) = weak.upgrade() {
                        crate::util::with_dock(&rc, |d| {
                            let result = if restore {
                                d.config_import(&path)
                            } else {
                                d.config_export(&path)
                            };
                            restored = restore && result.is_ok();
                            if let Err(e) = result {
                                d.show_error(e.message());
                            }
                        });
                    }
                }
            }
            ffi_gtk::destroy(dlg);
            if restored {
                if let Some(rc) = weak.upgrade() {
                    crate::util::with_dock(&rc, |d| {
                        d.close_launcher_dialogs();
                        if let Some(s) = d.settings_dialog.take() {
                            ffi_gtk::destroy(&s);
                        }
                        d.configure();
                    });
                }
            }
        });
        dialog.show();
    }
}

#[derive(Clone, Copy)]
enum BoolField {
    Previews,
    Numbers,
    AllWorkspaces,
    ScrollWindows,
}

impl BoolField {
    fn get(self, s: &Settings) -> bool {
        match self {
            BoolField::Previews => s.previews,
            BoolField::Numbers => s.numbers,
            BoolField::AllWorkspaces => s.all_workspaces,
            BoolField::ScrollWindows => s.scroll_windows,
        }
    }
    fn set(self, s: &mut Settings, value: bool) {
        match self {
            BoolField::Previews => s.previews = value,
            BoolField::Numbers => s.numbers = value,
            BoolField::AllWorkspaces => s.all_workspaces = value,
            BoolField::ScrollWindows => s.scroll_windows = value,
        }
    }
}
