//! Visual theme: colors + font, separate from structural config. A `Theme`
//! generates the whole stylesheet (so colors are data, not baked CSS) and the
//! graph palette the panels draw with. Themes can be loaded from
//! `~/.config/xerotop/themes/<name>.toml`; the built-in default reproduces the
//! original dark look.

use crate::config::{HeaderButton, TempSensor};
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
    // Font sizes (px) — gkrellm-style tiers applied to text roles:
    // small = sub-values, date, taskbar; normal = labels/values/base;
    // large = the clock time.
    pub font_small: i32,
    pub font_normal: i32,
    pub font_large: i32,
    // Graph palette — a named set reused across panels.
    pub green: String,
    pub cyan: String,
    pub amber: String,
    pub red: String,
    pub violet: String,
    /// Keyboard-LED "on"/glow color.
    pub led_on: String,
    /// Graph background fill (hex `#rrggbb`).
    pub graph_background: String,
    /// Graph background fill opacity 0.0 .. 1.0.
    pub graph_background_opacity: f64,
    // Optional panel-color defaults the theme can carry. Written only by the
    // "Save + panel colors" button; when present, an interactive theme switch
    // applies them to the config (so a theme can capture a full look, not just
    // the base palette). `None` (the usual case) = palette + fonts only. These
    // must stay last in the struct — TOML arrays-of-tables serialize after the
    // scalar fields.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sensors: Option<Vec<TempSensor>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub header: Option<Vec<HeaderButton>>,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            font_family: "FiraCode Nerd Font Mono".into(),
            background: "#101014".into(),
            foreground: "#ccddee".into(),
            label: "#99aabb".into(),
            muted: "#667788".into(),
            font_small: 10,
            font_normal: 12,
            font_large: 18,
            green: "#66ff66".into(),
            cyan: "#66ccff".into(),
            amber: "#ffbf4d".into(),
            red: "#ff7366".into(),
            violet: "#c78cff".into(),
            led_on: "#5fd75f".into(),
            graph_background: "#000000".into(),
            graph_background_opacity: 0.25,
            sensors: None,
            header: None,
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

/// Strip CSS-significant characters from a font name so a theme can't break out
/// of the quoted `font-family` value and inject rules.
fn sanitize_font(name: &str) -> String {
    name.replace(['"', '\\', ';', '{', '}', '\n', '\r'], "")
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
        // Normalize every interpolated color through parse_hex → "#rrggbb" so a
        // malformed theme value can't inject into or break the stylesheet, and
        // strip CSS-significant chars from the font name.
        let norm = |s: &str| {
            let (r, g, b) = parse_hex(s);
            format!("#{r:02x}{g:02x}{b:02x}")
        };
        let font = sanitize_font(&self.font_family);
        let (gr, gg, gb) = parse_hex(&self.graph_background);
        let graph_bg = format!("rgba({gr},{gg},{gb},{:.3})", self.graph_background_opacity.clamp(0.0, 1.0));
        format!(
            r#"
/* The bar window must be transparent, or our semi-transparent .bar fill
   composites over GTK's opaque window background (looks grey) instead of the
   desktop showing through. Scoped to .xerotop so the prefs window (same display,
   shares this provider) stays opaque. */
window.xerotop {{ background-color: transparent; }}
.xerotop .bar {{ font-family: "{font}", monospace; font-size: {normal}px; background-color: {bar_bg}; padding: 6px; }}
.xerotop .panel {{ padding: 0 4px; }}
.xerotop .meter {{ padding: 1px 4px; }}
.xerotop .rule {{ background-color: rgba(255,255,255,0.12); min-height: 1px; min-width: 1px; margin: 2px 0; }}
/* glyph picker in prefs must use the bar font (default-font prefs window has no
   Nerd Font glyphs) and a larger size so the little glyphs are legible */
.xerotop .glyphpick, .xerotop .glyphpick * {{ font-family: "{font}", monospace; }}
.xerotop .label {{ font-weight: bold; color: {label}; }}
/* tabular (fixed-width) digits so a changing number keeps the same glyph
   advance — combined with width_chars on the value labels, a value update is
   an in-place repaint instead of a width change that relayouts the bar. */
.xerotop .value {{ color: {value}; font-variant-numeric: tabular-nums; font-feature-settings: "tnum" 1; }}
.xerotop .meter-icon {{ color: {icon}; font-size: 20px; }}
/* the hourglass fills its em more than the FA glyphs — trim it to match. */
.xerotop .uptime-icon {{ font-size: 16px; }}
/* weather (nf-weather e3xx) glyphs render smaller per-em, so bump the font to
   match the meter/keyboard glyphs' visual size; the negative margins claw back
   the line-box padding the bigger font adds, so the weather row stays the same
   height as the meter rows instead of growing. */
.xerotop .weather-icon {{ color: {icon}; font-size: 30px; margin-top: -6px; margin-bottom: -6px; }}
.xerotop .mail-icon {{ color: #e6f0ff; font-size: 20px; }}
.xerotop .mail-unread {{ color: #ffbf4d; font-weight: bold; }}
.xerotop .graph {{ background-color: {graph_bg}; }}
.xerotop .bar-meter {{ background-color: transparent; }}
.xerotop .sub {{ font-size: {small}px; color: {muted}; font-variant-numeric: tabular-nums; font-feature-settings: "tnum" 1; }}
.xerotop .task {{ background: transparent; border: none; box-shadow: none; outline: none; min-height: 0; padding: 1px 3px; font-size: {small}px; color: {label}; }}
.xerotop .task:hover {{ color: {value}; }}
.xerotop .task-active {{ background-color: rgba(255,255,255,0.10); color: {bright}; }}
.xerotop .task-min {{ color: {muted}; font-style: italic; background-color: rgba(255,255,255,0.03); border-radius: 4px; }}
.xerotop .tray-item {{ background: transparent; border: none; box-shadow: none; outline: none; min-height: 0; min-width: 0; padding: 2px; }}
.xerotop .tray-item:hover {{ background-color: rgba(255,255,255,0.10); }}
.xerotop .clock-time {{ font-weight: bold; font-size: {large}px; color: {label}; }}
.xerotop .clock-ampm {{ font-size: {small}px; color: {value}; }}
.xerotop .clock-date {{ font-size: {small}px; color: {value}; }}
/* reserve the header-glyph height so enabling a date icon doesn't grow the row */
.xerotop .date-row {{ min-height: 20px; }}
/* keyboard indicators drawn as little rectangular LEDs: dim when off, glowing
   green when the function is active, with the label etched inside. */
.xerotop .led {{
  font-size: {small}px; font-weight: bold; padding: 0 3px; min-height: 11px;
  border-radius: 2px; border: 1px solid rgba(255,255,255,0.15);
  background-color: rgba(255,255,255,0.04); color: {muted};
}}
.xerotop .led-on {{
  color: #06140a; background-color: {led}; border-color: {led_border};
  box-shadow: 0 0 5px 1px {led};
}}
.xerotop .clock-daynum {{ font-weight: bold; font-size: {normal}px; color: {label}; }}
.xerotop .hbtn {{ background: transparent; border: none; box-shadow: none; outline: none; min-height: 0; min-width: 0; padding: 0 4px; color: {label}; font-size: 17px; }}
.xerotop .hbtn:hover {{ color: {bright}; }}
.xerotop .menu {{ padding: 4px; }}
.xerotop .menu-item {{ background: transparent; border: none; box-shadow: none; padding: 4px 14px; }}
.xerotop .menu-item:hover {{ background-color: rgba(255,255,255,0.10); }}
"#,
            font = font,
            small = self.font_small.clamp(4, 96),
            normal = self.font_normal.clamp(4, 96),
            large = self.font_large.clamp(4, 96),
            label = norm(&self.label),
            value = norm(&self.foreground),
            muted = norm(&self.muted),
            led = norm(&self.led_on),
            led_border = lighten(&self.led_on, 0.25),
        )
    }
}

/// A theme name is a strict slug so it can't escape the themes dir (e.g. `../`).
pub fn is_valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
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
    let name = if name.is_empty() { "default" } else { name };
    if !is_valid_name(name) {
        eprintln!("xerotop: invalid theme name '{name}'; using default");
        return Theme::default();
    }
    // Load themes/<name>.toml if present — including "default", so edits to the
    // default theme persist. Missing file → the built-in default.
    let path = themes_dir().join(format!("{name}.toml"));
    match std::fs::read_to_string(&path) {
        Ok(s) => toml::from_str(&s).unwrap_or_else(|e| {
            eprintln!("xerotop: theme '{name}' parse error ({e}); using default");
            Theme::default()
        }),
        Err(_) => Theme::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_hex_valid_with_and_without_hash() {
        assert_eq!(parse_hex("#ff8800"), (255, 136, 0));
        assert_eq!(parse_hex("ff8800"), (255, 136, 0));
        assert_eq!(parse_hex("  #00ff00 "), (0, 255, 0));
    }

    #[test]
    fn parse_hex_malformed_falls_back_to_grey() {
        assert_eq!(parse_hex("nope"), (128, 128, 128));
        assert_eq!(parse_hex("#fff"), (128, 128, 128)); // 3-digit not supported
        assert_eq!(parse_hex("#gggggg"), (128, 128, 128));
        assert_eq!(parse_hex(""), (128, 128, 128));
    }

    #[test]
    fn lighten_endpoints() {
        assert_eq!(lighten("#000000", 1.0), "#ffffff");
        assert_eq!(lighten("#3366cc", 0.0), "#3366cc"); // 0 = unchanged
        assert_eq!(lighten("#ffffff", 0.5), "#ffffff");
    }

    #[test]
    fn palette_maps_hex_to_rgba() {
        let t = Theme::default();
        let p = t.palette();
        // #66ff66 -> (0.4, 1.0, 0.4) at the graph fill alpha
        let (r, g, b, a) = p.green;
        assert!((r - 0.4).abs() < 0.01 && (g - 1.0).abs() < 0.01 && (b - 0.4).abs() < 0.01);
        assert!((a - 0.9).abs() < 0.001);
    }

    #[test]
    fn css_is_scoped_and_embeds_opacity_font_sizes() {
        let css = Theme::default().css(0.5);
        assert!(
            css.contains("window.xerotop"),
            "transparent rule must be scoped"
        );
        assert!(css.contains("rgba(16,16,20,0.500)"), "bg + opacity");
        assert!(css.contains("FiraCode Nerd Font Mono"));
        assert!(css.contains("font-size: 12px")); // font_normal default
        assert!(css.contains("font-size: 18px")); // font_large default (clock)
    }

    #[test]
    fn css_normalizes_bad_colors() {
        let t = Theme {
            label: "not-a-color".into(),
            ..Theme::default()
        };
        assert!(
            t.css(1.0).contains("#808080"),
            "bad color normalized to grey"
        );
    }

    #[test]
    fn sanitize_font_strips_css_significant_chars() {
        // A clean name passes through untouched.
        assert_eq!(
            sanitize_font("FiraCode Nerd Font Mono"),
            "FiraCode Nerd Font Mono"
        );
        // A malicious name can't keep the chars needed to escape the quoted value.
        let out = sanitize_font("Evil\"; } * { color: red; }");
        for c in ['"', ';', '{', '}', '\\', '\n', '\r'] {
            assert!(!out.contains(c), "must strip {c:?}");
        }
    }

    #[test]
    fn valid_name_accepts_slugs_rejects_paths() {
        assert!(is_valid_name("nord"));
        assert!(is_valid_name("my_theme-2"));
        assert!(!is_valid_name(""));
        assert!(!is_valid_name("../etc/passwd"));
        assert!(!is_valid_name("a/b"));
        assert!(!is_valid_name("a.b")); // dots disallowed (no traversal)
        assert!(!is_valid_name("has space"));
    }

    #[test]
    fn unknown_theme_resolves_to_default() {
        // A bogus/unsafe name must never read outside themes/ — falls back.
        let t = resolve("../../../etc/shadow");
        assert_eq!(t.font_family, Theme::default().font_family);
    }
}
