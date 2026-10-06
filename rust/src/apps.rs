//! Desktop identity, application matching, launch feedback and the
//! application index. Ported from `src/apps.c`.

use crate::dock::Dock;
use crate::util::{monotonic_us, t};
use gio::prelude::*;
use gtk::prelude::*;

/// `g_app_info_get_executable`, NULL mapped to an empty string.
pub fn app_executable(app: &gio::DesktopAppInfo) -> String {
    crate::ffi_gtk::app_info_executable(&app.clone().upcast::<gio::AppInfo>())
}

pub fn app_match_score(app: &gio::DesktopAppInfo, ids: &[String]) -> i32 {
    let wm = app.startup_wm_class();
    let id = app.id(); // g_app_info_get_id
    let exe = app_executable(app);
    let base = exe
        .rsplit('/')
        .next()
        .map(|s| s.to_string())
        .unwrap_or_default();
    let mut high = 0;
    for class in ids {
        let score = if wm.as_ref().is_some_and(|wm| wm.eq_ignore_ascii_case(class)) {
            100
        } else {
            continue_score(app, id.as_deref(), &base, class)
        };
        high = high.max(score);
    }
    high
}

fn continue_score(_app: &gio::DesktopAppInfo, id: Option<&str>, base: &str, class: &str) -> i32 {
    if !base.is_empty() && base.eq_ignore_ascii_case(class) {
        80
    } else if let Some(id) = id {
        if fuzzy_match(id, class) {
            40
        } else {
            0
        }
    } else {
        0
    }
}

pub fn app_equal(a: Option<&gio::DesktopAppInfo>, b: Option<&gio::DesktopAppInfo>) -> bool {
    let (Some(a), Some(b)) = (a, b) else {
        return false;
    };
    match (a.filename(), b.filename()) {
        (Some(pa), Some(pb)) => pa == pb,
        // g_app_info_equal for entries without a file (id + executable).
        _ => {
            let a = a.clone().upcast::<gio::AppInfo>();
            let b = b.clone().upcast::<gio::AppInfo>();
            a.equal(&b)
        }
    }
}

/// Identity bonus for a window that appeared while a pin was launching.
///
/// A matching startup id is the strongest signal, but it is only *a* signal:
/// when both ids are present and differ (a stale id on the window, or a
/// wrapper that never received ours), the process-ancestry probe must still
/// run. Otherwise the launcher silently loses the very window it started.
pub(crate) fn launch_probe_score(
    window_startup: Option<&str>,
    launcher_startup: Option<&str>,
    window_pid: i32,
    launcher_pid: i32,
) -> i32 {
    if let (Some(window_id), Some(launcher_id)) = (window_startup, launcher_startup) {
        if !window_id.is_empty() && window_id == launcher_id {
            return 220;
        }
    }
    if launcher_pid > 1 && pid_descends(window_pid, launcher_pid) {
        return 200;
    }
    0
}

/// Exact identity hints: full path, desktop id, basename or flatpak id.
pub(crate) fn exact_id(app: &gio::DesktopAppInfo, hint: Option<&str>) -> bool {
    let Some(hint) = hint else { return false };
    if hint.is_empty() {
        return false;
    }
    let path = app.filename().map(|p| p.to_string_lossy().into_owned());
    let id = app.id(); // g_app_info_get_id
    let base = path.as_ref().and_then(|p| {
        std::path::Path::new(p)
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
    });
    let desktop = if hint.ends_with(".desktop") {
        hint.to_string()
    } else {
        format!("{}.desktop", hint)
    };
    let flatpak = app.string("X-Flatpak").map(|s| s.to_string());
    path.as_deref() == Some(hint)
        || id.as_deref() == Some(desktop.as_str())
        || base.as_deref() == Some(desktop.as_str())
        || flatpak.as_deref() == Some(hint)
}

impl Dock {
    pub fn match_app(&self, window: &glib::Object) -> Option<gio::DesktopAppInfo> {
        // Manual associations take precedence over automatic identification.
        if let Some(rule) = crate::associations::association_key(window) {
            if let Some(path) = self.associations.get(&rule) {
                if let Some(app) = gio::DesktopAppInfo::from_filename(path) {
                    return Some(app);
                }
            }
        }
        let w = crate::ffi_xfce::Window::new(window);
        let ids = w.class_ids();
        let gtk_id = crate::ffi_x11::get_string_property(w.xid(), "_GTK_APPLICATION_ID");
        let desktop_id = crate::ffi_x11::get_string_property(w.xid(), "_KDE_NET_WM_DESKTOP_FILE");
        let startup = crate::ffi_x11::get_string_property(w.xid(), "_NET_STARTUP_ID");
        let pid = window_pid(window);
        let now = monotonic_us();

        // Imported pinned entries outside XDG directories participate too; a
        // pin wins ties but a stronger installed StartupWMClass match wins.
        let mut best: Option<gio::DesktopAppInfo> = None;
        let mut high = 0;
        for b in &self.buttons {
            let Some(app) = b.app.as_ref() else { continue };
            if !b.pinned {
                continue;
            }
            let mut score = app_match_score(app, &ids);
            if exact_id(app, gtk_id.as_deref()) || exact_id(app, desktop_id.as_deref()) {
                score = score.max(160);
            }
            if b.launch_until > now {
                score = score.max(launch_probe_score(
                    startup.as_deref(),
                    b.startup_id.as_deref(),
                    pid,
                    b.launch_pid,
                ));
            }
            if score > high {
                high = score;
                best = Some(app.clone());
            }
        }
        for app in &self.apps {
            let Some(app) = app.downcast_ref::<gio::DesktopAppInfo>() else {
                continue;
            };
            let mut score = app_match_score(app, &ids);
            if exact_id(app, gtk_id.as_deref()) || exact_id(app, desktop_id.as_deref()) {
                score = score.max(160);
            }
            if score > high {
                high = score;
                best = Some(app.clone());
            }
        }
        best
    }

    /// Rebuild the installed application index and refresh pins.
    pub fn reload_apps(&mut self) {
        self.apps = gio::AppInfo::all();
        self.app_generation += 1;
        for b in &mut self.buttons {
            if b.pinned {
                if let Some(desktop) = b.desktop.clone() {
                    b.app = gio::DesktopAppInfo::from_filename(&desktop);
                }
                b.icon_dirty = true;
            }
        }
        self.queue_refresh();
    }
}

// ---------------------------------------------------------------------------
// Launch
// ---------------------------------------------------------------------------

impl Dock {
    pub fn launch(&mut self, button_id: u64) {
        if let Some(b) = self.button_mut(button_id) {
            b.launch_error = None;
        }

        let (pinned, desktop) = self
            .button(button_id)
            .map(|b| (b.pinned, b.desktop.clone()))
            .unwrap_or((false, None));
        // Re-read idle pins so moved/deleted files fail visibly.
        if pinned {
            if let Some(desktop) = desktop {
                if let Some(b) = self.button_mut(button_id) {
                    b.app = gio::DesktopAppInfo::from_filename(&desktop);
                    b.icon_dirty = true;
                }
            }
        }
        let has_app = self.button(button_id).is_some_and(|b| b.app.is_some());
        if !has_app {
            let error = t("启动器文件已失效，请重新拖入该应用的 .desktop 文件。").to_string();
            let message = error.clone();
            if let Some(b) = self.button_mut(button_id) {
                b.launching = false;
                b.launch_until = 0;
                b.launch_error = Some(error);
            }
            self.show_error(&message);
            self.update_buttons();
            return;
        }

        {
            let timeout = self.settings.launch_timeout as i64;
            let Some(b) = self.button_mut(button_id) else {
                return;
            };
            b.launching = true;
            b.launch_until = monotonic_us() + timeout * 1000;
            b.launch_pid = 0;
            b.startup_id = None;
        }

        let timestamp = self.timestamp();
        let context = match gdk::Display::default().and_then(|d| d.app_launch_context()) {
            Some(ctx) => ctx,
            None => return,
        };
        context.set_timestamp(timestamp);

        // "launched" delivers the startup-notification id via platform data.
        let weak = self.weak();
        context.connect_launched(move |_ctx, _app, platform| {
            let dict = glib::VariantDict::new(Some(platform));
            let id = dict
                .lookup::<String>("startup-notification-id")
                .ok()
                .flatten();
            if let Some(id) = id {
                if let Some(rc) = weak.upgrade() {
                    crate::util::with_dock(&rc, |d| {
                        if let Some(b) = d.button_mut(button_id) {
                            b.startup_id = Some(id);
                        }
                    });
                }
            }
        });

        let Some(app) = self.button(button_id).and_then(|b| b.app.clone()) else {
            return;
        };
        let weak_pid = self.weak();
        let mut closed = (-1i32, -1i32, -1i32);
        let result = app.launch_uris_as_manager_with_fds(
            &[],
            Some(&context),
            glib::SpawnFlags::SEARCH_PATH,
            None,
            Some(&mut |_app, pid| {
                if let Some(rc) = weak_pid.upgrade() {
                    crate::util::with_dock(&rc, |d| {
                        if let Some(b) = d.button_mut(button_id) {
                            b.launch_pid = pid.0;
                        }
                    });
                }
            }),
            &mut closed.0,
            &mut closed.1,
            &mut closed.2,
        );

        match result {
            Err(error) => {
                let message = t("无法启动应用：%s")
                    .replace("%s", error.message())
                    .to_string();
                if let Some(b) = self.button_mut(button_id) {
                    b.launching = false;
                    b.launch_until = 0;
                    b.launch_error = Some(message.clone());
                }
                self.show_error(&message);
            }
            Ok(()) => {
                if !self.launch_tick.is_set() {
                    let weak = self.weak();
                    self.launch_tick
                        .set_timeout(100, move || match weak.upgrade() {
                            None => glib::ControlFlow::Break,
                            Some(rc) => match crate::util::with_dock(&rc, |d| d.launch_tick_step())
                            {
                                Some(flow) => flow,
                                None => glib::ControlFlow::Break,
                            },
                        });
                }
            }
        }
        self.update_buttons();
    }

    fn launch_tick_step(&mut self) -> glib::ControlFlow {
        let mut pending = false;
        let now = monotonic_us();
        let expired: Vec<u64> = self
            .buttons
            .iter()
            .filter(|b| b.launching && now >= b.launch_until)
            .map(|b| b.id)
            .collect();
        for id in expired {
            if let Some(b) = self.button_mut(id) {
                b.launching = false;
                b.launch_error =
                    Some(t("启动请求已发送，但未检测到新窗口。可再次点击重试。").to_string());
            }
            self.update_buttons();
        }
        for b in &self.buttons {
            if b.launching {
                pending = true;
                b.drawing.queue_draw();
            }
        }
        if !pending {
            self.launch_tick.take();
            return glib::ControlFlow::Break;
        }
        glib::ControlFlow::Continue
    }

    pub fn launch_complete(&mut self, button_id: u64) {
        if let Some(b) = self.button_mut(button_id) {
            b.launching = false;
            b.launch_error = None;
            b.drawing.queue_draw();
        }
    }
}

/// Window process id: application instance first, then `_NET_WM_PID`
/// (port of `zd_window_pid`).
pub fn window_pid(window: &glib::Object) -> i32 {
    let w = crate::ffi_xfce::Window::new(window);
    let pid = crate::ffi_xfce::application_pid(w.application().as_ref(), window);
    if pid > 0 {
        return pid;
    }
    let atom = crate::ffi_x11::intern_atom("_NET_WM_PID");
    match crate::ffi_x11::get_cardinal_property(w.xid(), atom) {
        Some(value) if value <= i32::MAX as u64 => value as i32,
        _ => 0,
    }
}

// ---------------------------------------------------------------------------
// Pure helpers (unit tested)
// ---------------------------------------------------------------------------

/// `normalize()` from audio.c: basename, lowercased, .desktop/.exe suffixes
/// stripped. `None` for empty input.
pub fn normalize(s: &str) -> Option<String> {
    if s.is_empty() {
        return None;
    }
    let base = std::path::Path::new(s)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| s.to_string());
    let mut n = base.to_lowercase();
    if n.ends_with(".desktop") {
        n.truncate(n.len() - 8);
    }
    if n.ends_with(".exe") {
        n.truncate(n.len() - 4);
    }
    Some(n)
}

pub fn fuzzy_match(a: &str, b: &str) -> bool {
    let (Some(x), Some(y)) = (normalize(a), normalize(b)) else {
        return false;
    };
    if x.is_empty() || y.is_empty() {
        return false;
    }
    if x == y {
        return true;
    }
    if x.len().min(y.len()) >= 4 && (x.contains(&y) || y.contains(&x)) {
        return true;
    }
    false
}

/// Walk /proc to check process ancestry, bounded like the C implementation.
pub fn pid_descends(mut pid: i32, ancestor: i32) -> bool {
    if pid <= 0 || ancestor <= 0 {
        return false;
    }
    for _ in 0..96 {
        if pid <= 1 {
            break;
        }
        if pid == ancestor {
            return true;
        }
        let data = match std::fs::read_to_string(format!("/proc/{}/stat", pid)) {
            Ok(d) => d,
            Err(_) => return false,
        };
        let Some(end) = data.rfind(')') else {
            return false;
        };
        let rest = &data[end + 1..];
        let mut parts = rest.split_whitespace();
        let _state = parts.next();
        let Some(parent) = parts.next().and_then(|p| p.parse::<i64>().ok()) else {
            return false;
        };
        if parent <= 0 || parent == pid as i64 {
            return false;
        }
        pid = parent as i32;
    }
    pid == ancestor
}
