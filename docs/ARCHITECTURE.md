# Architecture

Zero Dock is a GTK 3 shared module loaded by XFCE's external `wrapper-2.0` provider. Each installed instance normally gets its own process and configuration. No Python runtime is used by the plugin.

| File | Responsibility |
|---|---|
| `entry.c` | XFCE module registration |
| `plugin.c` | Instance lifecycle, application indexing, configuration and panel geometry |
| `buttons.c` | Icon drawing, interaction, pinning, ordering and scroll accumulation |
| `menu.c` | Native window, workspace, launcher and panel menus |
| `preview.c` | XComposite capture, frame cache, clickable previews and volume bubbles |
| `audio.c` | PulseAudio subscriptions, process matching, mute and volume |
| `input.c` | Separate XInput2 connection, device scroll increments and speaker hit testing |
| `zero-dock.h` | Shared instance and button interfaces |

libxfce4windowing supplies windows and workspaces. Launchers use GDesktopAppInfo. Audio streams match window process ancestry first, with executable/class hints as a fallback. Shared application audio can affect several window buttons.

Capture reads a compositor's redirected frame pixmap and scales it to a bounded thumbnail. Minimized windows retain their last frame in memory. GTK timers control delayed previews, refresh, active-window caching and volume-bubble expiry. Popup placement uses the anchor's screen coordinates and corresponding monitor bounds.

XInput2 uses its own X connection and GLib file-descriptor source, avoiding changes to GTK's event selections. Only scroll values over this instance's visible speaker controls become audio actions. GTK/raw duplicates use timestamps to avoid applying a tick twice. Device topology changes invalidate cached axis metadata.

Instance cleanup disconnects callbacks, removes timer/input sources, destroys popups, disconnects audio and releases window/image references. Drag payload keys are looked up against the local instance's button list; external data is never dereferenced as a pointer.
