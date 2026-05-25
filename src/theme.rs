//! Visual theme: colors + font, separate from structural config. A `Theme`
//! generates the whole stylesheet (so colors are data, not baked CSS) and the
//! graph palette the panels draw with. Themes can be loaded from
//! `~/.config/xerotop/themes/<name>.toml`; the built-in default reproduces the
//! original dark look.

use crate::widgets::Rgba;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Theme {
    /// Base font for the whole bar (Nerd Font recommended for the glyphs).
    pub font_family: String,
    /// Bar background (hex `#rrggbb`); alpha comes from `bar.opacity`.
    pub background: String,
    /// Primary value/readout text.
    pub foreground: String,
    /// Panel labels (CPU, MEM, …).
    pub label: String,
    /// Secondary/dim text (sub-values, minimized windows).
    pub muted: String,
    /// Lock button (brass by default).
    pub accent_lock: String,
    /// Power button (green by default).
    pub accent_power: String,
    // Graph palette — a named set reused across panels.
    pub green: String,
    pub cyan: String,
    pub amber: String,
    pub red: String,
    pub violet: String,
    pub pale: String,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            font_family: "FiraCode Nerd Font Mono".into(),
            background: "#101014".into(),
            foreground: "#ccddee".into(),
            label: "#99aabb".into(),
            muted: "#667788".into(),
            accent_lock: "#d4af37".into(),
            accent_power: "#5fd75f".into(),
            green: "#66ff66".into(),
            cyan: "#66ccff".into(),
            amber: "#ffbf4d".into(),
            red: "#ff7366".into(),
            violet: "#c78cff".into(),
            pale: "#b3ebff".into(),
        }
    }
}

/// Parse `#rrggbb` (or `rrggbb`) into 0–255 components; falls back to mid-grey.
fn parse_hex(s: &str) -> (u8, u8, u8) {
    let h = s.trim().trim_start_matches('#');
    if h.len() == 6
        && let (Ok(r), Ok(g), Ok(b)) = (
            u8::from_str_radix(&h[0..2], 16),
            u8::from_str_radix(&h[2..4], 16),
            u8::from_str_radix(&h[4..6], 16),
        )
    {
        return (r, g, b);
    }
    (128, 128, 128)
}

fn rgba(hex: &str, a: f64) -> Rgba {
    let (r, g, b) = parse_hex(hex);
    (r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0, a)
}

/// Blend a hex color toward white by `f` (0..1), returned as `#rrggbb`. Used to
/// derive the brighter :hover variants so they stay theme-driven.
fn lighten(hex: &str, f: f64) -> String {
    let (r, g, b) = parse_hex(hex);
    let mix = |c: u8| (c as f64 + (255.0 - c as f64) * f).round() as u8;
    format!("#{:02x}{:02x}{:02x}", mix(r), mix(g), mix(b))
}

impl Theme {
    /// The graph drawing palette for `panels`.
    pub fn palette(&self) -> crate::panels::Palette {
        crate::panels::Palette {
            green: rgba(&self.green, 0.9),
            cyan: rgba(&self.cyan, 0.9),
            amber: rgba(&self.amber, 0.9),
            red: rgba(&self.red, 0.9),
            violet: rgba(&self.violet, 0.9),
            pale: rgba(&self.pale, 0.85),
        }
    }

    /// Generate the full stylesheet. `opacity` (from config) sets the bar's
    /// background alpha. Structural rules (padding, sizes) are constant; colors
    /// and the font come from the theme.
    pub fn css(&self, opacity: f64) -> String {
        let (br, bg_, bb) = parse_hex(&self.background);
        let bar_bg = format!("rgba({br},{bg_},{bb},{:.3})", opacity.clamp(0.0, 1.0));
        let icon = lighten(&self.label, 0.12);
        let bright = lighten(&self.foreground, 0.3);
        format!(
            r#"
.bar {{ font-family: "{font}", monospace; font-size: 12px; background-color: {bar_bg}; padding: 6px; }}
.panel {{ padding: 2px 4px; }}
.meter {{ padding: 1px 4px; }}
.rule {{ background-color: rgba(255,255,255,0.12); min-height: 1px; min-width: 1px; margin: 2px 0; }}
.label {{ font-weight: bold; color: {label}; }}
.value {{ color: {value}; }}
.meter-icon {{ color: {icon}; font-size: 20px; }}
.graph {{ background-color: rgba(0,0,0,0.25); }}
.bar-meter {{ background-color: rgba(0,0,0,0.25); }}
.sub {{ font-size: 9px; color: {muted}; }}
.task {{ background: transparent; border: none; box-shadow: none; outline: none; min-height: 0; padding: 1px 3px; font-size: 11px; color: {label}; }}
.task:hover {{ color: {value}; }}
.task-active {{ background-color: rgba(255,255,255,0.10); color: {bright}; }}
.task-min {{ color: {muted}; font-style: italic; background-color: rgba(255,255,255,0.03); border-radius: 4px; }}
.tray-item {{ background: transparent; border: none; box-shadow: none; outline: none; min-height: 0; min-width: 0; padding: 2px; }}
.tray-item:hover {{ background-color: rgba(255,255,255,0.10); }}
.clock-time {{ font-weight: bold; font-size: 18px; color: {bright}; }}
.clock-date {{ font-size: 10px; color: {label}; }}
.hbtn {{ background: transparent; border: none; box-shadow: none; outline: none; min-height: 0; min-width: 0; padding: 0 4px; color: {label}; font-size: 17px; }}
.hbtn:hover {{ color: {bright}; }}
.lock {{ color: {lock}; }}
.lock:hover {{ color: {lock_hi}; }}
.power {{ color: {power}; }}
.power:hover {{ color: {power_hi}; }}
.menu {{ padding: 4px; }}
.menu-item {{ background: transparent; border: none; box-shadow: none; color: {value}; padding: 4px 14px; }}
.menu-item:hover {{ background-color: rgba(255,255,255,0.10); }}
"#,
            font = self.font_family,
            label = self.label,
            value = self.foreground,
            muted = self.muted,
            lock = self.accent_lock,
            lock_hi = lighten(&self.accent_lock, 0.25),
            power = self.accent_power,
            power_hi = lighten(&self.accent_power, 0.25),
        )
    }
}

pub fn themes_dir() -> PathBuf {
    crate::config::config_path()
        .parent()
        .map(|p| p.join("themes"))
        .unwrap_or_else(|| PathBuf::from("themes"))
}

/// Resolve a theme by name: `"default"`/empty → built-in; otherwise load
/// `themes/<name>.toml`, falling back to the built-in on any error.
pub fn resolve(name: &str) -> Theme {
    if name.is_empty() || name == "default" {
        return Theme::default();
    }
    let path = themes_dir().join(format!("{name}.toml"));
    match std::fs::read_to_string(&path) {
        Ok(s) => toml::from_str(&s).unwrap_or_else(|e| {
            eprintln!("xerotop: theme '{name}' parse error ({e}); using default");
            Theme::default()
        }),
        Err(_) => {
            eprintln!("xerotop: theme '{name}' not found; using default");
            Theme::default()
        }
    }
}
