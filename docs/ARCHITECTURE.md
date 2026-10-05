# Architecture

Zero Dock is a GTK 3 shared module loaded by XFCE's external `wrapper-2.0` provider. Each installed instance normally gets its own process and configuration. No Python runtime is used by the plugin. The plugin is written entirely in Rust: the crate's cdylib exports the XFCE module entry point (`xfce_panel_module_construct`) directly, so no C is compiled at all.

## Layout

| Path | Responsibility |
|---|---|
| `rust/src/plugin_abi.rs` | XFCE module ABI (`xfce_panel_module_construct` + `realize` hook), gettext domain binding, panic guard |
| `rust/src/lib.rs` | Crate root; `construct_plugin` shared by the ABI and the test host |
| `rust/src/dock.rs` | Instance lifecycle, pin persistence, window reconciliation and panel geometry |
| `rust/src/button.rs` | Icon drawing, interaction, pinning, ordering and scroll accumulation |
| `rust/src/apps.rs` | Desktop identity, launch feedback, process/startup hints and app indexing |
| `rust/src/associations.rs` | Exact class-signature rules and native desktop-file chooser |
| `rust/src/layout.rs` | Bounded icon layout, overflow menu, shared popup events and focus navigation |
| `rust/src/settings.rs` | Validated preferences, native error UI and minimal diagnostics |
| `rust/src/menu.rs` | Native window, workspace, launcher and panel menus |
| `rust/src/preview.rs` | XComposite capture, frame cache, clickable previews and volume bubbles |
| `rust/src/audio.rs` | PulseAudio subscriptions, process matching, mute and volume |
| `rust/src/input.rs` | Separate XInput2 connection, device scroll increments and speaker hit testing |
| `rust/src/ffi_xfce.rs` | Hand-written FFI for libxfce4panel / libxfce4windowing (no Rust bindings exist upstream) |
| `rust/src/ffi_x11.rs` | Xlib / XComposite / XInput2 / GDK-X11 glue and error traps |
| `rust/src/util.rs` | RAII timers (`Timer`), gettext helper, `GList` iteration |
| `rust/src/bin/test_host.rs` | Standalone GUI test host (former `tests/host.c`), linked against the same rlib |

## Ownership model

The module entry point builds the `XfcePanelPlugin` widget with `g_object_new` (the same properties the C macro passed), hooks `realize`, and hands the object to the panel; the plugin object then owns the single strong `Rc<RefCell<Dock>>` via `g_object_set_data_full`. Every signal handler (GTK, Xfw, GLib sources, PulseAudio callbacks) captures only a `Weak`, so no callback can keep the dock alive or touch freed state. Handler ids attached to longer-lived Xfw screen/workspace objects, the application monitor and icon theme are retained and explicitly disconnected during instance disposal, so repeated plugin reloads do not accumulate dead callbacks. Buttons are addressed by stable ids and menus resolve string keys against the live button list — no raw pointers cross callback boundaries. GLib timers are wrapped in a `Timer` RAII type that tracks whether the callback has already returned `Break`; dropping or replacing an already-completed timer therefore never calls `g_source_remove()` with a stale id.

The PulseAudio state machine lives in its own `Rc<RefCell<Audio>>` inside the dock, with a generation counter bumped on every reconnect so callbacks from a replaced connection can never mutate the new connection's buffers. Re-entrant borrows defer through an idle callback instead of aborting.

`unsafe` is confined to the `plugin_abi` module, the `ffi_*` modules and the XComposite capture path; all business logic is safe Rust. X11 calls that can raise asynchronous errors run inside GDK error traps exactly like the C implementation did.

## Behavior notes (unchanged from the C implementation)

libxfce4windowing supplies windows and workspaces. Launchers use GDesktopAppInfo. Audio streams match window process ancestry first, with executable/class hints as a fallback. Shared application audio can affect several window buttons.

A button can own both a persistent pin and a live window. An idle pin binds the first matching window; additional windows get independent buttons. Pinning a running window changes its existing button in place. Unpinning a running button removes only persistence. Closing a pinned window releases its live state and either transfers another matching window, including its cached frame, into that position or leaves an idle launcher. The window table always maps each live window to exactly one button. Window release disconnects callbacks and dismisses dependent menus/previews before rebinding.

Application matching considers both imported pinned entries and installed desktop files. Startup notification IDs and recently launched process ancestry rank above explicit application/desktop IDs, followed by StartupWMClass, executable basenames and fuzzy desktop IDs. Pins win equal-score ties. Desktop file paths distinguish imported entries; sharing an executable alone does not make two entries identical. Class and identity-property changes invalidate matching, as do changes to the application index. Pins remain visible across workspace filters and prefer a current-workspace window; swapping preserves both window buttons and cached frames. Additional window buttons follow the filter.

Launching uses GDesktopAppInfo's manager API with the desktop file's native command and environment. A dock timer animates pending launches and expires them for retry. Idle-pin left clicks are suppressed while pending; explicit new-instance requests remain available. The first matching new window ends the pending state. Failures use a native dialog and unavailable pins remain persisted. Launch callback connections are removed before releasing the launch context, and all pending timers are removed during instance disposal.

Icon caches retain physical-resolution pixels and draw at logical size. Window icon, theme and scale changes invalidate them. Audio updates refresh badges and the preview audio controls without rebuilding icons or application associations. Settings read typed values with defaults and bounds and preserve unrelated keys on save. Gettext uses an instance-independent domain without changing the application's global translation domain.

Capture reads a compositor's redirected frame pixmap and scales it to a bounded thumbnail. Minimized windows retain their last frame in memory. GTK timers control delayed previews, refresh, active-window caching and volume-bubble expiry. Popup placement uses the anchor's screen coordinates and corresponding monitor bounds.

XInput2 uses its own X connection and GLib file-descriptor source, avoiding changes to GTK's event selections. Only scroll values over this instance's visible speaker controls become audio actions. GTK/raw duplicates use timestamps to avoid applying a tick twice. Device topology changes invalidate cached axis metadata.

Instance cleanup disconnects callbacks, removes timer/input sources, destroys popups, disconnects audio and releases window/image references. Drag payload keys are looked up against the local instance's button list; external data is never dereferenced as a pointer.

Manual associations use a SHA-256 of length-prefixed, case-normalized class identifiers, preserving the entire class combination rather than fuzzy names. Each signature maps to a desktop file and is stored in the instance's `Associations` group. Invalid or unavailable entries fall back to automatic recognition. The chooser explains the scope: identical classes share a rule. Rule changes invalidate application matching and reconcile pins without duplicate windows.

Workspace eligibility is separate from icon visibility. `MaxVisible` bounds the number of visible positions, including the overflow control; zero disables the bound. Hidden buttons retain their window and pin state and remain accessible from the overflow menu and window cycling. Menu callbacks resolve copied button keys against the current instance instead of retaining button pointers. Keyboard focus navigation includes the overflow control.

Configuration restore validates the input size, group, pin paths and association entries before writing. The current configuration is saved to a private-permission sibling backup first; atomic replacement precedes runtime reconciliation. Running pinned buttons become ordinary live buttons temporarily, restored pins take over matching windows, and unknown configuration keys remain preserved. Defaults reset preferences while retaining pins and manual rules.

Popup sizes use the monitor at the anchor's screen position, with work-area bounds and a missing-monitor fallback. Monitor/scale changes dismiss stale popups; cached images are fitted to the new bounds. Thumbnail storage is limited to 32 MiB per instance by evicting older frames, keeping the just-captured frame. Long-run tests measure resident memory and idle CPU separately from claims about zero leaks or all hardware combinations.

## Building

`meson setup build && ninja -C build` runs one locked Cargo release build and materializes both `libzero-dock.so` and `zero-dock-test-host` from that shared output. `Cargo.lock` is part of the Meson dependency graph. With `-Dtests=true`, `meson test -C build` runs the 19 Rust unit tests and, when Xvfb/xfwm4/D-Bus helpers are installed, a repeated isolated GUI lifecycle smoke under `G_DEBUG=fatal-warnings`.

Requires: Rust/Cargo, meson >= 0.63, GTK 3.24, libxfce4panel >= 4.20, libxfce4windowing >= 4.20, libpulse (with glib mainloop), X11/XComposite/XInput2.
