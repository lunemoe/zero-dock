# Changelog

## 0.3.0 — 2026-10-01

- Add persistent, exact window-class associations with a desktop-file chooser and per-window rule removal. Manual rules take priority; indistinguishable classes share a rule.
- Limit visible icons with an overflow menu while preserving separate window buttons, workspace filtering, saved pin order and access to hidden windows.
- Add preview close controls, workspace labels, complete title tooltips and monitor-aware thumbnail and popup limits.
- Use GTK theme colors for popup, audio badge, active indicators and focus outlines. Add focus navigation and configurable left/middle click and window scrolling.
- Back up and restore pinned apps, associations and preferences through native file choosers. Validate imports and automatically preserve the current configuration before replacement; retain running windows.
- Bound thumbnail caches to 32 MiB per instance and evict older frames under pressure.
- Extend isolated testing with installed Chrome/Thunar window lifecycles, disposable audio server recovery, scale/theme checks and configurable stress runs; add read-only desktop association inspection.

## 0.2.0 — 2026-10-01

- Reuse pinned icons for the first matching window, keeping additional windows separate and restoring the launcher after the last close.
- Pin existing windows in place; unpin running applications without removing their window buttons.
- Match imported desktop entries alongside installed applications and rebind buttons when window classes change.
- Preserve previews, desktop actions, audio controls and saved order on running pinned icons; deduplicate pins loaded from configuration.
- Add an isolated regression test that launches actual fixture processes and checks the pin/window lifecycle and changing application metadata.
- Show launch progress, suppress repeated left clicks during startup, allow timeout/retry, and display native failure messages.
- Use application IDs, startup notification IDs and launched process ancestry alongside window classes; watch late identity properties and cache matching results.
- Prefer the current workspace's window at a fixed position and add an application window-selection menu.
- Preserve unavailable launchers with a warning and keep unknown configuration keys when saving.
- Cache icons with theme, icon and scale invalidation; update audio controls separately from window rebuilding.
- Add bounded preview/launch settings, restore defaults, minimal diagnostic export and English translations.
- Centralize the version, include new source files in release archives, and add instance reload and generated-file cleanup tools.

## 0.1.0 — 2026-10-01

First public release.

- Native C/GTK 3 XFCE external panel plugin for X11.
- Independent icon-only window buttons and per-instance pinned launchers.
- Window/workspace menus, application desktop actions and drag reordering.
- Clickable XComposite previews and in-memory minimized-window frames.
- PulseAudio/PipeWire-Pulse application mute and volume controls.
- XInput2 smooth-wheel handling, fractional accumulation and duplicate-event suppression.
- Horizontal/vertical layout, reserved slots and preview-aware auto-hide.
- Local unit, isolated integration and sanitizer validation tools.

This is an early release. See [verification scope](VERIFICATION.md) for tested behavior and remaining coverage.
