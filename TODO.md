# xerotop — TODO

Running list of planned work and ideas. Done items live in git history; this is
what's *not* built yet.

## Configurability (the old "Phase 3")

Hardcoded constants that should become config/theme knobs:

- **Graph height** (`GRAPH_H` = 24, `MINI_H` = 14) — per-panel or global.
- **Graph time window** — currently a fixed ~60s in `widgets.rs`.
- **Bar-meter thickness** (`BAR_H` = 7).
- **Top-list count** (`top` shows a fixed N processes).
- **EMA smoothing alpha** for the top process list.
- **Scroll step sizes** for volume / brightness (currently 5%).

## Panels / widgets to add

- **Voltages in the `sensors` panel** — the panel was renamed from `temp` to
  `sensors` because it already does temps + fans; extend `metrics::SensorKind`
  and the hwmon scan to also expose `inN_input` (voltage) rails as selectable
  rows.

- **Load average** — `/proc/loadavg` (1/5/15 min). Trivial native panel.
- **Now playing (MPRIS)** — current track/artist + play-pause over D-Bus (we
  already talk D-Bus for the tray).
- **Network info** — IP address / wifi SSID alongside the net throughput graph.

## Polish / architecture

- **Live-drag perf split** — opacity (pure CSS) and thickness/length/monitor
  (window geometry) shouldn't rebuild *all* panels on every slider tick; split
  `apply()` into restyle / relayout / full-rebuild paths.
- **Horizontal-mode polish** — top/bottom edges work but the layout (fixed graph
  widths, etc.) needs tuning for a horizontal bar.
- **Multiple bars** — more than one bar at once.
- **Occlusion-aware pausing** — stop updating when the bar is covered/offscreen.
- **Example themes** — ship a couple of `themes/*.toml` so the theme dropdown
  isn't just "default" out of the box.

## Docs / site

- Add a real **screenshot** to `www/` and the README.
