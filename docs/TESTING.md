# Local testing

No GitHub CI or Actions workflows are used. Checks run on the developer's machine.

## Build and automated checks

```sh
meson setup build-dev --prefix=/usr --libdir=lib -Dtests=true --werror
meson compile -C build-dev
meson test -C build-dev --print-errorlogs
cargo fmt --manifest-path rust/Cargo.toml --check
cargo clippy --manifest-path rust/Cargo.toml --all-targets -- -D warnings
G_DEBUG=fatal-warnings cargo test --manifest-path rust/Cargo.toml --all-targets
python -m py_compile tests/run-isolated.py tools/*.py
```

`meson test` always runs the Rust unit suite (19 tests). When `Xvfb`, `xfwm4` and `dbus-run-session` are installed it also runs the isolated GUI-host smoke test. The unit suite covers process ancestry, application matching and precedence, pin serialization/order, settings validation and round-trip, timer lifecycle, scroll accumulation, atomic private config writes, registration metadata and association helpers.

The unit suite is expected to pass with `G_DEBUG=fatal-warnings`. In particular, a one-shot `Timer` that has already returned `ControlFlow::Break` must be safe to query, clear or drop without producing a stale `g_source_remove()` GLib critical.

## Isolated GUI lifecycle smoke

```sh
python tests/run-isolated.py build-dev
python tests/run-isolated.py build-dev --iterations 10 --seconds 2
```

The harness starts a private D-Bus session, Xvfb display and xfwm4 compositor, then launches the real Rust `zero-dock-test-host` repeatedly with `G_DEBUG=fatal-warnings`. Runs reuse the same private rc file, so construction, config load/save, timer teardown, Xfw/signal teardown and clean process exit are exercised across repeated lifecycles without touching the user's desktop.

The default is three two-second lifecycle runs. `--private-audio` additionally starts disposable PipeWire/Pulse services when those binaries are available. `--scale 2` and `--geometry WIDTHxHEIGHT` cover basic scale/geometry variants.

## Manual GUI test host

```sh
build-dev/zero-dock-test-host [rc-file] [seconds]
G_DEBUG=fatal-warnings build-dev/zero-dock-test-host my.rc 20
```

The host constructs a real `XfcePanelPlugin` object in a plain GTK window — the same object shape the panel wrapper creates. Use the manual path for interactions the smoke harness does not synthesize yet: pin/unpin, preview hover/click, window menus, drag-and-drop, audio controls, workspace changes and application-specific behavior.

## Remaining integration gap

The former C integration binaries (`zero-dock-integration`, `zero-dock-real-preview`, `zero-dock-inspect-apps`) poked directly into C internals and were removed with the Rust rewrite. The isolated Rust host now restores automated construction/disposal and fatal-warning regression coverage, but full behavioral parity is not restored yet. Dedicated Rust scenarios are still needed for pinned-window transfer, launch timeout/retry, identity/workspace rebinding, audio-server reconnection, preview pixel verification and long stress runs.

## Sanitizers

The Rust core builds with the standard toolchain. Address/UB sanitizers can be enabled with nightly Rust and an appropriate `RUSTFLAGS=-Zsanitizer=...` build. Sanitizer runs over the GUI host are useful regression evidence but are not a zero-leak guarantee for GTK/X11 or all third-party image loaders.

## Optional real desktop check

Run `build-dev/zero-dock-test-host` on the current desktop and verify preview capture and real input behavior. Multi-monitor placement, mixed DPI, application-specific audio matching, Steam/Wine/Flatpak identity and long-running suspend/resume behavior still require real-desktop validation.

Review [VERIFICATION.md](../VERIFICATION.md) for the initial release's evidence and limits. Do not publish personal desktop captures or complete session logs with reports.
