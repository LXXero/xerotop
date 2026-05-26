# xerotop — TODO

Running list of planned work and ideas. Done items live in git history; this is
what's *not* built yet.

## Configurability (the old "Phase 3")

Hardcoded constants that should become config/theme knobs:

- *(optional)* **Configurable `top` EMA alpha** — the `top` list is already
  EMA-smoothed (`metrics.rs` `ALPHA = 0.3`, calm by design); exposing alpha as a
  knob is the only open bit, and low value since it isn't jittery. Skip unless
  asked.

(Done: per-panel graph height, per-panel graph time window, global meter-bar
thickness, top-list count, vol/bri scroll step.)

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

- (Done: real screenshot at `www/screenshot.png`, in the README + site.)
- Refresh the screenshot when the layout changes meaningfully.
