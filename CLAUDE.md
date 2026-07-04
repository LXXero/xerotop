# xerotop — working notes

A gkrellm-style, battery-conscious Wayland (wlroots/layer-shell) system-monitor
bar in Rust + GTK4. Meters, taskbar, and tray in one process. GPL-3.0-or-later.

## Layout

- `src/bar.rs` — assembles the layer-shell bar from config; owns the live-apply
  tiers and the central scheduler.
- `src/panels.rs` — every panel builder + the shared thread-locals/hosts. Panel
  kinds: `header clock cpu cores memory gpu disk net sensors battery volume
  brightness keyboard wx mail top uptime tasks tray`.
- `src/config.rs` — `Config`/`PanelConfig` (TOML at `~/.config/xerotop/config.toml`)
  and the enums. New per-panel options go on `PanelConfig` with `#[serde(default)]`
  and must be added to **both** `default_panels()` (config.rs) and the add-panel
  literal (prefs.rs) or it won't compile.
- `src/theme.rs` — `Theme` (colors + fonts), CSS generation, theme resolution.
- `src/prefs.rs` — the GTK prefs window.

## Build / run

- `cargo build --release` → `target/release/xerotop`. Build green before committing.
- Installed as a package (`/usr/bin/xerotop`), **not** a symlink anymore.
- Dev iteration: `make link` (symlinks `~/.local/bin/xerotop` → `target/release`,
  so a rebuild updates it), `make run`, or `make restart` (rebuild + hot-swap the
  running bar). The labwc menu "XeroTop" item + autostart launch `/usr/bin/xerotop`
  (the package), so they run the *installed* build, not your dev tree.
- To restart the running bar: `pkill xerotop && setsid -f xerotop >/dev/null 2>&1`.

## Live-apply tiers (preserve graph history!)

The bar deliberately avoids full rebuilds so scrolling graph history survives an
edit. Use the lightest tier that does the job:

- `restyle()` — CSS only (colors, fonts, opacity, **corner radius**). No rebuild.
- `relayout()` — window geometry (monitor, edge, length, align). No rebuild.
- `apply_mail()` / `apply_header()` — mail + header-command edits, applied live
  (the click handlers read their command live) without a rebuild.
- `apply()` — **full rebuild**: drops and recreates every panel, resetting all
  graph history. Only for structural changes (add/remove/reorder panels, edge
  flip, panel-content toggles).

Prefs text fields should apply on **both** Enter (`connect_activate`) *and*
focus-out (`EventControllerFocus::connect_leave`) — otherwise "type + Save"
silently drops the edit. Dependent controls hide when their parent toggle is off.

## Adding themes  ⚠️

Themes live in `themes/<name>.toml` and are **palette + fonts only**. A theme may
technically also carry `[[header]]` and `[[sensors]]` tables, but a theme switch
**applies those to the user's config** (`prefs.rs`) — i.e. it would overwrite the
user's header buttons / sensor colors. That machinery exists only for the prefs
"Save + panel colors" button. **When adding a distributable color preset, include
ONLY the scalar palette/font fields — no `[[header]]` or `[[sensors]]` blocks.**
(`gkrellm`/`nyz` are correct references; strip any header blocks you copy from a
theme made via "Save + panel colors".)

Required fields (see any theme or `Theme::default()`): `font_family`, `background`,
`foreground`, `label`, `muted`, `font_small/normal/large`, `green cyan amber red
violet`, `led_on`, `graph_background`, `graph_background_opacity`. Use a Nerd Font
for `font_family` or the glyphs render as tofu (default: `FiraCode Nerd Font Mono`).

To ship a theme built into the binary, add it to `EMBEDDED_THEMES` in `theme.rs`
via `include_str!("../themes/<name>.toml")` — then it shows in the prefs picker
without a config-dir copy. Resolution order: filesystem theme → embedded → default.

## Packaging

- `packaging/aur/PKGBUILD` (`xerotop-git`) builds from git and installs via
  `make install DESTDIR="$pkgdir" PREFIX=/usr` (a normal DESTDIR/PREFIX install;
  `make link` is the dev-only symlink shortcut).
- `options=('!lto' '!debug')`: `!lto` because the `ring` crate (pulled in by the
  weather HTTPS fetch) ships non-LTO assembly and fails to link under makepkg's
  LTO; `!debug` skips an empty debug split package.
- Deps: `gtk4 gtk4-layer-shell ttf-firacode-nerd`; `brightnessctl` optdepend.
- Rebuild/install: `cd packaging/aur && makepkg -sfi`.

## Commits

Clean, professional, conventional-style messages (`theme:`, `header:`, `prefs:`,
`packaging:` …). No cutesy language in commits. End with:
`Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>`
