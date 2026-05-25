# xerotop

A **gkrellm-style, battery-conscious system monitor** for Wayland (wlroots /
layer-shell), in Rust + GTK4. A vertical (or horizontal) stack of live meters —
CPU, memory, GPU, disk, network, temps/fan, battery, volume, brightness, a top
process list — plus a real **taskbar** and **system tray with cascading menus**,
all in **one process** with **zero polling subprocesses**.

It's the successor to an [ewwii](https://github.com/Ewwii-sh/ewwii)-based bar
that spawned ~600 shell processes per second to poll metrics. xerotop reads
`/proc`, `/sys`, ALSA and statvfs natively and redraws only on new data.

## Design principles

- **Battery-first.** One central scheduler, one 250ms timer. Each panel updates
  at its own (possibly fractional) interval; on battery every interval is
  stretched by a configurable multiplier. With `smooth = false` (stepped graphs)
  there are no extra wakeups — graphs redraw only when a new sample arrives. The
  default `smooth = true` adds continuous scrolling via a frame clock (prettier,
  but it does redraw per frame while visible); flip it off for minimal wakeups.
- **Worse-is-better.** Smallest thing that genuinely works, then iterate.
- **Single process.** Metrics, taskbar via wlr-foreign-toplevel, and tray via
  StatusNotifier all live in-process — no IPC, no sockets, no helper daemons.
  (Each Wayland/D-Bus protocol gets its own thread; snapshots stream to GTK over
  channels.)
- **Configure by prefs, not a config language.** Plain, stable TOML. The format
  is boring on purpose and will not be rug-pulled.
- **Orientation-agnostic.** Vertical and horizontal bars share one layout model;
  multiple bars are a planned extension. v1 ships the vertical bar.

## Status — feature-complete vs. the ewwii bar it replaces

Working:

- **layer-shell bar** anchorable to any edge, configurable thickness & opacity
- native meters with autoscaled gamma graphs: **CPU, MEM (used + cache), GPU,
  DISK, NET**
- **TEMP** panel: multiple hwmon sensors as mini-charts + fan RPM
- **BAT / VOL / BRI** as icon + level bar + value, with **scroll/click control**
  (volume via ALSA, brightness via `brightnessctl`)
- **TOP** process list (EMA-smoothed CPU%)
- **header**: green power button (→ lock/logout/reboot/shutdown popover),
  12-hour clock + date, brass lock button
- **taskbar** (`win`): open windows via wlr-foreign-toplevel — app icons, focus
  highlight, minimized = grayed/italic, left-click activate, right-click minimize
- **system tray** (`tray`): StatusNotifier host — themed/pixmap icons,
  left-click activate, right-click **cascading D-Bus menu** with hover submenus
  and full `AboutToShow` support; menu item ids are resolved by path from the
  freshest layout so apps that renumber mid-open (nm-applet, …) don't misfire
- TOML config with first-run defaults and a single battery-aware scheduler
- **live preferences GUI** — right-click any dead space on the bar (gkrellm-style)
  for Preferences/Quit, or run `xerotop --prefs`. Every control applies instantly
  (no restart): edge, thickness, monitor, opacity, gamma, panels (add/remove/
  reorder/intervals), and a full theme editor with live color & font pickers.
- **themes** — colors + font are data, not baked CSS. The built-in `default`
  reproduces the dark look; the GUI's "Save theme as…" writes
  `~/.config/xerotop/themes/<name>.toml`, selectable by name.

Planned: occlusion-aware pausing, multiple bars, horizontal-mode polish.

## Build & run

```sh
cargo build --release
./target/release/xerotop
```

Requires GTK4, `gtk4-layer-shell`, a wlroots compositor (labwc, sway, …), and —
for tray/taskbar/brightness — a StatusNotifier-capable session, the
foreign-toplevel protocol, and `brightnessctl` on `PATH`.

### Toolchain note

Pinned to **gtk4-rs 0.10** so it builds on Rust **1.89** (gtk4-rs ≥ 0.11 needs
Rust ≥ 1.92). If you install a newer toolchain, bumping `Cargo.toml` to gtk4
0.11 / gtk4-layer-shell 0.8 is a one-liner.

## Configuration

Most people won't touch the file — right-click the bar (or `xerotop --prefs`)
and use the GUI. First run writes `~/.config/xerotop/config.toml`. Highlights:

```toml
theme = "default"   # built-in, or a name under ~/.config/xerotop/themes/<name>.toml

[bar]
edge = "right"      # left | right | top | bottom  (left/right = vertical)
thickness = 150     # px: width for vertical bars, height for horizontal
length = "full"     # "full" to fill the edge, or a pixel count (e.g. 600)
align = "center"    # start | center | end  (where a fixed-length bar sits)
monitor = 0         # output index; -1 = compositor's choice
smooth = true       # continuous graph scrolling; false = stepped (less battery)
graph_gamma = 2.0   # autoscaled-graph spikiness; >1 sharper peaks, 1.0 = linear
opacity = 0.88      # background opacity: 0.0 transparent .. 1.0 opaque

[power]
battery_interval_multiplier = 2.0   # on battery, multiply every interval by this

[actions]                           # header buttons run these on click
lock = "loginctl lock-session"
# logout / reboot / shutdown ...

[[panel]]                           # panels render in order (top→bottom / left→right)
type = "header"
time_format = "%I:%M %p"            # 12-hour; "%H:%M" for 24-hour
date_format = "%a %d %b"
[[panel]]
type = "cpu"
interval = 2                        # seconds; may be fractional (0.5 = 2/sec)
graph = true
# ... mem, gpu, disk, net, temp, bat, vol, bri, top, win (taskbar), tray
```

## Architecture

| Module | Role |
|---|---|
| `config.rs`  | TOML schema + load/first-run defaults (`Edge`, panels, actions) |
| `power.rs`   | AC/battery detection (`/sys/class/power_supply`) |
| `metrics.rs` | native samplers: cpu, mem, gpu, disk (statvfs), net, hwmon temps/fan, battery, ALSA volume, brightness |
| `widgets.rs` | reusable `Graph` (multi-series, filled, gamma, time-windowed; redraws on push only) and `Bar` level meter |
| `panels.rs`  | `Panel` builders + taskbar & tray UI (icon resolution, cascading menus) |
| `taskbar.rs` | calloop thread: wlr-foreign-toplevel client → toplevel snapshots / actions |
| `tray.rs`    | tokio thread: StatusNotifier host (`system-tray`) → item+menu snapshots / actions |
| `theme.rs`   | `Theme` (colors + font) → generated stylesheet + graph palette; loads theme files |
| `prefs.rs`   | live preferences GUI (General / Theme / Panels), mutates state + `apply()` |
| `bar.rs`     | layer-shell window, `BarHandle::apply()` live re-render, right-click menu, scheduler |
| `main.rs`    | app bootstrap, `--prefs` |
