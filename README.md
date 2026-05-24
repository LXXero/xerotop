# xerotop

A **gkrellm-style, battery-conscious system monitor** for Wayland (wlroots /
layer-shell), in Rust + GTK4. A vertical (or horizontal) stack of live meters —
CPU, memory, network, clock — with a *proper* taskbar and tray planned, all in
**one process** with **zero subprocesses**.

It's the successor to an [ewwii](https://github.com/Ewwii-sh/ewwii)-based bar
that spawned ~600 shell processes per second to poll metrics. xerotop reads
`/proc` and `/sys` natively and redraws only on new data.

## Design principles

- **Battery-first.** One central scheduler, one timer. Each panel updates at its
  own interval; on battery every interval is stretched by a configurable
  multiplier. Graphs redraw **only when a new sample arrives**, never on a frame
  clock. Idle wakeups stay near zero.
- **Worse-is-better.** Smallest thing that genuinely works, then iterate.
- **Single process.** Metrics, (soon) taskbar via wlr-foreign-toplevel and tray
  via StatusNotifier — no IPC, no sockets, no helper daemons.
- **Configure by prefs, not a config language.** Plain, stable TOML. The format
  is boring on purpose and will not be rug-pulled.
- **Orientation-agnostic.** Vertical and horizontal bars share one layout model;
  multiple bars are a planned extension. v1 ships the vertical bar.

## Status — walking skeleton

Working: layer-shell bar (any edge), native **CPU / MEM / NET / TEMP / BATTERY**
meters with graphs, a clock, TOML config with first-run defaults, and a single
battery-aware scheduler.

Planned: system tray (StatusNotifier), taskbar (wlr-foreign-toplevel), disk panel,
theme packs, occlusion-aware pausing, multi-bar.

## Build & run

```sh
cargo build --release
./target/release/xerotop
```

Requires GTK4, `gtk4-layer-shell`, and a wlroots compositor (labwc, sway, …).

### Toolchain note

Pinned to **gtk4-rs 0.10** so it builds on Rust **1.89** (gtk4-rs ≥ 0.11 needs
Rust ≥ 1.92). If you install a newer toolchain, bumping `Cargo.toml` to gtk4
0.11 / gtk4-layer-shell 0.8 is a one-liner.

## Configuration

First run writes `~/.config/xerotop/config.toml`:

```toml
[bar]
edge = "right"      # left | right | top | bottom  (left/right = vertical)
thickness = 150     # px: width for vertical bars, height for horizontal
monitor = 0

[power]
battery_interval_multiplier = 2.0

[[panel]]
type = "clock"
[[panel]]
type = "cpu"
interval = 1
graph = true
# ... mem, net
```

## Architecture

| Module | Role |
|---|---|
| `config.rs`  | TOML schema + load/defaults |
| `power.rs`   | AC/battery detection (`/sys/class/power_supply`) |
| `metrics.rs` | native `/proc` & `/sys` samplers (cpu, mem, net, temp, battery) |
| `widgets.rs` | reusable ring-buffer `Graph` (redraws on push only) |
| `panels.rs`  | `Panel` builders (root widget + interval + update closure) |
| `bar.rs`     | layer-shell window + central battery-aware scheduler |
| `main.rs`    | app + CSS |
