# xerotop — TODO

Running list of planned work and ideas. Done items live in git history; this is
what's *not* built yet.

## Configurability (the old "Phase 3")

Hardcoded constants that should become config/theme knobs:

- **Graph time window** — currently a fixed ~60s in `widgets.rs`.
- **EMA smoothing alpha** for the top process list (de-jitter the ordering).

(Done: per-panel graph height, global meter-bar thickness, top-list count,
vol/bri scroll step.)

## Panels / widgets to add

- **Now playing (MPRIS)** — current track/artist + play-pause over D-Bus (we
  already talk D-Bus for the tray).
- **Network info** — IP address / wifi SSID alongside the net throughput graph.

## Polish / architecture

- **Horizontal-mode polish** — top/bottom edges work but the layout (fixed graph
  widths, etc.) needs tuning for a horizontal bar.
- **Multiple bars** — more than one bar at once.
- **Occlusion-aware pausing** — stop updating when the bar is covered/offscreen.
- **Example themes** — ship a couple of `themes/*.toml` so the theme dropdown
  isn't just "default" out of the box.

## Docs / site

- Add a real **screenshot** to `www/` and the README.
