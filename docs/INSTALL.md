# Installation

## Supported environment

The initial validation environment is Arch/CachyOS, XFCE 4.20, GTK 3.24 and X11. PulseAudio or PipeWire-Pulse provides application audio. The plugin uses system GTK themes and icons. Wayland support is not implemented.

See [README requirements](../README.md#requirements). Meson checks minimum library versions. On other distributions install matching development packages; package names differ.

## Arch package

For a published release, download both `PKGBUILD` and `zero-dock-<version>.tar.gz` from [Releases](https://github.com/lunemoe/zero-dock/releases) into the same directory, inspect `PKGBUILD`, then run:

```sh
makepkg -si
```

From a source checkout:

```sh
python tools/make-dist.py
cd packaging
makepkg -si
```

The source archive has a reproducible SHA-256 digest recorded in `packaging/PKGBUILD`. `makepkg` performs the basic unit checks. Run integration checks separately as described in [TESTING.md](TESTING.md).

## Source install

```sh
meson setup build --prefix=/usr --libdir=lib -Dtests=false
meson compile -C build
sudo meson install -C build
```

The module is installed under `<libdir>/xfce4/panel/plugins/`, with its registration under `<datadir>/xfce4/panel/plugins/`. Match the paths used by your distribution's panel. A custom `~/.local` prefix is not assumed to be searched automatically.

## Add and configure

Right-click the panel → **Panel → Add New Items → Zero Dock Window Dock**. The Chinese entry is **Zero Dock 窗口停靠**. Restarting the desktop is not required to add a registered plugin. If a newly installed item is absent, open Add New Items again and verify the installation paths.

Open the instance's **Properties** for previews, numbering, workspace filtering and reserved slots. Use the panel's icon-size setting for larger icons. Pin applications through a window's context menu or drop `.desktop` files from a file manager.

## Upgrade and removal

The currently running external process may retain the previous library after a package upgrade. To replace it safely:

1. Copy `~/.config/xfce4/panel/zero-dock-<instance-id>.rc` to a persistent backup outside that directory. Removing the instance removes its configuration.
2. Upgrade the package, then remove only the old Zero Dock instance using its panel menu.
3. Add a new Zero Dock instance. To import the saved settings when adding from a terminal, use the panel's native interface:

```sh
gdbus call --session --dest org.xfce.Panel \
  --object-path /org/xfce/Panel --method org.xfce.Panel.AddNewItem \
  zero-dock "['--import-config=/absolute/path/to/saved.rc']"
```

If there are several panels, XFCE asks which one to use. The import is read only when the new instance has no existing configuration; settings are then saved under the new instance ID. You can also re-pin launchers manually.

There is no need to restart the panel, XFCE, window manager or display manager. If the dock fails, remove only that instance and add XFCE's standard **Window Buttons**, disabling grouping and labels if desired.

For an Arch package, remove the instance first and uninstall with `sudo pacman -R xfce4-zero-dock-plugin`. For a source install, Meson's build directory contains `meson-logs/install-log.txt`; review that list before deleting installed files.
