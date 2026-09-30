# Local testing

No GitHub CI or Actions workflows are used. Checks run on the developer's machine.

## Build and unit checks

```sh
meson setup build-dev --prefix=/usr --libdir=lib -Dtests=true --werror
meson compile -C build-dev
meson test -C build-dev --print-errorlogs
clang-format --dry-run --Werror src/*.c src/*.h tests/*.c
python -m py_compile tests/run-isolated.py tools/make-dist.py
```

Unit checks cover process ancestry, application-name matching, pin persistence filtering/order and XFCE registration metadata.

## Isolated desktop integration

```sh
python tests/run-isolated.py build-dev
python tests/run-isolated.py build-dev --filter /integration/native-all
```

Requires Xvfb, xfwm4, D-Bus, `pacat`, Python and a running PulseAudio-compatible audio server. It creates a private X display, session bus and configuration directory. It terminates only processes it starts. Audio tests create a zero-data stream and alter only that stream.

Coverage includes independent windows, native wrapper embedding, real XTest drag/click events, launcher persistence, preview capture/cache, menus, workspace state, application audio limits, scroll accumulation, XInput2 hit testing, duplicate suppression and disposal. The external-wrapper fixture has no complete panel D-Bus service, so ProviderSignal ServiceUnknown warnings there are expected fixture limitations.

## Sanitizers

```sh
meson setup build-asan --prefix=/usr --libdir=lib -Dtests=true   -Db_sanitize=address,undefined -Db_lundef=false --buildtype=debug
meson compile -C build-asan
ASAN_OPTIONS=detect_leaks=0 python tests/run-isolated.py build-asan   --filter /integration/native-all
```

This checks the internal GTK host. External wrappers with sanitizer preloading can interact with third-party image-loader processes; that path is not claimed as covered. GTK process-global leak reporting is disabled, so this is not a zero-leak guarantee.

## Optional real desktop check

```sh
build-dev/zero-dock-real-preview
```

This explicitly opens its own colored fixture and test host on your current desktop and verifies red-to-blue captured pixels. It closes only its own windows. Real mouse behavior, multi-monitor placement, mixed DPI, application-specific audio matching and long-running use still need manual validation; automated XTest events are not a substitute for physical-device checks.

Review [VERIFICATION.md](../VERIFICATION.md) for the initial release's evidence and limits. Do not publish personal desktop captures or complete session logs with reports.
