# Local testing

No GitHub CI or Actions workflows are used. Checks run on the developer's machine.

## Build and unit checks

```sh
meson setup build-dev --prefix=/usr --libdir=lib -Dtests=true --werror
meson compile -C build-dev
meson test -C build-dev --print-errorlogs
clang-format --dry-run --Werror src/*.c src/*.h tests/*.c
python -m py_compile tests/run-isolated.py tools/*.py
```

Unit checks cover process ancestry, application-name matching, desktop-entry identity and match precedence, pin persistence filtering/order and XFCE registration metadata.

## Isolated desktop integration

```sh
python tests/run-isolated.py build-dev
python tests/run-isolated.py build-dev --filter /integration/native-all
python tests/run-isolated.py build-dev --filter /integration/pinned-lifecycle
python tests/run-isolated.py build-dev --filter /integration/launch-feedback
python tests/run-isolated.py build-dev --filter /integration/identity-workspaces
```

Requires Xvfb, xfwm4, D-Bus, `pacat`, Python and a running PulseAudio-compatible audio server. It creates a private X display, session bus and configuration directory. It terminates only processes it starts. Audio tests create a zero-data stream and alter only that stream.

Coverage includes independent windows, native wrapper embedding, real XTest drag/click events, launcher persistence, preview capture/cache, menus, workspace state, application audio limits, scroll accumulation, XInput2 hit testing, duplicate suppression and disposal. The external-wrapper fixture has no complete panel D-Bus service, so ProviderSignal ServiceUnknown warnings there are expected fixture limitations.

The pinned-lifecycle fixture launches separate GTK processes through a real desktop entry. It checks that one window reuses the original pin widget, left click minimizes/restores it, middle click creates an independent second window, closing the first transfers the remaining window and cached frame, the final close restores an idle launcher, unpinning keeps a live window, and duplicate pins/order are handled on reload. It also changes WM_CLASS to check rebinding and unmatched windows. This does not establish compatibility with every real application's startup metadata.

Launch feedback tests exercise a slow child process, repeated clicks, no-window timeout/retry, failed working-directory startup, missing-file persistence, native preference controls and reset, minimal diagnostic output, icon caching/invalidation and disposal during startup. Identity/workspace tests change GTK and desktop application properties on a live window with shared WM_CLASS, then move and activate windows between workspaces and select them through the application menu.

## Sanitizers

```sh
meson setup build-asan --prefix=/usr --libdir=lib -Dtests=true   -Db_sanitize=address,undefined -Db_lundef=false --buildtype=debug
meson compile -C build-asan
ASAN_OPTIONS=detect_leaks=0 python tests/run-isolated.py build-asan   --filter /integration/native-all
ASAN_OPTIONS=detect_leaks=0 UBSAN_OPTIONS=halt_on_error=1 python tests/run-isolated.py build-asan --filter /integration/pinned-lifecycle
ASAN_OPTIONS=detect_leaks=0 UBSAN_OPTIONS=halt_on_error=1 python tests/run-isolated.py build-asan --filter /integration/launch-feedback
ASAN_OPTIONS=detect_leaks=0 UBSAN_OPTIONS=halt_on_error=1 python tests/run-isolated.py build-asan --filter /integration/identity-workspaces
```

This checks the internal GTK host. External wrappers with sanitizer preloading can interact with third-party image-loader processes; that path is not claimed as covered. GTK process-global leak reporting is disabled, so this is not a zero-leak guarantee.

## Optional real desktop check

```sh
build-dev/zero-dock-real-preview
```

This explicitly opens its own colored fixture and test host on your current desktop and verifies red-to-blue captured pixels. It closes only its own windows. Real mouse behavior, multi-monitor placement, mixed DPI, application-specific audio matching and long-running use still need manual validation; automated XTest events are not a substitute for physical-device checks.

Review [VERIFICATION.md](../VERIFICATION.md) for the initial release's evidence and limits. Do not publish personal desktop captures or complete session logs with reports.

## 0.3.0 extended checks

```sh
python tests/run-isolated.py build-dev --filter /integration/improvements
python tests/run-isolated.py build-dev --filter /integration/improvements --scale 2 --geometry 2048x1536
python tests/run-isolated.py build-dev --filter /integration/audio-recovery --private-audio
python tests/run-isolated.py build-dev --filter /integration/real-apps --real-apps
python tests/run-isolated.py build-dev --filter /integration/stress --soak-seconds 180
```

The standard suite runs six integration paths and skips three optional paths until explicitly enabled. Improvements cover rule precedence/reload, overflow selection and single-slot limits, keyboard focus, click settings, configuration round-trip and rejected-file preservation, preview close/rebinding, cache eviction, both popup orientations, monitor invalidation and Adwaita light/dark theme colors.

`--private-audio` requires PipeWire, pipewire-pulse, WirePlumber and pactl. It starts private runtime sockets, a null sink and WirePlumber's policy-only profile; it does not create hardware monitors or restart the user's services. Recovery kills and replaces only these private pulse servers twice, checking stale stream removal, reconnection and mute control. The test process group is cleaned up after completion or failure.

`--real-apps` uses the installed Chrome and Thunar desktop entries. Chrome uses a disposable profile; Thunar runs on the private bus and display. Tests open two windows, exercise pin reuse, minimize/restore and close/transfer. This does not test Steam games, Wine, Flatpak apps or every browser profile. Use `build-dev/zero-dock-inspect-apps` for a read-only count of current window associations; it performs no window actions and omits titles/captures.

The stress path repeatedly creates/closes 32 windows and updates icons, audio controls and captured frames, then measures idle CPU for ten seconds. Duration is configurable up to 86400 seconds for a separate 24-hour run. Short runs are regression evidence, not completed 24-hour or suspend/resume verification. Uniform scale-2 Xvfb tests are not a physical mixed-DPI or monitor-hotplug test.

ASan/UBSan also cover the improvements and private audio recovery paths:

```sh
ASAN_OPTIONS=detect_leaks=0 UBSAN_OPTIONS=halt_on_error=1 python tests/run-isolated.py build-asan --filter /integration/improvements
ASAN_OPTIONS=detect_leaks=0 UBSAN_OPTIONS=halt_on_error=1 python tests/run-isolated.py build-asan --filter /integration/audio-recovery --private-audio
```
