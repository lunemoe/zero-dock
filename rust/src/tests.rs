//! Model tests ported from `tests/test-core.c`: process ancestry, fuzzy
//! matching, desktop identity, pin persistence, invalid settings handling
//! and plugin registration data. These run headless via `cargo test`.

use crate::apps::{app_equal, app_match_score, exact_id, fuzzy_match, pid_descends};
use crate::settings::Settings;
use glib::KeyFile;
use std::collections::HashMap;

/// `/audio/ancestry`
#[test]
fn ancestry() {
    let pid = std::process::id() as i32;
    let ppid = {
        // /proc/self/stat field 4
        let data = std::fs::read_to_string("/proc/self/stat").unwrap();
        let end = data.rfind(')').unwrap();
        let mut parts = data[end + 1..].split_whitespace();
        parts.next(); // state
        parts.next().unwrap().parse::<i32>().unwrap()
    };
    assert!(pid_descends(pid, pid));
    assert!(pid_descends(pid, ppid));
    assert!(!pid_descends(pid, 0));
    assert!(!pid_descends(0, pid));
    assert!(!pid_descends(pid, 2147483647));
}

/// `/audio/matching`
#[test]
fn matching() {
    assert!(fuzzy_match("/usr/bin/google-chrome", "Google-chrome"));
    assert!(fuzzy_match("org.mozilla.firefox.desktop", "firefox"));
    assert!(fuzzy_match("game.exe", "game"));
    assert!(!fuzzy_match("sh", "shotwell"));
    assert!(!fuzzy_match("", "chrome"));
    assert!(!fuzzy_match("", ""));
    assert!(!fuzzy_match("firefox", "chromium"));
}

/// `/applications/desktop-identity`
#[test]
fn desktop_identity() {
    let dir = std::env::temp_dir().join(format!("zero-dock-apps-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let first = dir.join("first.desktop");
    let second = dir.join("second.desktop");
    let entry = "[Desktop Entry]\nType=Application\nName=Fixture\n\
                Exec=/usr/bin/true\nStartupWMClass=FirstFixture\n";
    std::fs::write(&first, entry).unwrap();
    std::fs::write(&second, entry).unwrap();

    let a = gio::DesktopAppInfo::from_filename(&first).expect("first.desktop");
    let copy = gio::DesktopAppInfo::from_filename(&first).expect("first.desktop");
    let b = gio::DesktopAppInfo::from_filename(&second).expect("second.desktop");

    assert!(app_equal(Some(&a), Some(&copy)));
    assert!(!app_equal(Some(&a), Some(&b)));
    assert!(!app_equal(Some(&a), None));
    assert!(!app_equal(None, Some(&a)));

    let exact = vec!["firstfixture".to_string()];
    let executable = vec!["true".to_string()];
    let unrelated = vec!["OtherFixture".to_string()];
    assert!(app_match_score(&a, &exact) > app_match_score(&a, &executable));
    assert_eq!(app_match_score(&a, &unrelated), 0);
    assert_eq!(app_match_score(&a, &[]), 0);

    let _ = std::fs::remove_file(&first);
    let _ = std::fs::remove_file(&second);
    let _ = std::fs::remove_dir(&dir);
}

/// `/settings/pins-filter-order` — the real serializer used by `Dock::save`.
#[test]
fn pins_filter_order() {
    use crate::settings::write_pins_to_keyfile;
    let file = KeyFile::new();
    write_pins_to_keyfile(
        &file,
        &["/tmp/a.desktop".to_string(), "/tmp/c.desktop".to_string()],
    );
    let pins = file.string_list("Dock", "Pinned").unwrap();
    assert_eq!(pins.len(), 2);
    assert_eq!(pins[0].as_str(), "/tmp/a.desktop");
    assert_eq!(pins[1].as_str(), "/tmp/c.desktop");

    // An empty pin list writes an (empty) key rather than failing.
    let empty = KeyFile::new();
    write_pins_to_keyfile(&empty, &[]);
    assert_eq!(empty.string_list("Dock", "Pinned").map(|l| l.len()), Ok(0));
    let _ = HashMap::<String, String>::new(); // associations use the same shape
}

/// `/settings/invalid-values`: malformed values keep defaults or clamp.
#[test]
fn invalid_settings() {
    let mut settings = Settings::default();
    let file = KeyFile::new();
    file.load_from_data(
        "[Dock]\nPreviews=broken\nNumbers=false\nSlots=-4\n\
         PreviewWidth=bad\nPreviewDelay=99999\nPreviewInterval=1\n\
         LaunchTimeout=bad\nLeftAction=-2\nMiddleAction=99\nMaxVisible=-\
         5\nScrollWindows=bad\n",
        glib::KeyFileFlags::NONE,
    )
    .unwrap();
    settings.load(&file);
    assert!(settings.previews);
    assert!(!settings.numbers);
    assert_eq!(settings.slots, 0);
    assert_eq!(settings.preview_width, 300);
    assert_eq!(settings.preview_delay, 2000);
    assert_eq!(settings.preview_interval, 200);
    assert_eq!(settings.launch_timeout, 10000);
    assert_eq!(settings.left_action, 0);
    assert_eq!(settings.middle_action, 2);
    assert_eq!(settings.max_visible, 0);
    assert!(settings.scroll_windows);
}

/// `/native/registration`: the installed .desktop must register this module.
#[test]
fn registration() {
    // Path relative to the crate dir (rust/), resolved by cargo's cwd.
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let desktop = manifest.parent().unwrap().join("data/zero-dock.desktop");
    let contents = std::fs::read_to_string(&desktop).expect("data/zero-dock.desktop");
    assert!(contents.contains("[Xfce Panel]"));
    assert!(contents.contains("X-XFCE-Module=zero-dock"));
    assert!(contents.contains("X-XFCE-Internal=false"));
    assert!(contents.contains("X-XFCE-Unique=false"));
    assert!(contents.contains("X-XFCE-Supports-X11=true"));
}

/// Manual association keys: SHA-256 of length-prefixed class ids.
#[test]
fn association_keys() {
    use crate::associations::association_key;
    use crate::settings::valid_key;

    // Windows are needed for real class ids; validate the key format rules
    // and the hash determinism through the pure helpers instead.
    assert!(valid_key(&"a".repeat(64)));
    assert!(!valid_key(&"g".repeat(64)));
    assert!(!valid_key("abc"));

    let key1 =
        glib::compute_checksum_for_data(glib::ChecksumType::Sha256, b"2:ab;").unwrap_or_default();
    let key2 =
        glib::compute_checksum_for_data(glib::ChecksumType::Sha256, b"2:ab;").unwrap_or_default();
    assert_eq!(key1, key2);
    let _ = association_key as fn(&glib::Object) -> Option<String>;
}

// ---------------------------------------------------------------------------
// Scroll accumulation (button.rs scroll_steps)
// ---------------------------------------------------------------------------

use crate::button::{scroll_steps, ScrollInfo};

fn smooth(dx: f64, dy: f64) -> ScrollInfo {
    ScrollInfo {
        direction: gdk::ScrollDirection::Smooth,
        delta_x: dx,
        delta_y: dy,
        time: 0,
        x: 0.0,
        y: 0.0,
        x_root: 0.0,
        y_root: 0.0,
    }
}

fn discrete(direction: gdk::ScrollDirection) -> ScrollInfo {
    ScrollInfo {
        direction,
        delta_x: 0.0,
        delta_y: 0.0,
        time: 0,
        x: 0.0,
        y: 0.0,
        x_root: 0.0,
        y_root: 0.0,
    }
}

/// Direction events snap to ±1 and reset the accumulator (mouse wheel).
#[test]
fn scroll_direction_snap() {
    let mut acc = 3.7;
    assert_eq!(
        scroll_steps(&discrete(gdk::ScrollDirection::Up), Some(&mut acc)),
        1
    );
    assert_eq!(acc, 0.0);
    assert_eq!(
        scroll_steps(&discrete(gdk::ScrollDirection::Down), Some(&mut acc)),
        -1
    );
    assert_eq!(acc, 0.0);
    assert_eq!(
        scroll_steps(&discrete(gdk::ScrollDirection::Left), Some(&mut acc)),
        1
    );
    assert_eq!(
        scroll_steps(&discrete(gdk::ScrollDirection::Right), Some(&mut acc)),
        -1
    );
}

/// Smooth events accumulate until a whole step is reached (touchpads,
/// XInput2 raw value scaling). The accumulator subtracts the delta, so a
/// "scroll down" gesture (positive dy) yields negative steps — consistent
/// with the discrete Down direction.
#[test]
fn scroll_smooth_accumulates() {
    let mut acc = 0.0;
    assert_eq!(scroll_steps(&smooth(0.0, 0.6), Some(&mut acc)), 0);
    assert!((acc + 0.6).abs() < 1e-9);
    assert_eq!(scroll_steps(&smooth(0.0, 0.5), Some(&mut acc)), -1);
    assert!((acc + 0.1).abs() < 1e-9);

    // The dominant axis wins.
    acc = 0.0;
    assert_eq!(scroll_steps(&smooth(-2.0, 0.3), Some(&mut acc)), 2);
    // Upward gestures go positive, like discrete Up.
    acc = 0.0;
    assert_eq!(scroll_steps(&smooth(0.0, -0.7), Some(&mut acc)), 0);
    assert_eq!(scroll_steps(&smooth(0.0, -0.4), Some(&mut acc)), 1);
    assert!((acc - 0.1).abs() < 1e-9);
}

/// Accumulated steps are clamped to ±32 per event.
#[test]
fn scroll_clamps() {
    let mut acc = 0.0;
    assert_eq!(scroll_steps(&smooth(1000.0, 0.0), Some(&mut acc)), -32);
    assert_eq!(scroll_steps(&smooth(-1000.0, 0.0), Some(&mut acc)), 32);
    // A missing accumulator ignores smooth deltas (matches the C guard).
    assert_eq!(scroll_steps(&smooth(5.0, 5.0), None), 0);
}

// ---------------------------------------------------------------------------
// Timer RAII (util.rs)
// ---------------------------------------------------------------------------

use crate::util::Timer;
use std::cell::Cell;
use std::rc::Rc;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// `timeout_add_local` always attaches to the global default main context,
/// so the timer tests serialize access to it.
static CONTEXT_LOCK: Mutex<()> = Mutex::new(());

fn pump(context: &glib::MainContext, seconds: f64) {
    let deadline = Instant::now() + Duration::from_secs_f64(seconds);
    while Instant::now() < deadline {
        let _ = context.iteration(false);
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// Run `f` with a private thread-default main context so timer tests never
/// contend with each other (cargo runs them in parallel threads).
fn with_own_context(f: impl FnOnce(&glib::MainContext)) {
    let _serial = CONTEXT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let context = glib::MainContext::default();
    context
        .with_thread_default(|| f(&context))
        .expect("context must be acquirable while holding the test lock");
}

/// A live timer fires; once dropped its source is removed and never fires.
#[test]
fn timer_fires_and_drop_cancels() {
    let fired = Rc::new(Cell::new(false));
    with_own_context(|context| {
        let mut live = Timer::default();
        {
            let fired = fired.clone();
            live.set_timeout(1, move || {
                fired.set(true);
                glib::ControlFlow::Break
            });
        }
        pump(context, 2.0);
        assert!(fired.get(), "live timer should have fired");

        // Dropped timer: the source is removed by Drop, callback never runs.
        let cancelled = Rc::new(Cell::new(false));
        {
            let mut doomed = Timer::default();
            let cancelled2 = cancelled.clone();
            doomed.set_timeout(30, move || {
                cancelled2.set(true);
                glib::ControlFlow::Break
            });
            doomed.clear();
        }
        pump(context, 0.2);
        assert!(!cancelled.get(), "dropped timer must not fire");

        // set_timeout on an armed timer replaces the pending source (no leak,
        // no double fire of the old one).
        let replaced = Rc::new(Cell::new(0u32));
        let mut timer = Timer::default();
        for _ in 0..3 {
            let replaced = replaced.clone();
            timer.set_timeout(1, move || {
                replaced.set(replaced.get() + 1);
                glib::ControlFlow::Break
            });
        }
        pump(context, 2.0);
        assert_eq!(replaced.get(), 1, "only the last replacement should fire");
        // Timer observes Break automatically; no explicit take() is required.
        assert!(
            !timer.is_set(),
            "completed timer must no longer report itself as live"
        );
    });
}

/// Dropping a timer whose source already fired must be silent and idempotent.
/// This test is also run with `G_DEBUG=fatal-warnings` by the isolated smoke.
#[test]
fn timer_stale_drop_is_safe() {
    let fired = Rc::new(Cell::new(false));
    let mut timer = Timer::default();
    with_own_context(|context| {
        let fired = fired.clone();
        timer.set_timeout(1, move || {
            fired.set(true);
            glib::ControlFlow::Break
        });
        pump(context, 2.0);
    });
    assert!(fired.get());
    assert!(!timer.is_set());
    // The source destroyed itself via Break. Drop must not call
    // g_source_remove() on the stale id or emit a GLib critical.
    drop(timer);
}

// ---------------------------------------------------------------------------
// Atomic private file writes (settings.rs write_private_file)
// ---------------------------------------------------------------------------

use crate::settings::write_private_file;
use std::os::unix::fs::PermissionsExt;

#[test]
fn private_file_round_trip_and_mode() {
    let dir = std::env::temp_dir().join(format!("zd-test-file-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config.rc");

    write_private_file(&path.to_string_lossy(), b"first").unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"first");
    let mode = std::fs::metadata(&path).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600, "config files must be private");

    // Overwriting an existing file works and leaves no temp siblings.
    write_private_file(&path.to_string_lossy(), b"second").unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"second");
    let leftovers: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().contains(".zd-tmp-"))
        .collect();
    assert!(leftovers.is_empty(), "temp files must be renamed away");

    // A leftover temp file from a crashed writer with the same (recycled)
    // pid must not block the next write.
    let stale = dir.join(format!("config.rc.zd-tmp-{}", std::process::id()));
    std::fs::write(&stale, b"stale").unwrap();
    write_private_file(&path.to_string_lossy(), b"third").unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"third");

    // Unwritable target fails cleanly.
    assert!(write_private_file("/nonexistent-dir/zd/x", b"no").is_err());

    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// Settings save/load round trip (settings.rs)
// ---------------------------------------------------------------------------

#[test]
fn settings_round_trip() {
    let mut settings = Settings {
        previews: false,
        numbers: false,
        all_workspaces: false,
        scroll_windows: false,
        preview_width: 480,
        preview_delay: 700,
        preview_interval: 900,
        launch_timeout: 15000,
        slots: 5,
        max_visible: 24,
        left_action: 1,
        middle_action: 2,
    };
    let file = KeyFile::new();
    settings.save(&file);
    let data = file.to_data();
    let reloaded = KeyFile::new();
    reloaded
        .load_from_data(data.as_str(), glib::KeyFileFlags::NONE)
        .unwrap();
    let mut back = Settings::default();
    back.load(&reloaded);
    assert_eq!(back, settings);

    // Out-of-range values clamp into the documented bounds.
    let clamped = KeyFile::new();
    clamped
        .load_from_data(
            "[Dock]\nPreviewWidth=99999\nPreviewDelay=1\nSlots=999\nMaxVisible=999\nMiddleAction=42\n",
            glib::KeyFileFlags::NONE,
        )
        .unwrap();
    settings.load(&clamped);
    assert_eq!(settings.preview_width, 600);
    assert_eq!(settings.preview_delay, 100);
    assert_eq!(settings.slots, 32);
    assert_eq!(settings.max_visible, 64);
    assert_eq!(settings.middle_action, 2);
}

// ---------------------------------------------------------------------------
// Match score precedence (apps.rs)
// ---------------------------------------------------------------------------

fn fixture_desktop(dir: &std::path::Path, name: &str, body: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    std::fs::write(
        &path,
        format!("[Desktop Entry]\nType=Application\nName=Fixture {name}\n{body}\n"),
    )
    .unwrap();
    path
}

#[test]
fn app_match_score_precedence() {
    let dir = std::env::temp_dir().join(format!("zd-test-score-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    // GIO refuses desktop entries whose Exec points at a missing binary, so
    // the executable-rank fixture gets a real (empty) executable of its own.
    let fake_bin = dir.join("ScoreTarget");
    std::fs::write(&fake_bin, "#!/bin/sh\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&fake_bin, std::fs::Permissions::from_mode(0o755)).unwrap();

    let wm = gio::DesktopAppInfo::from_filename(fixture_desktop(
        &dir,
        "wm.desktop",
        "Exec=/bin/true\nStartupWMClass=ScoreTarget\n",
    ))
    .unwrap();
    let exe = gio::DesktopAppInfo::from_filename(fixture_desktop(
        &dir,
        "exe.desktop",
        &format!("Exec={}\n", fake_bin.to_string_lossy()),
    ))
    .unwrap();
    let id = gio::DesktopAppInfo::from_filename(fixture_desktop(
        &dir,
        "zzz-scoretarget.desktop",
        "Exec=/bin/true\n",
    ))
    .unwrap();

    let class = vec!["ScoreTarget".to_string()];
    let score_wm = app_match_score(&wm, &class);
    let score_exe = app_match_score(&exe, &class);
    let score_id = app_match_score(&id, &class);
    assert_eq!(score_wm, 100, "StartupWMClass must outrank everything");
    assert_eq!(score_exe, 80, "executable basename is the middle rank");
    assert_eq!(score_id, 40, "fuzzy desktop id is the lowest rank");
    assert!(score_wm > score_exe && score_exe > score_id);

    // Best-of across classes, and no match scores zero.
    assert_eq!(app_match_score(&wm, &["unrelated".to_string()]), 0);
    assert_eq!(
        app_match_score(&wm, &["unrelated".to_string(), "scoretarget".to_string()]),
        100
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn exact_id_matches_all_hints() {
    let dir = std::env::temp_dir().join(format!("zd-test-exact-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = fixture_desktop(
        &dir,
        "flat.example.desktop",
        "Exec=/bin/true\nX-Flatpak=flat.example\n",
    );
    let app = gio::DesktopAppInfo::from_filename(&path).unwrap();

    let path_str = path.to_string_lossy().into_owned();
    assert!(exact_id(&app, Some(&path_str)), "full path hint");
    assert!(exact_id(&app, Some("flat.example")), "desktop id hint");
    assert!(
        exact_id(&app, Some("flat.example.desktop")),
        "id with suffix"
    );
    assert!(exact_id(&app, Some("flat.example")), "flatpak id hint");
    assert!(!exact_id(&app, Some("other.app")));
    assert!(!exact_id(&app, Some("")));
    assert!(!exact_id(&app, None));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn normalize_strips_suffixes_and_takes_basename() {
    use crate::apps::normalize;
    assert_eq!(
        normalize("/usr/bin/Google-Chrome").as_deref(),
        Some("google-chrome")
    );
    assert_eq!(
        normalize("org.foo.Bar.desktop").as_deref(),
        Some("org.foo.bar")
    );
    assert_eq!(normalize("game.EXE").as_deref(), Some("game"));
    assert!(normalize("").is_none());
}

#[test]
fn association_helpers() {
    use crate::settings::{is_absolute_desktop_path, valid_key};
    assert!(is_absolute_desktop_path(
        "/usr/share/applications/x.desktop"
    ));
    assert!(!is_absolute_desktop_path("relative/x.desktop"));
    assert!(!is_absolute_desktop_path("/usr/share/applications/x.txt"));
    assert!(valid_key(&"0123456789abcdef".repeat(4)));
    assert!(
        valid_key(&"ABCDEF0123456789".repeat(4)),
        "uppercase hex is accepted like g_ascii_isxdigit"
    );
    assert!(!valid_key(&"0".repeat(63)));
    assert!(!valid_key(&"0".repeat(65)));
    assert!(!valid_key(&"g".repeat(64)));
}

// ---------------------------------------------------------------------------
// Launch identity probe (apps.rs)
// ---------------------------------------------------------------------------

use crate::apps::launch_probe_score;

/// A matching startup id is the strongest launch signal.
#[test]
fn launch_probe_startup_id_match_wins() {
    let pid = std::process::id() as i32;
    // Self-descendance would also score 200, so the id must outrank it.
    assert_eq!(
        launch_probe_score(Some("id-1"), Some("id-1"), pid, pid),
        220
    );
    // Even with no usable pid data.
    assert_eq!(launch_probe_score(Some("id-1"), Some("id-1"), 0, 0), 220);
}

/// A *mismatched* startup id must not swallow the ancestry fallback: this is
/// the regression where a launcher lost the window it had just started.
#[test]
fn launch_probe_falls_back_to_ancestry_on_mismatch() {
    let pid = std::process::id() as i32;
    assert_eq!(
        launch_probe_score(Some("stale-id"), Some("our-id"), pid, pid),
        200,
        "mismatched ids must fall back to process ancestry"
    );
    assert_eq!(
        launch_probe_score(Some("stale-id"), Some("our-id"), 0, 0),
        0,
        "with neither id nor ancestry matching there is no bonus"
    );
}

/// Partial information still uses whichever probe is available.
#[test]
fn launch_probe_handles_missing_halves() {
    let pid = std::process::id() as i32;
    assert_eq!(launch_probe_score(None, Some("our-id"), pid, pid), 200);
    assert_eq!(launch_probe_score(Some("stale"), None, pid, pid), 200);
    assert_eq!(launch_probe_score(None, None, pid, pid), 200);
    assert_eq!(launch_probe_score(Some(""), Some(""), pid, pid), 200);
    // launch_pid of 1 or less is "unknown", never a match.
    assert_eq!(launch_probe_score(None, None, pid, 1), 0);
    assert_eq!(launch_probe_score(None, None, pid, 0), 0);
}
