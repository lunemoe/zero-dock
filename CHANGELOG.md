# Changelog

## 0.3.3 — 2026-10-13

- Complete same-instance icon reordering locally on release, including gaps between icons, and release native input grabs immediately. Avoid XFCE XEmbed drop completion races that could leave the dock unresponsive until GTK's drop timeout.
- Replay drag-end callbacks interrupted by a dock update, clearing the drag state so clicks and previews keep working after cancellation. Ignore deferred completion from older drags.
- Stop dragging an icon from stuttering: pointer motion during a drag now repaints only the buttons carrying the drop insertion marker instead of running the whole dock refresh (layout, tooltips, per-button geometry and the audio matching pass) for every motion event. On a dock with 13 icons and 5 audio streams this drops the per-event cost from ~11.5 ms to ~0.03 ms and the CPU time of a scripted 90-event drag by ~5.5x.
- Recompute audio ownership only when the stream list or the button/window mapping changes, instead of on every refresh.
- Cover the drag insertion marker (tracked button, leading/trailing half, cleared on leave and drop) in the native integration scenario.

## 0.3.2 — 2026-10-13

- Fix automatic XFCE panels collapsing Zero Dock into a single overflow button after startup or a panel restart. Publish content-derived XEmbed hints for automatic panels and retain a small minimum for fixed panels; bound content by the monitor's space after other panel items.
- Add a real XFCE panel regression for startup, window-driven growth/shrinkage, fixed-length overflow and vertical orientation alongside external-wrapper checks.

## 0.3.1 — 2026-10-13

- Adapt the icon area to content and actual panel allocation, with automatic overflow and expansion in both orientations; migrate old reserved-slot and icon-limit settings.
- Fix negative window-cycle indices for multi-step smooth scrolling and retain full actions and status for hidden windows.
- Accept desktop-file drops across the plugin and add searchable native app selection, pin/rule management and in-place launcher replacement.
- Report unsaved configuration changes, retry writes automatically or manually, and propagate association save failures.
- Reject invalid and excessive restores before replacing configuration; retain all 512 valid association entries.
- Apply middle-click preferences to previews and extend privacy-preserving runtime diagnostics.
- Add adaptive/reliability regression scenarios, UI snapshot capture and duration-based stress up to 24 hours; refresh the Rust testing documentation and English translations.
- Scope instance reloads to their owning panel session and respect XDG configuration and state directories.

## 0.3.0 — 2026-10-12

- Rewrite the panel plugin from C to Rust. The cdylib exports the XFCE module entry point directly; `unsafe` is confined to five FFI modules under a crate-root `#![deny(unsafe_code)]`, and every behavior module builds with zero `unsafe`.
- Fix a plugin object reference leak when opening the preferences dialog (the plugin could be finalized while the panel still held it) and restore the screen default icon theme, so application icons follow the user's icon theme again.
- Score launcher-to-window matching so a launcher claims the window it just started instead of growing a duplicate button, and register menus opened without a GDK event so the panel does not auto-hide under them.
- Fix a cairo surface leak in preview capture and a stale temp file that could block future configuration saves; keep the current association rule when the app chooser is confirmed without a selection.
- Add persistent, exact window-class associations with a desktop-file chooser and per-window rule removal. Manual rules take priority; indistinguishable classes share a rule.
- Limit visible icons with an overflow menu while preserving separate window buttons, workspace filtering, saved pin order and access to hidden windows.
- Add preview close controls, workspace labels, complete title tooltips and monitor-aware thumbnail and popup limits.
- Use GTK theme colors for popup, audio badge, active indicators and focus outlines. Add focus navigation and configurable left/middle click and window scrolling.
- Back up and restore pinned apps, associations and preferences through native file choosers. Validate imports and automatically preserve the current configuration before replacement; retain running windows.
- Bound thumbnail caches to 32 MiB per instance and evict older frames under pressure.
- Extend isolated testing with installed Chrome/Thunar window lifecycles, disposable audio server recovery, scale/theme checks and configurable stress runs; add read-only desktop association inspection.
- Make the isolated suites runnable from Wayland desktop sessions: clear Wayland host variables, force the private session's X11 backend and pin Chrome to `--ozone-platform=x11`.

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
