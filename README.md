# RustShot Wayland

RustShot’s annotate-and-save workflow for **Omarchy / Hyprland** (Wayland).

The original [RustShot](../RustShot) talks to X11. This sibling grabs pixels with `grim` (the same wlr-screencopy path Omarchy uses) and draws the overlay as a **wlr-layer-shell** surface so Hyprland actually shows it on top of the desktop.

## Needs

- Wayland session (`WAYLAND_DISPLAY`)
- `grim`, `wl-copy` (wl-clipboard)
- Hyprland (`hyprctl`) for monitor layout — optional but expected on Omarchy

## Build

```bash
cd RustShotWayland
cargo install --path .
```

Binary: `rustshot-wayland`. Config: `~/.config/rustshot-wayland/config.toml`.  
DBus name: `org.rustshot.Wayland` (can run next to X11 `rustshot`).

## Use

```bash
rustshot-wayland                 # daemon
rustshot-wayland gui             # region select + annotate
rustshot-wayland gui -c          # + clipboard
rustshot-wayland full -c         # all outputs, no UI
```

Hyprland bind (user config, e.g. `~/.config/hypr/bindings.lua`):

```text
exec-once = rustshot-wayland
bind = , Print, exec, rustshot-wayland gui -c
```

Or DBus:

```bash
dbus-send --session --type=method_call --dest=org.rustshot.Wayland / \
  org.rustshot.Wayland.graphicCaptureFlags \
  string:"" uint32:0 boolean:true boolean:false string:""
```

Overlay keys match RustShot: drag region, `1`–`8` tools, Enter save, Ctrl+C copy, Esc cancel.

On Omarchy, overlay chrome (selection frame, tool strip, hints, tray) follows the live theme at `~/.local/state/omarchy/current/theme/colors.toml`. Annotation strokes stay red.
