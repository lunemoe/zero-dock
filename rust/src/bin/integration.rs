//! Isolated desktop integration suite for the Rust Zero Dock core.
//!
//! Built only with the `test-harness` feature. The runner intentionally uses
//! real GTK windows, libxfce4windowing, X11 properties, the production Dock
//! implementation and (optionally) the real XFCE external wrapper/PulseAudio.
//!
//! Raw `unsafe` here is confined to the `raw` helper module and to the two
//! places that must construct a real `XfcePanelPlugin` through the C ABI: a
//! test harness has to play the part of a foreign process and of a pointer
//! device, which no safe binding can express. The shipped plugin library
//! itself contains no `unsafe` outside its FFI modules.

use gio::prelude::*;
use glib::gobject_ffi::GObject;
use glib::translate::FromGlibPtrNone;
use gtk::prelude::*;
use std::cell::Cell;
use std::ffi::CString;
use std::fs;
use std::os::raw::{c_char, c_int};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};
use x11::{xlib, xtest};
use zero_dock::test_api::{self, DockRef, ScrollInfo, XfwWindow, XfwWorkspace};

type TestResult = Result<(), String>;

macro_rules! check {
    ($cond:expr, $($arg:tt)*) => {
        if !$cond {
            return Err(format!($($arg)*));
        }
    };
}

extern "C" {
    fn xfce_panel_plugin_get_type() -> glib::ffi::GType;
    fn xfce_panel_plugin_provider_set_size(provider: *mut GObject, size: c_int);
    fn xfce_panel_plugin_provider_set_icon_size(provider: *mut GObject, icon_size: c_int);
    fn xfce_panel_plugin_provider_set_mode(provider: *mut GObject, mode: c_int);
    fn g_object_new(
        object_type: glib::ffi::GType,
        first_property_name: *const c_char,
        ...
    ) -> *mut GObject;
    fn g_object_set_data(object: *mut GObject, key: *const c_char, data: *mut std::ffi::c_void);
}

// ---------------------------------------------------------------------------
// Test-only raw X11 / libc helpers
// ---------------------------------------------------------------------------
//
// The harness plays the part of a *foreign application* and a *user with a
// pointer*: it must set WM_CLASS properties directly and inject real pointer
// events, which no safe binding offers. Every raw call is collected in this one
// module; the scenarios above use these safe wrappers only.
mod raw {
    #![allow(unsafe_code)]

    use super::*;

    /// `XSetClassHint` + flush on the fixture's X window.
    pub fn set_class_hint(xid: u64, instance: &str, class: &str) {
        let display = test_api::xdisplay();
        let name = CString::new(instance).unwrap();
        let class = CString::new(class).unwrap();
        let mut hint = xlib::XClassHint {
            res_name: name.as_ptr() as *mut c_char,
            res_class: class.as_ptr() as *mut c_char,
        };
        unsafe {
            xlib::XSetClassHint(display, xid, &mut hint);
            xlib::XFlush(display);
        }
    }

    /// Set or delete a UTF8_STRING window property.
    pub fn set_utf8_property(xid: u64, property: &str, value: Option<&str>) {
        let display = test_api::xdisplay();
        let prop = CString::new(property).unwrap();
        let utf8_name = CString::new("UTF8_STRING").unwrap();
        unsafe {
            let prop = xlib::XInternAtom(display, prop.as_ptr(), xlib::False);
            if let Some(value) = value {
                let utf8 = xlib::XInternAtom(display, utf8_name.as_ptr(), xlib::False);
                xlib::XChangeProperty(
                    display,
                    xid,
                    prop,
                    utf8,
                    8,
                    xlib::PropModeReplace,
                    value.as_bytes().as_ptr(),
                    value.len() as c_int,
                );
            } else {
                xlib::XDeleteProperty(display, xid, prop);
            }
            xlib::XFlush(display);
        }
    }

    /// Move the pointer with XTest.
    pub fn move_pointer(x: i32, y: i32) {
        unsafe {
            xtest::XTestFakeMotionEvent(test_api::xdisplay(), -1, x, y, 0);
            xlib::XFlush(test_api::xdisplay());
        }
    }

    /// Press and release one pointer button with XTest.
    pub fn click_pointer(button: u32) {
        unsafe {
            xtest::XTestFakeButtonEvent(test_api::xdisplay(), button, 1, 0);
            xtest::XTestFakeButtonEvent(test_api::xdisplay(), button, 0, 0);
            xlib::XFlush(test_api::xdisplay());
        }
    }

    /// Press button 1, glide to (`tx`, `ty`) in steps, release.
    pub fn drag_pointer(from: (i32, i32), to: (i32, i32)) {
        unsafe {
            let display = test_api::xdisplay();
            xtest::XTestFakeMotionEvent(display, -1, from.0, from.1, 0);
            xlib::XFlush(display);
            xtest::XTestFakeButtonEvent(display, 1, 1, 0);
            xlib::XFlush(display);
            for step in 1..=12 {
                let x = from.0 + (to.0 - from.0) * step / 12;
                let y = from.1 + (to.1 - from.1) * step / 12;
                xtest::XTestFakeMotionEvent(display, -1, x, y, 0);
                xlib::XFlush(display);
                pump(15);
            }
            xtest::XTestFakeButtonEvent(display, 1, 0, 0);
            xlib::XFlush(display);
        }
    }

    /// Send a signal to another process (test fixtures only).
    pub fn terminate(pid: i32) {
        unsafe {
            libc::kill(pid, libc::SIGTERM);
        }
    }

    /// This process's consumed CPU time in microseconds.
    pub fn cpu_micros() -> u64 {
        unsafe {
            let mut usage: libc::rusage = std::mem::zeroed();
            if libc::getrusage(libc::RUSAGE_SELF, &mut usage) != 0 {
                return 0;
            }
            ((usage.ru_utime.tv_sec + usage.ru_stime.tv_sec) as u64) * 1_000_000
                + (usage.ru_utime.tv_usec + usage.ru_stime.tv_usec) as u64
        }
    }

    /// Live refcount of a raw GObject (leak/over-release probes).
    pub fn ref_count(object: *mut GObject) -> u32 {
        unsafe { (*object).ref_count }
    }
}

static NEXT_ID: AtomicU32 = AtomicU32::new(12000);
/// Fixture windows cascade down the right side; the dock host sits at the
/// top-left. Without explicit positions xfwm4 may stack a fixture over the
/// dock, and XTest input then lands on the wrong window.
static FIXTURE_SLOT: AtomicU32 = AtomicU32::new(0);

fn pump(ms: u64) {
    let end = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < end {
        while glib::MainContext::default().iteration(false) {}
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn wait_until(ms: u64, mut f: impl FnMut() -> bool) -> bool {
    let end = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < end {
        if f() {
            return true;
        }
        pump(20);
    }
    f()
}

fn temp_dir(tag: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "zero-dock-rust-{tag}-{}-{}",
        std::process::id(),
        NEXT_ID.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&path).expect("create integration temp dir");
    path
}

fn write_desktop(path: &Path, name: &str, class: &str, exec: &str) {
    fs::write(
        path,
        format!(
            "[Desktop Entry]\nType=Application\nName={name}\nExec={exec}\nStartupWMClass={class}\n"
        ),
    )
    .expect("write desktop fixture");
}

fn set_wm_class(window: &gtk::Window, class: &str) {
    let Some(gdk_window) = window.window() else {
        panic!("fixture must be realized before WM_CLASS");
    };
    let xid = test_api::gdk_xid(&gdk_window);
    raw::set_class_hint(xid, "zero-dock-fixture", class);
}

fn set_utf8_property(window: &gtk::Window, property: &str, value: Option<&str>) {
    let xid = test_api::gdk_xid(&window.window().expect("realized fixture"));
    raw::set_utf8_property(xid, property, value);
}

fn fixture(title: &str, class: &str) -> gtk::Window {
    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.set_title(title);
    window.set_default_size(380, 220);
    let slot = FIXTURE_SLOT.fetch_add(1, Ordering::Relaxed) % 3;
    window.move_(660, 60 + 240 * slot as i32);
    let events = gtk::EventBox::new();
    let label = gtk::Label::new(Some("Zero Dock Rust integration fixture\npreview pixels"));
    events.set_child(Some(&label));
    window.set_child(Some(&events));
    window.realize();
    set_wm_class(&window, class);
    window.show_all();
    window
}

fn color_icon(window: &gtk::Window, rgba: u32) {
    let pix =
        gdk_pixbuf::Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, true, 8, 32, 32).expect("icon pixbuf");
    pix.fill(rgba);
    window.set_icon(Some(&pix));
}

fn widget_center(widget: &impl IsA<gtk::Widget>) -> (i32, i32) {
    let widget = widget.as_ref();
    let top = widget.toplevel().expect("widget toplevel");
    let (wx, wy) = widget
        .translate_coordinates(&top, 0, 0)
        .expect("widget coordinates");
    let top_window = top.window().expect("toplevel GDK window");
    // gdk_window_get_origin binds as (screen-number, x, y); the old C test
    // passed x/y out through pointers. Taking the first two fields here sent
    // every XTest coordinate to a wrong, screen-number-skewed point.
    let (_, ox, oy) = top_window.origin();
    let allocation = widget.allocation();
    (
        ox + wx + allocation.width() / 2,
        oy + wy + allocation.height() / 2,
    )
}

/// Move the pointer over a widget without clicking (C harness `mouse_at`).
fn hover_widget(widget: &impl IsA<gtk::Widget>) {
    let (x, y) = widget_center(widget);
    raw::move_pointer(x, y);
    pump(120);
}

fn click_widget(widget: &impl IsA<gtk::Widget>, button: u32) {
    let (x, y) = widget_center(widget);
    raw::move_pointer(x, y);
    pump(120);
    raw::click_pointer(button);
    pump(120);
}

fn drag_between(source: &impl IsA<gtk::Widget>, target: &impl IsA<gtk::Widget>, before: bool) {
    let (sx, sy) = widget_center(source);
    let (mut tx, mut ty) = widget_center(target);
    let allocation = target.as_ref().allocation();
    if allocation.width() >= allocation.height() {
        tx += if before {
            -allocation.width() / 4
        } else {
            allocation.width() / 4
        };
    } else {
        ty += if before {
            -allocation.height() / 4
        } else {
            allocation.height() / 4
        };
    }
    raw::drag_pointer((sx, sy), (tx, ty));
    pump(220);
}

struct Host {
    window: gtk::Window,
    dock: DockRef,
    raw_plugin: *mut GObject,
    _rc_c: CString,
}

impl Host {
    fn new(rc: &Path) -> Host {
        let window = gtk::Window::new(gtk::WindowType::Toplevel);
        window.set_title("Zero Dock Rust Integration Host");
        window.set_skip_taskbar_hint(true);
        window.set_accept_focus(false);
        window.set_default_size(600, 56);

        let name = CString::new("zero-dock").unwrap();
        let display = CString::new("Zero Dock Test").unwrap();
        let comment = CString::new("Rust integration").unwrap();
        let unique = NEXT_ID.fetch_add(1, Ordering::Relaxed) as i32;
        let raw_plugin = unsafe {
            g_object_new(
                xfce_panel_plugin_get_type(),
                c"name".as_ptr(),
                name.as_ptr(),
                c"unique-id".as_ptr(),
                unique,
                c"display-name".as_ptr(),
                display.as_ptr(),
                c"comment".as_ptr(),
                comment.as_ptr(),
                std::ptr::null::<c_char>(),
            )
        };
        assert!(!raw_plugin.is_null());

        let rc_c = CString::new(rc.to_string_lossy().as_bytes()).unwrap();
        unsafe {
            g_object_set_data(
                raw_plugin,
                c"test-rc".as_ptr(),
                rc_c.as_ptr() as *mut std::ffi::c_void,
            );
        }

        let plugin_object: glib::Object = unsafe { FromGlibPtrNone::from_glib_none(raw_plugin) };
        let plugin_widget: gtk::Widget = plugin_object
            .downcast()
            .expect("XfcePanelPlugin must be a GtkWidget");
        window.set_child(Some(&plugin_widget));
        unsafe { xfce_panel_plugin_provider_set_size(raw_plugin, 48) };

        let dock = unsafe { test_api::construct(raw_plugin) }.expect("construct dock");
        // Pin the dock to the top-left, clear of the cascading fixtures, so
        // XTest input always lands on dock widgets.
        window.move_(10, 10);
        window.show_all();
        pump(100);
        Host {
            window,
            dock,
            raw_plugin,
            _rc_c: rc_c,
        }
    }

    fn refresh(&self) {
        self.dock.borrow_mut().refresh();
        pump(100);
    }

    fn set_icon_size(&self, size: i32) {
        unsafe { xfce_panel_plugin_provider_set_icon_size(self.raw_plugin, size) };
        pump(100);
    }

    fn set_mode(&self, mode: i32) {
        unsafe { xfce_panel_plugin_provider_set_mode(self.raw_plugin, mode) };
        pump(100);
    }

    fn close(self) {
        let Host {
            window,
            dock,
            raw_plugin: _,
            _rc_c,
        } = self;
        zero_dock::test_api::destroy_widget(&window);
        pump(120);
        // Production teardown is driven by the plugin object's free-data /
        // object-data destroy path. Keep this assertion in the integration
        // harness so tests cannot silently fall back to an artificial order.
        assert!(
            dock.borrow().disposing,
            "plugin destruction did not dispose Dock"
        );
        drop(dock);
        drop(_rc_c);
        pump(40);
    }
}

fn xid(window: &gtk::Window) -> u64 {
    test_api::gdk_xid(&window.window().expect("realized fixture"))
}

fn button_for(host: &Host, window: &gtk::Window) -> Option<u64> {
    test_api::button_for_xid(&host.dock.borrow(), xid(window))
}

fn window_count(host: &Host) -> usize {
    host.dock.borrow().window_index.len()
}

fn scenario_native_all() -> TestResult {
    let dir = temp_dir("native");
    let rc = dir.join("dock.rc");
    let desktop = dir.join("drag.desktop");
    write_desktop(
        &desktop,
        "Dragged Fixture",
        "ZeroDockFixture",
        "/usr/bin/true",
    );

    let host = Host::new(&rc);

    // Real XFCE provider property changes must drive the Rust signal handlers.
    host.set_icon_size(0);
    check!(
        host.dock.borrow().icon_size == 40,
        "automatic icon size should be 40 for a 48px single-row panel"
    );
    host.set_icon_size(36);
    check!(
        host.dock.borrow().icon_size == 36,
        "explicit provider icon size should propagate"
    );

    let a = fixture("Native A", "ZeroDockFixture");
    let b = fixture("Native B", "ZeroDockFixture");
    pump(300);
    host.refresh();

    check!(window_count(&host) == 2, "expected two tracked windows");
    let mut aid = button_for(&host, &a).ok_or("missing button A")?;
    let mut bid = button_for(&host, &b).ok_or("missing button B")?;
    check!(aid != bid, "two windows must have independent buttons");

    // Real URI-list drag from another GTK widget pins the running button.
    // The drag source is fixture B's child (a real GTK drag origin) without
    // adding another tracked toplevel window.
    let drag_source: gtk::Widget = b.child().ok_or("fixture B child")?.upcast::<gtk::Widget>();
    let uri_target = [gtk::TargetEntry::new(
        "text/uri-list",
        gtk::TargetFlags::empty(),
        2,
    )];
    drag_source.drag_source_set(
        gdk::ModifierType::BUTTON1_MASK,
        &uri_target,
        gdk::DragAction::COPY,
    );
    let uri = format!("file://{}", desktop.to_string_lossy());
    drag_source.connect_drag_data_get(move |_w, _ctx, data, _info, _time| {
        data.set_uris(&[uri.as_str()]);
    });
    pump(100);
    let target_main = host
        .dock
        .borrow()
        .button(aid)
        .ok_or("drag target A")?
        .main
        .clone();
    drag_between(&drag_source, &target_main, true);
    pump(250);
    host.refresh();
    aid = button_for(&host, &a).ok_or("A missing after URI drag")?;
    check!(
        host.dock
            .borrow()
            .button(aid)
            .is_some_and(|button| button.pinned),
        "desktop URI drag should pin the running button in place"
    );

    // Real mouse drag between dock buttons must exercise GTK DnD ordering.
    bid = button_for(&host, &b).ok_or("B missing before reorder")?;
    let (source_id, target_id) = {
        let d = host.dock.borrow();
        let ai = d.button_index(aid).ok_or("A index")?;
        let bi = d.button_index(bid).ok_or("B index")?;
        if ai < bi {
            (bid, aid)
        } else {
            (aid, bid)
        }
    };
    let source_main = host
        .dock
        .borrow()
        .button(source_id)
        .ok_or("source button")?
        .main
        .clone();
    let reorder_target = host
        .dock
        .borrow()
        .button(target_id)
        .ok_or("target button")?
        .main
        .clone();
    drag_between(&source_main, &reorder_target, true);
    let d = host.dock.borrow();
    check!(
        d.button_index(source_id).ok_or("source index after DnD")?
            < d.button_index(target_id).ok_or("target index after DnD")?,
        "real mouse drag should reorder dock buttons"
    );
    drop(d);

    // Composite capture and minimize path.
    {
        let mut d = host.dock.borrow_mut();
        let captured = d.capture(aid);
        check!(captured.is_some(), "XComposite capture should succeed");
        d.minimize(aid);
    }
    check!(
        wait_until(1000, || {
            host.dock
                .borrow()
                .button(aid)
                .and_then(|button| button.window.as_ref())
                .is_some_and(|window| XfwWindow::new(window).is_minimized())
        }),
        "minimize did not reach Xfw"
    );

    // Open the real preview and click its image EventBox with XTest. This must
    // restore/activate the minimized window and release panel autohide.
    {
        let mut d = host.dock.borrow_mut();
        d.settings.preview_delay = 100;
        d.schedule_preview(aid);
    }
    pump(300);
    let image_box: gtk::Widget = {
        let d = host.dock.borrow();
        check!(
            d.preview
                .as_ref()
                .is_some_and(|preview| preview.is_visible()),
            "preview should become visible"
        );
        check!(d.autohide_blocked, "preview should block panel autohide");
        let preview = d.preview.as_ref().unwrap();
        let content = preview
            .child()
            .ok_or("preview content")?
            .downcast::<gtk::Box>()
            .map_err(|_| "preview content is not a GtkBox".to_string())?;
        content
            .children()
            .into_iter()
            .next()
            .ok_or("preview image EventBox")?
    };
    click_widget(&image_box, 1);
    check!(
        wait_until(1000, || {
            host.dock
                .borrow()
                .button(aid)
                .and_then(|button| button.window.as_ref())
                .is_some_and(|window| {
                    let window = XfwWindow::new(window);
                    window.is_active() && !window.is_minimized()
                })
        }),
        "preview click did not activate/restore window"
    );
    check!(
        !host.dock.borrow().autohide_blocked,
        "preview click must release autohide"
    );
    check!(
        host.dock
            .borrow()
            .preview
            .as_ref()
            .is_none_or(|preview| !preview.is_visible()),
        "preview should hide after activation"
    );

    // Tooltip fallback when visual previews are disabled.
    {
        let mut d = host.dock.borrow_mut();
        d.settings.previews = false;
        d.update_buttons();
        check!(
            d.button(aid)
                .and_then(|button| button.main.tooltip_text())
                .is_some(),
            "disabled previews require a tooltip fallback"
        );
        d.settings.previews = true;
        d.window_menu(aid, None);
        check!(
            d.menu.as_ref().is_some_and(|menu| menu.is_visible()),
            "window menu should open"
        );
        if let Some(menu) = d.menu.take() {
            zero_dock::test_api::destroy_widget(&menu);
        }
    }

    // Real provider mode switch must rotate the dock.
    host.set_mode(1); // XFCE_PANEL_PLUGIN_MODE_VERTICAL
    check!(
        host.dock.borrow().orientation == gtk::Orientation::Vertical,
        "provider vertical mode should rotate the dock"
    );
    check!(
        host.dock.borrow().box_.orientation() == gtk::Orientation::Vertical,
        "inner box did not rotate"
    );

    // Minimize-all uses the production capture path and preserves frames.
    host.dock.borrow_mut().activate(aid);
    host.dock.borrow_mut().activate(bid);
    pump(150);
    host.dock.borrow_mut().minimize_all();
    check!(
        wait_until(1200, || {
            let d = host.dock.borrow();
            [aid, bid].iter().all(|id| {
                d.button(*id)
                    .and_then(|button| button.window.as_ref())
                    .is_some_and(|window| XfwWindow::new(window).is_minimized())
            })
        }),
        "minimize-all did not minimize every tracked window"
    );
    {
        let d = host.dock.borrow();
        check!(
            d.button(aid)
                .and_then(|button| button.thumbnail.as_ref())
                .is_some()
                && d.button(bid)
                    .and_then(|button| button.thumbnail.as_ref())
                    .is_some(),
            "minimize-all should leave cached frames"
        );
    }

    // Panel show-desktop item must mirror Xfw screen state in both directions.
    {
        let screen = host
            .dock
            .borrow()
            .screen
            .clone()
            .ok_or("screen unavailable")?;
        if screen.show_desktop() {
            screen.set_show_desktop(false);
            pump(100);
        }
        let item = host
            .dock
            .borrow()
            .show_desktop_item
            .clone()
            .ok_or("show-desktop menu item")?;
        item.activate();
        pump(200);
        host.dock.borrow_mut().menu_sync();
        check!(screen.show_desktop(), "show-desktop activation failed");
        check!(item.is_active(), "show-desktop menu state did not sync");
        item.activate();
        pump(200);
        host.dock.borrow_mut().menu_sync();
        check!(
            !screen.show_desktop(),
            "show-desktop did not toggle back off"
        );
        check!(!item.is_active(), "show-desktop item stayed active");
    }

    // If a hovered window disappears, preview/autohide state must be cleared.
    // Hover the pinned button (which holds A's window) like a real pointer
    // would, then close the hovered window: the pinned button survives as a
    // launcher, so no layout shift can hand the hover to a neighbor.
    let pin_id = {
        let d = host.dock.borrow();
        d.buttons
            .iter()
            .find(|button| button.pinned)
            .map(|button| button.id)
            .ok_or("missing pinned button")?
    };
    host.dock.borrow_mut().activate(pin_id);
    {
        let main = host
            .dock
            .borrow()
            .button(pin_id)
            .ok_or("pin button for hover")?
            .main
            .clone();
        hover_widget(&main);
    }
    {
        let mut d = host.dock.borrow_mut();
        d.schedule_preview(pin_id);
    }
    pump(300);
    check!(
        host.dock.borrow().hover_button == Some(pin_id),
        "expected the pinned button to own preview hover"
    );
    a.close();
    pump(350);
    host.refresh();
    check!(
        host.dock.borrow().hover_button.is_none(),
        "closing hovered window must clear hover"
    );
    check!(
        !host.dock.borrow().autohide_blocked,
        "closing hovered window must release autohide"
    );

    b.close();
    pump(250);
    host.refresh();
    check!(window_count(&host) == 0, "windows should disappear cleanly");
    host.close();
    let _ = fs::remove_dir_all(dir);
    Ok(())
}

fn scenario_pinned_lifecycle() -> TestResult {
    let dir = temp_dir("pins");
    let rc = dir.join("dock.rc");
    let desktop = dir.join("fixture.desktop");
    write_desktop(
        &desktop,
        "Pinned Fixture",
        "ZeroDockPinned",
        "/usr/bin/true",
    );
    let host = Host::new(&rc);
    let pin = host
        .dock
        .borrow_mut()
        .add_pin(&desktop.to_string_lossy())
        .ok_or("add pin failed")?;
    check!(
        host.dock
            .borrow()
            .button(pin)
            .is_some_and(|b| b.pinned && b.window.is_none()),
        "pin should start idle"
    );

    let first = fixture("Pinned first", "ZeroDockPinned");
    pump(250);
    host.refresh();
    check!(
        button_for(&host, &first) == Some(pin),
        "first window must reuse pin"
    );
    let original_widget = host.dock.borrow().button(pin).unwrap().widget.clone();

    let second = fixture("Pinned second", "ZeroDockPinned");
    pump(250);
    host.refresh();
    check!(window_count(&host) == 2, "two live windows expected");
    let second_id = button_for(&host, &second).ok_or("second button missing")?;
    check!(
        second_id != pin,
        "second live window needs a separate button"
    );

    host.dock.borrow_mut().close_window(pin);
    pump(350);
    host.refresh();
    check!(window_count(&host) == 1, "one window should remain");
    check!(
        host.dock
            .borrow()
            .button(pin)
            .is_some_and(|b| { b.window_xid() == xid(&second) && b.widget == original_widget }),
        "remaining window must transfer into pinned widget"
    );

    host.dock.borrow_mut().unpin(pin);
    check!(
        host.dock
            .borrow()
            .button(pin)
            .is_some_and(|b| !b.pinned && b.window.is_some()),
        "unpin must retain live window"
    );
    let app = gio::DesktopAppInfo::from_filename(&desktop).ok_or("desktop app")?;
    host.dock.borrow_mut().pin_app(&app);
    let repinned = button_for(&host, &second).ok_or("repinned button missing")?;
    check!(
        host.dock
            .borrow()
            .button(repinned)
            .is_some_and(|b| b.pinned),
        "repin live app"
    );

    host.dock.borrow_mut().close_window(repinned);
    pump(350);
    host.refresh();
    check!(
        window_count(&host) == 0,
        "last close should clear live window"
    );
    check!(
        host.dock.borrow().buttons.iter().any(|b| b.pinned
            && b.window.is_none()
            && b.desktop.as_deref() == Some(desktop.to_string_lossy().as_ref())),
        "last close must restore idle launcher"
    );

    first.close();
    second.close();
    host.close();
    let _ = fs::remove_dir_all(dir);
    Ok(())
}

fn fixture_process(class: &str, title: &str, seconds: u64) {
    gtk::init().expect("fixture gtk init");
    let window = fixture(title, class);
    let loop_ = glib::MainLoop::new(None, false);
    let quit = loop_.clone();
    window.connect_destroy(move |_| quit.quit());
    let quit = loop_.clone();
    glib::timeout_add_seconds_local(seconds.max(1) as u32, move || {
        quit.quit();
        glib::ControlFlow::Break
    });
    loop_.run();
}

fn scenario_launch_feedback() -> TestResult {
    let dir = temp_dir("launch");
    let rc = dir.join("dock.rc");
    let success = dir.join("success.desktop");
    let timeout = dir.join("timeout.desktop");
    let bad = dir.join("bad.desktop");
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let quoted = format!("\"{}\"", exe.to_string_lossy());
    write_desktop(
        &success,
        "Launch Success",
        "ZeroDockLaunch",
        &format!("{quoted} --fixture-process ZeroDockLaunch LaunchSuccess 5"),
    );
    write_desktop(&timeout, "No Window", "NoWindowClass", "/usr/bin/true");
    fs::write(
        &bad,
        "[Desktop Entry]\nType=Application\nName=Bad\nExec=/definitely/missing/zero-dock-binary\n",
    )
    .map_err(|e| e.to_string())?;

    let host = Host::new(&rc);
    let success_id = host
        .dock
        .borrow_mut()
        .add_pin(&success.to_string_lossy())
        .ok_or("success pin")?;
    let timeout_id = host
        .dock
        .borrow_mut()
        .add_pin(&timeout.to_string_lossy())
        .ok_or("timeout pin")?;
    let bad_id = host
        .dock
        .borrow_mut()
        .add_pin(&bad.to_string_lossy())
        .ok_or("bad pin")?;

    host.dock.borrow_mut().settings.launch_timeout = 500;
    host.dock.borrow_mut().launch(success_id);
    check!(
        host.dock
            .borrow()
            .button(success_id)
            .is_some_and(|b| b.launching),
        "launch should enter pending state"
    );
    check!(
        wait_until(2500, || {
            host.dock.borrow_mut().refresh();
            host.dock
                .borrow()
                .button(success_id)
                .is_some_and(|b| b.window.is_some() && !b.launching)
        }),
        "launched fixture window did not bind to pin"
    );
    check!(
        host.dock
            .borrow()
            .button(success_id)
            .unwrap()
            .launch_error
            .is_none(),
        "successful launch must clear error"
    );

    host.dock.borrow_mut().launch(timeout_id);
    check!(
        wait_until(1600, || {
            !host
                .dock
                .borrow()
                .button(timeout_id)
                .is_some_and(|b| b.launching)
        }),
        "no-window launch should time out"
    );
    check!(
        host.dock
            .borrow()
            .button(timeout_id)
            .is_some_and(|b| b.launch_error.is_some()),
        "timeout must expose launch error"
    );

    host.dock.borrow_mut().launch(bad_id);
    pump(150);
    check!(
        host.dock
            .borrow()
            .button(bad_id)
            .is_some_and(|b| !b.launching && b.launch_error.is_some()),
        "spawn failure must be visible"
    );

    let diagnostic = host.dock.borrow().diagnostics();
    check!(
        diagnostic.contains("Zero Dock"),
        "diagnostics should identify product"
    );
    check!(
        !diagnostic.contains(&dir.to_string_lossy().to_string()),
        "diagnostics must not leak temp/user paths"
    );
    host.dock.borrow_mut().save();
    let file = glib::KeyFile::new();
    file.load_from_file(&rc, glib::KeyFileFlags::NONE)
        .map_err(|e| e.to_string())?;
    let pins = file
        .string_list("Dock", "Pinned")
        .map_err(|e| e.to_string())?;
    check!(
        pins.len() == 3,
        "all pins including failed launch must persist"
    );

    if let Some(window) = host
        .dock
        .borrow()
        .button(success_id)
        .and_then(|b| b.window.clone())
    {
        XfwWindow::new(&window).close(0);
    }
    pump(250);
    host.close();
    let _ = fs::remove_dir_all(dir);
    Ok(())
}

fn scenario_identity_workspaces() -> TestResult {
    let dir = temp_dir("identity");
    let rc = dir.join("dock.rc");
    let first = dir.join("first.desktop");
    let second = dir.join("second.desktop");
    write_desktop(
        &first,
        "Identity First",
        "ZeroDockIdentity",
        "/usr/bin/true",
    );
    write_desktop(
        &second,
        "Identity Second",
        "ZeroDockIdentity",
        "/usr/bin/true",
    );

    let host = Host::new(&rc);
    let first_id = host
        .dock
        .borrow_mut()
        .add_pin(&first.to_string_lossy())
        .ok_or("first pin")?;
    let second_id = host
        .dock
        .borrow_mut()
        .add_pin(&second.to_string_lossy())
        .ok_or("second pin")?;
    let w = fixture("Identity Window", "ZeroDockIdentity");
    pump(250);
    host.refresh();
    check!(
        button_for(&host, &w) == Some(first_id),
        "first pin should win tie"
    );

    set_utf8_property(&w, "_GTK_APPLICATION_ID", Some("second"));
    pump(120);
    host.dock.borrow_mut().refresh();
    check!(
        button_for(&host, &w) == Some(second_id),
        "_GTK_APPLICATION_ID should rebind shared class"
    );

    set_utf8_property(&w, "_GTK_APPLICATION_ID", None);
    set_utf8_property(
        &w,
        "_KDE_NET_WM_DESKTOP_FILE",
        Some(&first.to_string_lossy()),
    );
    pump(120);
    host.dock.borrow_mut().refresh();
    check!(
        button_for(&host, &w) == Some(first_id),
        "desktop-file identity should rebind to first pin"
    );

    let w2 = fixture("Workspace Window", "ZeroDockIdentity");
    pump(250);
    host.refresh();
    let w2_id = button_for(&host, &w2).ok_or("workspace second button")?;

    let screen = host
        .dock
        .borrow()
        .screen
        .clone()
        .ok_or("Xfw screen unavailable")?;
    let spaces = test_api::workspaces(&screen);
    if spaces.len() >= 2 {
        let current = host
            .dock
            .borrow()
            .button(first_id)
            .and_then(|b| b.window.clone())
            .and_then(|w| XfwWindow::new(&w).workspace())
            .ok_or("window workspace")?;
        let other = spaces
            .iter()
            .find(|s| s.as_ptr() != current.as_ptr())
            .cloned()
            .ok_or("alternate workspace")?;
        host.dock.borrow_mut().settings.all_workspaces = false;
        let first_window = host
            .dock
            .borrow()
            .button(first_id)
            .unwrap()
            .window
            .clone()
            .unwrap();
        XfwWindow::new(&first_window).move_to_workspace(&other);
        pump(200);
        host.dock.borrow_mut().refresh();
        check!(
            host.dock
                .borrow()
                .button(first_id)
                .is_some_and(|b| b.widget.is_visible()),
            "pinned item remains visible across workspace filtering"
        );
        XfwWorkspace::activate(&other);
        pump(200);
        host.dock.borrow_mut().refresh();
        check!(
            test_api::button_for_xid(&host.dock.borrow(), xid(&w)).is_some(),
            "workspace switch must preserve window tracking"
        );
    } else {
        eprintln!("SKIP workspace move subcase: fewer than 2 workspaces");
    }

    check!(
        w2_id != first_id || window_count(&host) == 2,
        "two windows stay tracked"
    );
    w.close();
    w2.close();
    pump(250);
    host.close();
    let _ = fs::remove_dir_all(dir);
    Ok(())
}

fn scenario_improvements() -> TestResult {
    let dir = temp_dir("improvements");
    let rc = dir.join("settings.rc");
    let desktop = dir.join("manual.desktop");
    let backup = dir.join("backup.rc");
    let bad = dir.join("bad.rc");
    write_desktop(
        &desktop,
        "Manual association",
        "DistinctManual",
        "/usr/bin/true",
    );

    let host = Host::new(&rc);
    {
        let d = host.dock.borrow();
        // The dock must use the screen default theme so lookups follow the
        // user's icon theme instead of the Adwaita/hicolor fallbacks.
        let dock_theme: *mut glib::gobject_ffi::GObject =
            glib::translate::ToGlibPtr::to_glib_none(&d.icon_theme.clone().upcast::<glib::Object>())
                .0;
        let default_theme = gtk::IconTheme::default().expect("default icon theme");
        let screen_theme: *mut glib::gobject_ffi::GObject =
            glib::translate::ToGlibPtr::to_glib_none(&default_theme.clone().upcast::<glib::Object>())
                .0;
        check!(
            dock_theme == screen_theme,
            "dock must use the screen default icon theme"
        );
    }
    {
        // Opening the preferences dialog borrows the plugin widget to place
        // it; that must not steal a reference to the plugin object itself.
        let plugin = host.raw_plugin;
        let before = raw::ref_count(plugin);
        {
            let mut d = host.dock.borrow_mut();
            d.configure();
            check!(d.settings_dialog.is_some(), "settings dialog must open");
        }
        let after_open = raw::ref_count(plugin);
        check!(
            after_open == before,
            "configure() must not change the plugin refcount"
        );
        let dialog = host
            .dock
            .borrow_mut()
            .settings_dialog
            .take()
            .expect("settings dialog still open");
        zero_dock::test_api::destroy_widget(&dialog);
    }
    pump(100);
    let mut pin = host
        .dock
        .borrow_mut()
        .add_pin(&desktop.to_string_lossy())
        .ok_or("manual pin")?;
    let first = fixture("Manual association fixture", "UnrelatedBeforeAssociation");
    check!(
        wait_until(2500, || {
            host.refresh();
            button_for(&host, &first).is_some()
        }),
        "running fixture"
    );
    let running = button_for(&host, &first).expect("running fixture vanished");
    let window = host
        .dock
        .borrow()
        .button(running)
        .unwrap()
        .window
        .clone()
        .unwrap();

    check!(
        host.dock
            .borrow_mut()
            .associate(&window, Some("/missing.desktop"))
            .is_err(),
        "invalid association must fail"
    );
    host.dock
        .borrow_mut()
        .associate(&window, Some(&desktop.to_string_lossy()))
        .map_err(|e| e.to_string())?;
    pump(150);
    host.refresh();
    check!(
        button_for(&host, &first) == Some(pin),
        "manual rule should rebind pin"
    );
    // The association API only borrows the XfwWindow. Do not keep an owning
    // test-side clone alive past Dock/Screen teardown.
    drop(window);

    let mut others = Vec::new();
    for i in 0..7 {
        others.push(fixture(
            &format!("Overflow {i}"),
            &format!("OverflowClass{i}"),
        ));
    }
    pump(300);
    host.refresh();
    {
        let mut d = host.dock.borrow_mut();
        d.settings.max_visible = 3;
        d.settings.left_action = 1;
        d.settings.middle_action = 2;
        d.settings.scroll_windows = false;
        d.update_buttons();
        let visible = d.buttons.iter().filter(|b| b.widget.is_visible()).count();
        check!(
            visible <= 2,
            "max_visible=3 leaves at most two normal buttons"
        );
        check!(d.overflow.is_visible(), "overflow button must be visible");

        d.config_export(&backup.to_string_lossy())
            .map_err(|e| e.to_string())?;
        d.association_clear(pin);
        d.settings.max_visible = 0;
    }

    host.dock
        .borrow_mut()
        .config_import(&backup.to_string_lossy())
        .map_err(|e| e.to_string())?;
    {
        let d = host.dock.borrow();
        check!(d.settings.max_visible == 3, "config restore max_visible");
        check!(d.settings.left_action == 1, "config restore left action");
        check!(
            d.settings.middle_action == 2,
            "config restore middle action"
        );
        check!(!d.settings.scroll_windows, "config restore scroll setting");
        check!(d.associations.len() == 1, "association restore");
    }
    pin = button_for(&host, &first).ok_or("button missing after config restore")?;
    check!(
        host.dock.borrow().button(pin).is_some_and(|b| b.pinned),
        "restored live window should be owned by a pin"
    );

    let before = fs::read(&rc).map_err(|e| e.to_string())?;
    fs::write(&bad, "[Other]\nKey=bad\n").map_err(|e| e.to_string())?;
    check!(
        host.dock
            .borrow_mut()
            .config_import(&bad.to_string_lossy())
            .is_err(),
        "invalid config must be rejected"
    );
    let after = fs::read(&rc).map_err(|e| e.to_string())?;
    check!(
        before == after,
        "invalid restore must not mutate live config"
    );

    {
        let mut d = host.dock.borrow_mut();
        d.settings.max_visible = 0;
        d.update_buttons();
        d.settings.preview_delay = 100;
        d.schedule_preview(pin);
    }
    pump(250);
    {
        let mut d = host.dock.borrow_mut();
        check!(
            d.preview.as_ref().is_some_and(|p| p.is_visible()),
            "preview should open"
        );
        check!(
            d.preview_title
                .as_ref()
                .is_some_and(|l| !l.text().is_empty()),
            "preview title"
        );
        check!(
            d.preview_workspace
                .as_ref()
                .is_some_and(|l| !l.text().is_empty()),
            "preview workspace label"
        );
        let preview = d.preview.as_ref().unwrap();
        let (x, y) = preview.position();
        let (width, height) = preview.size();
        let bounds = gdk::Display::default()
            .and_then(|display| {
                d.container
                    .window()
                    .and_then(|w| display.monitor_at_window(&w))
            })
            .map(|m| m.workarea())
            .unwrap_or_else(|| gdk::Rectangle::new(0, 0, 1024, 768));
        check!(
            x >= bounds.x() && y >= bounds.y(),
            "preview starts in monitor bounds"
        );
        check!(
            x + width <= bounds.x() + bounds.width() && y + height <= bounds.y() + bounds.height(),
            "preview ends in monitor bounds"
        );

        for b in &mut d.buttons {
            b.thumbnail = gdk_pixbuf::Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, true, 8, 2048, 1024);
            b.thumbnail_time += b.id as i64;
        }
        d.trim_frames(pin);
        let total: usize = d
            .buttons
            .iter()
            .filter_map(|b| b.thumbnail.as_ref())
            .map(|p| p.byte_length())
            .sum();
        check!(
            total <= 32 * 1024 * 1024,
            "frame cache exceeded 32 MiB: {total}"
        );
        check!(
            d.button(pin).and_then(|b| b.thumbnail.as_ref()).is_some(),
            "trim_frames must keep requested frame"
        );
        d.hide_preview();
        // Restore the exported value so production teardown persists the same
        // configuration we just verified importing.
        d.settings.max_visible = 3;
    }

    for w in others {
        w.close();
    }
    first.close();
    pump(250);
    host.close();

    // Reload persisted association/config and ensure it survives instance recreation.
    let host2 = Host::new(&rc);
    check!(
        host2.dock.borrow().associations.len() == 1,
        "rule survives reload"
    );
    check!(
        host2.dock.borrow().settings.max_visible == 3,
        "settings survive reload"
    );
    host2.close();

    let _ = fs::remove_dir_all(dir);
    Ok(())
}

fn resident_kib() -> u64 {
    fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|text| {
            text.lines().find_map(|line| {
                line.strip_prefix("VmRSS:")
                    .and_then(|rest| rest.split_whitespace().next())
                    .and_then(|v| v.parse().ok())
            })
        })
        .unwrap_or(0)
}

fn cpu_micros() -> u64 {
    raw::cpu_micros()
}

fn scenario_stress() -> TestResult {
    let dir = temp_dir("stress");
    let rc = dir.join("dock.rc");
    let host = Host::new(&rc);
    let initial = resident_kib();
    let cycles: usize = std::env::var("ZERO_DOCK_STRESS_CYCLES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(5);

    let mut warm = initial;
    for cycle in 0..cycles {
        let mut windows = Vec::new();
        for i in 0..32 {
            windows.push(fixture(
                &format!("Stress {cycle}-{i}"),
                &format!("StressClass{cycle}_{i}"),
            ));
        }
        pump(300);
        host.refresh();
        check!(
            window_count(&host) == 32,
            "stress cycle {cycle}: expected 32 windows"
        );
        check!(
            host.dock.borrow().overflow.is_visible(),
            "stress overflow should be visible"
        );

        // Exercise capture/cache and button refresh on a representative sample.
        let ids: Vec<u64> = host
            .dock
            .borrow()
            .buttons
            .iter()
            .filter(|b| b.window.is_some())
            .take(8)
            .map(|b| b.id)
            .collect();
        for id in ids {
            let _ = host.dock.borrow_mut().capture(id);
        }
        host.dock.borrow_mut().update_audio_buttons();

        for w in windows {
            w.close();
        }
        pump(350);
        host.refresh();
        check!(
            window_count(&host) == 0,
            "stress cycle {cycle}: stale windows remain"
        );
        check!(
            host.dock
                .borrow()
                .buttons
                .iter()
                .all(|b| b.window.is_none()),
            "stress cycle {cycle}: live button leaked"
        );
        if cycle == 0 {
            warm = resident_kib();
        }
    }

    let cpu_before = cpu_micros();
    let wall = Instant::now();
    pump(2000);
    let cpu_after = cpu_micros();
    let elapsed = wall.elapsed().as_micros().max(1) as f64;
    let cpu_pct = 100.0 * (cpu_after.saturating_sub(cpu_before)) as f64 / elapsed;
    check!(
        matches!(cpu_pct.partial_cmp(&20.0), Some(std::cmp::Ordering::Less)),
        "idle CPU unexpectedly high: {cpu_pct:.1}%"
    );

    let final_rss = resident_kib();
    check!(
        final_rss < warm.max(initial) + 128 * 1024,
        "RSS grew too much: initial={initial} warm={warm} final={final_rss} KiB"
    );
    host.close();
    let _ = fs::remove_dir_all(dir);
    Ok(())
}

fn start_pacat() -> Result<Child, String> {
    Command::new("pacat")
        .args([
            "--playback",
            "--raw",
            "--rate=8000",
            "--channels=1",
            "--format=s16le",
            "--volume=0",
            "/dev/zero",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("start pacat: {e}"))
}

fn scenario_audio_recovery() -> TestResult {
    let pulse_pid: i32 = match std::env::var("ZERO_DOCK_PRIVATE_PULSE_PID")
        .ok()
        .and_then(|v| v.parse().ok())
    {
        Some(pid) => pid,
        None => {
            eprintln!("SKIP audio-recovery: private audio was not requested");
            return Ok(());
        }
    };
    check!(
        Command::new("sh")
            .arg("-c")
            .arg("command -v pacat >/dev/null && command -v pactl >/dev/null")
            .status()
            .map(|s| s.success())
            .unwrap_or(false),
        "audio recovery requires pacat and pactl"
    );

    let dir = temp_dir("audio");
    let rc = dir.join("dock.rc");
    let window = fixture("Private audio recovery", "ZeroDockAudio");
    let host = Host::new(&rc);
    check!(
        wait_until(2500, || {
            host.refresh();
            button_for(&host, &window).is_some()
        }),
        "audio fixture button"
    );
    let id = button_for(&host, &window).expect("audio fixture vanished");

    let mut stream = start_pacat()?;
    check!(
        wait_until(2000, || host.dock.borrow_mut().audio_status(id).present),
        "audio stream did not appear"
    );

    // Volume arithmetic is 5% per step and must clamp to [0%, 200%].
    host.dock.borrow_mut().audio_volume(id, -100);
    pump(150);
    check!(
        host.dock.borrow_mut().audio_status(id).percent == 0,
        "volume floor should clamp to 0%"
    );
    host.dock.borrow_mut().audio_volume(id, 1);
    pump(150);
    check!(
        host.dock.borrow_mut().audio_status(id).percent == 5,
        "one volume step should equal 5%"
    );

    // A physical XTest wheel event over the production speaker widget must
    // traverse GTK's scroll handler and add exactly one step.
    let sound = host
        .dock
        .borrow()
        .button(id)
        .ok_or("audio button")?
        .sound
        .clone();
    check!(
        sound.is_visible(),
        "speaker control should be visible for audio"
    );
    click_widget(&sound, 4);
    check!(
        wait_until(800, || host.dock.borrow_mut().audio_status(id).percent
            == 10),
        "physical wheel over speaker did not add 5%"
    );

    // Raw-XInput and GTK can describe the same physical scroll. Matching
    // timestamps must de-duplicate it rather than applying 10%.
    host.dock.borrow_mut().audio_volume(id, -100);
    pump(100);
    let (sx, sy) = widget_center(&sound);
    host.dock.borrow_mut().input_delta(sx, sy, 0.0, -1.0, 4242);
    let duplicate = ScrollInfo {
        direction: gdk::ScrollDirection::Smooth,
        delta_x: 0.0,
        delta_y: -1.0,
        time: 4242,
        x: 0.0,
        y: 0.0,
        x_root: sx as f64,
        y_root: sy as f64,
    };
    host.dock.borrow_mut().audio_scroll(id, &duplicate);
    pump(150);
    check!(
        host.dock.borrow_mut().audio_status(id).percent == 5,
        "raw/GTK duplicate scroll was applied more than once"
    );

    host.dock.borrow_mut().audio_volume(id, 100);
    pump(150);
    check!(
        host.dock.borrow_mut().audio_status(id).percent == 200,
        "volume ceiling should clamp to 200%"
    );
    host.dock.borrow_mut().audio_volume(id, -100);
    pump(150);
    check!(
        host.dock.borrow_mut().audio_status(id).percent == 0,
        "volume should return to 0%"
    );

    // Volume bubble timeout is a production Timer regression check: it must
    // hide cleanly without leaving a stale GLib SourceId.
    host.dock.borrow_mut().volume_bubble(id);
    check!(
        host.dock
            .borrow()
            .bubble
            .as_ref()
            .is_some_and(|bubble| bubble.is_visible()),
        "volume bubble should be visible"
    );
    pump(1400);
    check!(
        host.dock
            .borrow()
            .bubble
            .as_ref()
            .is_none_or(|bubble| !bubble.is_visible()),
        "volume bubble should auto-hide"
    );

    raw::terminate(pulse_pid);
    pump(800);
    check!(
        !host.dock.borrow_mut().audio_status(id).present,
        "audio status should clear after server loss"
    );
    let _ = stream.kill();
    let _ = stream.wait();

    let mut pulse = Command::new("pipewire-pulse")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("restart pipewire-pulse: {e}"))?;
    pump(1000);
    let status = Command::new("pactl")
        .args([
            "load-module",
            "module-null-sink",
            "sink_name=zero-dock-recovery",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| e.to_string())?;
    check!(status.success(), "failed to recreate private null sink");
    let status = Command::new("pactl")
        .args(["set-default-sink", "zero-dock-recovery"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| e.to_string())?;
    check!(status.success(), "failed to set private default sink");

    stream = start_pacat()?;
    check!(
        wait_until(5500, || host.dock.borrow_mut().audio_status(id).present),
        "audio did not recover after server restart"
    );
    let before = host.dock.borrow_mut().audio_status(id).muted;
    host.dock.borrow_mut().audio_mute(id);
    pump(200);
    let after = host.dock.borrow_mut().audio_status(id).muted;
    check!(after != before, "mute control failed after audio recovery");

    let _ = stream.kill();
    let _ = stream.wait();
    let _ = pulse.kill();
    let _ = pulse.wait();
    window.close();
    host.close();
    let _ = fs::remove_dir_all(dir);
    Ok(())
}

fn scenario_external_wrappers() -> TestResult {
    let wrapper = Path::new("/usr/lib/xfce4/panel/wrapper-2.0");
    if !wrapper.exists() {
        eprintln!("SKIP external-wrappers: wrapper-2.0 not installed");
        return Ok(());
    }
    let build = std::env::var("ZERO_DOCK_TEST_BUILD").unwrap_or_else(|_| ".".to_string());
    let library = Path::new(&build).join("libzero-dock.so");
    check!(
        library.exists(),
        "plugin library missing: {}",
        library.display()
    );

    let f1 = fixture("External Red", "ExternalRed");
    let f2 = fixture("External Green", "ExternalGreen");
    color_icon(&f1, 0xdd3333ff);
    color_icon(&f2, 0x33dd33ff);

    let host = gtk::Window::new(gtk::WindowType::Toplevel);
    host.set_skip_taskbar_hint(true);
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 3);
    let s1 = gtk::Socket::new();
    let s2 = gtk::Socket::new();
    s1.set_size_request(520, 48);
    s2.set_size_request(520, 48);
    box_.pack_start(&s1, false, false, 0);
    box_.pack_start(&s2, false, false, 0);
    host.set_child(Some(&box_));
    host.move_(10, 10);
    host.show_all();
    pump(100);

    let args = |id: &str, socket: &gtk::Socket| {
        vec![
            library.to_string_lossy().to_string(),
            id.to_string(),
            socket.id().to_string(),
            "zero-dock".to_string(),
            "Zero Dock Test".to_string(),
            "Rust external wrapper test".to_string(),
        ]
    };
    let mut one = Command::new(wrapper)
        .env_remove("G_DEBUG")
        .args(args("9091", &s1))
        .spawn()
        .map_err(|e| e.to_string())?;
    let mut two = Command::new(wrapper)
        .env_remove("G_DEBUG")
        .args(args("9092", &s2))
        .spawn()
        .map_err(|e| e.to_string())?;
    check!(
        wait_until(2500, || s1.plug_window().is_some()
            && s2.plug_window().is_some()),
        "real wrapper processes did not embed"
    );
    check!(one.id() != two.id(), "wrappers must be separate processes");

    // Exercise a real XTest pointer move/click inside one embedded wrapper.
    let (ox, oy) = s1.window().expect("socket window").root_origin();
    raw::move_pointer(ox + 30, oy + 24);
    raw::click_pointer(1);
    pump(150);

    let _ = one.kill();
    let _ = one.wait();
    pump(300);
    check!(
        s2.plug_window().is_some(),
        "peer wrapper died when first wrapper exited"
    );
    check!(
        two.try_wait().map_err(|e| e.to_string())?.is_none(),
        "peer wrapper exited"
    );

    let _ = two.kill();
    let _ = two.wait();
    host.close();
    f1.close();
    f2.close();
    pump(100);
    Ok(())
}

fn center_rgb(pixbuf: &gdk_pixbuf::Pixbuf) -> (u8, u8, u8) {
    let bytes = pixbuf.read_pixel_bytes();
    let pixels = bytes.as_ref();
    let x = pixbuf.width() / 2;
    let y = pixbuf.height() / 2;
    let offset =
        y as usize * pixbuf.rowstride() as usize + x as usize * pixbuf.n_channels() as usize;
    (pixels[offset], pixels[offset + 1], pixels[offset + 2])
}

fn scenario_preview_pixels() -> TestResult {
    let dir = temp_dir("preview-pixels");
    let rc = dir.join("dock.rc");
    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.set_title("Zero Dock live preview pixels");
    window.set_default_size(320, 220);
    let area = gtk::DrawingArea::new();
    let blue = Rc::new(Cell::new(false));
    let paint_blue = blue.clone();
    area.connect_draw(move |_widget, cr| {
        if paint_blue.get() {
            cr.set_source_rgb(0.08, 0.12, 0.90);
        } else {
            cr.set_source_rgb(0.90, 0.12, 0.08);
        }
        let _ = cr.paint();
        glib::Propagation::Proceed
    });
    window.set_child(Some(&area));
    window.move_(660, 60);
    window.realize();
    set_wm_class(&window, "ZeroDockPreviewPixels");
    window.show_all();
    pump(300);

    let host = Host::new(&rc);
    pump(250);
    host.refresh();
    let id = button_for(&host, &window).ok_or("preview-pixel fixture was not tracked")?;

    let red = host
        .dock
        .borrow_mut()
        .capture(id)
        .ok_or("initial red capture failed")?;
    let (r1, _g1, b1) = center_rgb(&red);
    check!(
        r1 > b1,
        "first compositor capture should be red-dominant: r={r1} b={b1}"
    );

    blue.set(true);
    area.queue_draw();
    pump(350);
    let frame = host
        .dock
        .borrow_mut()
        .capture(id)
        .ok_or("updated blue capture failed")?;
    let (r2, _g2, b2) = center_rgb(&frame);
    check!(
        b2 > r2,
        "second compositor capture should be blue-dominant: r={r2} b={b2}"
    );
    check!(
        (r1, b1) != (r2, b2),
        "captured frame pixels did not change after repaint"
    );

    window.close();
    pump(200);
    host.close();
    let _ = fs::remove_dir_all(dir);
    Ok(())
}

fn spawn_real_app(kind: &str, dir: &Path) -> Result<Child, String> {
    let mut command = match kind {
        "chrome" => {
            let mut command = Command::new("/usr/bin/google-chrome-stable");
            command.args([
                format!("--user-data-dir={}/profile", dir.to_string_lossy()),
                "--no-first-run".to_string(),
                "--no-default-browser-check".to_string(),
                "--disable-background-networking".to_string(),
                "--disable-component-update".to_string(),
                "--disable-sync".to_string(),
                "--disable-extensions".to_string(),
                "--disable-gpu".to_string(),
                "--password-store=basic".to_string(),
                "--new-window".to_string(),
                "about:blank".to_string(),
            ]);
            command
        }
        "thunar" => {
            let mut command = Command::new("/usr/bin/thunar");
            command.arg(dir);
            command
        }
        _ => return Err(format!("unknown real app fixture: {kind}")),
    };
    command
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("spawn {kind}: {e}"))
}

fn stop_child(child: &mut Child) {
    if child.try_wait().ok().flatten().is_none() {
        let _ = child.kill();
        let _ = child.wait();
    }
}

fn scenario_real_apps() -> TestResult {
    let candidates = [
        (
            "/usr/share/applications/google-chrome.desktop",
            "/usr/bin/google-chrome-stable",
            "Google Chrome",
            "chrome",
        ),
        (
            "/usr/share/applications/thunar.desktop",
            "/usr/bin/thunar",
            "Thunar",
            "thunar",
        ),
    ];
    let available: Vec<_> = candidates
        .into_iter()
        .filter(|(desktop, executable, _, _)| {
            Path::new(desktop).is_file() && Path::new(executable).is_file()
        })
        .collect();
    if available.is_empty() {
        eprintln!("SKIP real-apps: Chrome/Thunar are not installed");
        return Ok(());
    }

    for (desktop, _executable, label, kind) in available {
        eprintln!("real-apps: {label}");
        let dir = temp_dir(kind);
        let rc = dir.join("dock.rc");
        let host = Host::new(&rc);
        let pin = host
            .dock
            .borrow_mut()
            .add_pin(desktop)
            .ok_or_else(|| format!("{label}: pin could not load"))?;
        check!(
            host.dock.borrow().button(pin).is_some_and(|button| {
                button.pinned && button.app.is_some() && button.window.is_none()
            }),
            "{label}: expected idle application pin"
        );

        let mut first = spawn_real_app(kind, &dir)?;
        check!(
            wait_until(8000, || {
                host.dock.borrow_mut().refresh();
                host.dock
                    .borrow()
                    .button(pin)
                    .is_some_and(|button| button.window.is_some())
            }),
            "{label}: first real window did not bind to pin"
        );
        check!(
            window_count(&host) == 1,
            "{label}: expected exactly one tracked window after first launch"
        );

        host.dock.borrow_mut().minimize(pin);
        check!(
            wait_until(1800, || {
                host.dock
                    .borrow()
                    .button(pin)
                    .and_then(|button| button.window.as_ref())
                    .is_some_and(|window| XfwWindow::new(window).is_minimized())
            }),
            "{label}: minimize failed"
        );
        host.dock.borrow_mut().activate(pin);
        check!(
            wait_until(1800, || {
                host.dock
                    .borrow()
                    .button(pin)
                    .and_then(|button| button.window.as_ref())
                    .is_some_and(|window| !XfwWindow::new(window).is_minimized())
            }),
            "{label}: restore failed"
        );

        let mut second = spawn_real_app(kind, &dir)?;
        check!(
            wait_until(8000, || {
                host.dock.borrow_mut().refresh();
                window_count(&host) >= 2
            }),
            "{label}: second real window did not appear"
        );
        check!(
            window_count(&host) == 2,
            "{label}: expected two tracked windows"
        );

        // Closing the pinned window must transfer the sibling into the same
        // persistent pin, then the final close restores an idle launcher.
        host.dock.borrow_mut().close_window(pin);
        check!(
            wait_until(2500, || {
                host.dock.borrow_mut().refresh();
                window_count(&host) == 1
                    && host
                        .dock
                        .borrow()
                        .button(pin)
                        .is_some_and(|button| button.window.is_some())
            }),
            "{label}: sibling window did not transfer into pin"
        );

        host.dock.borrow_mut().close_window(pin);
        check!(
            wait_until(2500, || {
                host.dock.borrow_mut().refresh();
                window_count(&host) == 0
                    && host
                        .dock
                        .borrow()
                        .button(pin)
                        .is_some_and(|button| button.pinned && button.window.is_none())
            }),
            "{label}: final close did not restore idle launcher"
        );

        stop_child(&mut first);
        stop_child(&mut second);
        host.close();
        let _ = fs::remove_dir_all(dir);
    }
    Ok(())
}

type Scenario = (&'static str, fn() -> TestResult);

fn scenarios() -> Vec<Scenario> {
    vec![
        ("native-all", scenario_native_all),
        ("external-wrappers", scenario_external_wrappers),
        ("pinned-lifecycle", scenario_pinned_lifecycle),
        ("launch-feedback", scenario_launch_feedback),
        ("identity-workspaces", scenario_identity_workspaces),
        ("improvements", scenario_improvements),
        ("stress", scenario_stress),
        ("audio-recovery", scenario_audio_recovery),
        ("preview-pixels", scenario_preview_pixels),
        ("real-apps", scenario_real_apps),
        ("xi-probe", scenario_xi_probe),
    ]
}

/// Diagnostic probe: inject XTest motion and button events on a private X
/// connection with XI2 button masks selected on the root window, then print
/// every event the server delivers. Never fails; prints evidence only.
fn scenario_xi_probe() -> TestResult {
    unsafe {
        use x11::xinput2;
        let display = xlib::XOpenDisplay(std::ptr::null());
        check!(!display.is_null(), "probe: no display");
        let mut opcode: c_int = 0;
        let mut event_code: c_int = 0;
        let mut error: c_int = 0;
        let mut major: c_int = 2;
        let mut minor: c_int = 2;
        let name = b"XInputExtension\0";
        check!(
            xlib::XQueryExtension(
                display,
                name.as_ptr() as *const c_char,
                &mut opcode,
                &mut event_code,
                &mut error
            ) != xlib::False,
            "probe: no XInputExtension"
        );
        check!(
            xinput2::XIQueryVersion(display, &mut major, &mut minor) == 0,
            "probe: XIQueryVersion failed"
        );
        eprintln!("[probe] XI version {major}.{minor} opcode={opcode}");

        let root = xlib::XDefaultRootWindow(display);
        let mask_len = ximask_len_probe(xinput2::XI_LASTEVENT);
        let mut mask = vec![0u8; mask_len];
        xinput2::XISetMask(&mut mask, xinput2::XI_ButtonPress);
        xinput2::XISetMask(&mut mask, xinput2::XI_ButtonRelease);
        xinput2::XISetMask(&mut mask, xinput2::XI_Motion);
        xinput2::XISetMask(&mut mask, xinput2::XI_RawButtonPress);
        xinput2::XISetMask(&mut mask, xinput2::XI_RawButtonRelease);
        xinput2::XISetMask(&mut mask, xinput2::XI_RawMotion);
        let mut masks = [
            xinput2::XIEventMask {
                deviceid: xinput2::XIAllMasterDevices,
                mask_len: mask.len() as c_int,
                mask: mask.as_mut_ptr(),
            },
            xinput2::XIEventMask {
                deviceid: xinput2::XIAllDevices,
                mask_len: mask.len() as c_int,
                mask: mask.as_mut_ptr(),
            },
        ];
        check!(
            xinput2::XISelectEvents(display, root, masks.as_mut_ptr(), masks.len() as c_int) as i32
                == 0_i32,
            "probe: XISelectEvents failed"
        );
        xlib::XFlush(display);

        // Core button press mask on a probe window too, for the legacy path.
        let probe_window =
            xlib::XCreateSimpleWindow(display, root, 100, 100, 200, 200, 1, 0, 0xffffff);
        xlib::XSelectInput(
            display,
            probe_window,
            xlib::ButtonPressMask | xlib::ButtonReleaseMask | xlib::ButtonMotionMask,
        );
        xlib::XMapWindow(display, probe_window);
        xlib::XFlush(display);

        xtest::XTestFakeMotionEvent(display, -1, 200, 200, 0);
        xlib::XFlush(display);
        xtest::XTestFakeButtonEvent(display, 1, 1, 0);
        xtest::XTestFakeButtonEvent(display, 1, 0, 0);
        xtest::XTestFakeButtonEvent(display, 4, 1, 0);
        xtest::XTestFakeButtonEvent(display, 4, 0, 0);
        xlib::XSync(display, xlib::False);

        let deadline = Instant::now() + Duration::from_millis(700);
        let mut seen = Vec::new();
        while Instant::now() < deadline {
            while xlib::XPending(display) > 0 {
                let mut event: xlib::XEvent = std::mem::zeroed();
                xlib::XNextEvent(display, &mut event);
                if event.type_ == xlib::GenericEvent {
                    if xlib::XGetEventData(display, &mut event.generic_event_cookie) != 0 {
                        let data = event.generic_event_cookie.data as *const xinput2::XIRawEvent;
                        let detail = if data.is_null() { -1 } else { (*data).detail };
                        seen.push(format!("XI2 evtype={} detail={}", (*data).evtype, detail));
                        xlib::XFreeEventData(display, &mut event.generic_event_cookie);
                    }
                } else {
                    seen.push(format!("CORE type={}", event.type_));
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        eprintln!("[probe] events: {:?}", seen);
        xlib::XCloseDisplay(display);
    }
    Ok(())
}

const fn ximask_len_probe(event: i32) -> usize {
    ((event + 7) >> 3) as usize
}

fn run_one(name: &str, test: fn() -> TestResult) -> bool {
    eprintln!("=== RUN {name} ===");
    let started = Instant::now();
    match std::panic::catch_unwind(test) {
        Ok(Ok(())) => {
            eprintln!(
                "=== PASS {name} ({:.2}s) ===",
                started.elapsed().as_secs_f64()
            );
            true
        }
        Ok(Err(error)) => {
            eprintln!("=== FAIL {name}: {error} ===");
            false
        }
        Err(_) => {
            eprintln!("=== PANIC {name} ===");
            false
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--fixture-process") {
        let class = args.get(2).map(String::as_str).unwrap_or("ZeroDockLaunch");
        let title = args
            .get(3)
            .map(String::as_str)
            .unwrap_or("Zero Dock Fixture");
        let seconds = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(5);
        fixture_process(class, title, seconds);
        return;
    }

    gtk::init().expect("gtk init");
    let requested = args
        .windows(2)
        .find(|pair| pair[0] == "--scenario")
        .map(|pair| pair[1].clone());

    let mut ok = true;
    for (name, test) in scenarios() {
        if requested.as_deref().is_some_and(|want| want != name) {
            continue;
        }
        ok &= run_one(name, test);
        pump(100);
    }
    if let Some(name) = requested {
        if !scenarios().iter().any(|(candidate, _)| *candidate == name) {
            eprintln!("unknown scenario: {name}");
            std::process::exit(2);
        }
    }
    if !ok {
        std::process::exit(1);
    }
}
