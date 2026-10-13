# Local testing

No GitHub CI or Actions workflows are used. Checks run on the developer's machine.

## Build and automated checks

```sh
meson setup build-dev --prefix=/usr --libdir=lib -Dtests=true --werror
meson compile -C build-dev
meson test -C build-dev --print-errorlogs
cargo fmt --manifest-path rust/Cargo.toml --check
cargo clippy --locked --manifest-path rust/Cargo.toml --all-targets --features test-harness -- -D warnings
G_DEBUG=fatal-warnings cargo test --locked --manifest-path rust/Cargo.toml --all-targets --features test-harness
python -m py_compile tests/*.py tools/*.py
```

`meson test` runs 24 Rust unit tests. With the installed X11/audio helpers it also runs lifecycle smoke, seven core behavior scenarios, scale-2 checks, window stress, external wrappers and private audio recovery. Unit coverage includes adaptive capacity boundaries and migration of old fixed-length keys in addition to matching, persistence, timers and scrolling. All harnesses isolate config, cache, data and D-Bus state; audio suites use disposable services and test streams.

The unit suite is expected to pass with `G_DEBUG=fatal-warnings`. In particular, a one-shot `Timer` that has already returned `ControlFlow::Break` must be safe to query, clear or drop without producing a stale `g_source_remove()` GLib critical.

## Isolated GUI lifecycle smoke

```sh
python tests/run-isolated.py build-dev
python tests/run-isolated.py build-dev --iterations 10 --seconds 2
```

The harness starts a private D-Bus session, Xvfb display and xfwm4 compositor, then launches the real Rust `zero-dock-test-host` repeatedly with `G_DEBUG=fatal-warnings`. Runs reuse the same private rc file, so construction, config load/save, timer teardown, Xfw/signal teardown and clean process exit are exercised across repeated lifecycles without touching the user's desktop.

Both harnesses force the private session to X11 (Wayland session variables are cleared; `GDK_BACKEND=x11` and `XDG_SESSION_TYPE=x11` are set), so they behave identically when started from a Wayland desktop; the Chrome fixture additionally passes `--ozone-platform=x11`.

The default is three two-second lifecycle runs. `--private-audio` additionally starts disposable PipeWire/Pulse services when those binaries are available. `--scale 2` and `--geometry WIDTHxHEIGHT` cover basic scale/geometry variants.

## Manual GUI test host

```sh
build-dev/zero-dock-test-host [rc-file] [seconds]
G_DEBUG=fatal-warnings build-dev/zero-dock-test-host my.rc 20
```

The host constructs a real `XfcePanelPlugin` object in a plain GTK window — the same object shape the panel wrapper creates. It is useful for exploratory interaction and application-specific checks; the behavior harness below already synthesizes the main window, drag, preview, menu, audio and workspace flows.

## Rust behavior and sustained stress

```sh
python tests/run-integration.py build-dev --suite core --private-audio
python tests/run-integration.py build-dev --scenario adaptive-reliability --scale 2 --geometry 2048x1536 --private-audio
python tests/run-integration.py build-dev --suite audio --private-audio
python tests/run-integration.py build-dev --suite real-apps
python tests/run-integration.py build-dev --suite stress --stress-seconds 60 --private-audio
# Optional long run; this is an entry point, not evidence of a completed 24-hour test:
python tests/run-integration.py build-dev --suite stress --stress-seconds 86400 --private-audio
```

Core scenarios cover native controls and real drag input, pin/window transfer, launch feedback, identity/workspaces, manual rules and config restoration, changing preview pixels, and adaptive/reliability regressions. The latter shrinks and expands actual GTK allocations horizontally and vertically, covers a single-position overflow, tests hidden-row context actions, preview middle-click settings, native app selection and launcher replacement, rejects 513-rule imports without changing the file, preserves all 512 valid rules, and verifies failed writes recover through automatic retry. `ZERO_DOCK_TEST_IMAGES=/tmp/zero-dock-images` optionally saves synthetic UI snapshots from this scenario for visual review.

The stress duration is 0–86400 seconds; zero uses `--stress-cycles` instead. The runner adjusts its timeout to the chosen duration and reports cycle/window counts, warm/final RSS and idle CPU. Real-app scenarios use temporary Chrome profiles and isolated Thunar windows. These checks do not establish all browser profiles, Flatpak/Steam/Wine compatibility, hardware mixed DPI or suspend/resume behavior.

## Sanitizers

The Rust core builds with the standard toolchain. Address/UB sanitizers can be enabled with nightly Rust and an appropriate `RUSTFLAGS=-Zsanitizer=...` build. Sanitizer runs over the GUI host are useful regression evidence but are not a zero-leak guarantee for GTK/X11 or all third-party image loaders.

## Optional real desktop check

Run `build-dev/zero-dock-test-host` on the current desktop and verify preview capture and real input behavior. Multi-monitor placement, mixed DPI, application-specific audio matching, Steam/Wine/Flatpak identity and long-running suspend/resume behavior still require real-desktop validation.

Review [VERIFICATION.md](../VERIFICATION.md) for the initial release's evidence and limits. Do not publish personal desktop captures or complete session logs with reports.
