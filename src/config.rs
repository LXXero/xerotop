//! Declarative TOML config. Boring and stable on purpose — no config *language*,
//! just data. Missing fields fall back to sane defaults; a starter file is
//! written on first run.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Edge {
    Left,
    #[default]
    Right,
    Top,
    Bottom,
}

impl Edge {
    /// Top/Bottom bars lay panels out horizontally; Left/Right, vertically.
    pub fn is_horizontal(self) -> bool {
        matches!(self, Edge::Top | Edge::Bottom)
    }
}

/// The bar's length along its long axis (height for vertical bars, width for
/// horizontal): either stretched to fill the monitor edge, or a fixed pixel
/// count. In TOML: `length = "full"` (or "max") or `length = 600`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(from = "LenRepr", into = "LenRepr")]
pub enum BarLength {
    #[default]
    Full,
    Px(i32),
}

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum LenRepr {
    Word(String),
    Num(i32),
}

impl From<LenRepr> for BarLength {
    fn from(r: LenRepr) -> Self {
        match r {
            LenRepr::Num(n) if n > 0 => BarLength::Px(n),
            _ => BarLength::Full, // "full"/"max"/anything else
        }
    }
}
impl From<BarLength> for LenRepr {
    fn from(b: BarLength) -> Self {
        match b {
            BarLength::Full => LenRepr::Word("full".into()),
            BarLength::Px(n) => LenRepr::Num(n),
        }
    }
}

/// Where a fixed-length bar sits along its edge (ignored when length = full).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Align {
    Start,
    #[default]
    Center,
    End,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BarConfig {
    pub edge: Edge,
    /// Bar thickness in px (width for vertical bars, height for horizontal).
    pub thickness: i32,
    /// Long-axis size: "full" (fill the edge) or a pixel count.
    pub length: BarLength,
    /// Position of a fixed-length bar along its edge (start/center/end).
    pub align: Align,
    pub monitor: i32,
    /// Continuous graph scrolling (smoother, but redraws per frame). Off = stepped.
    pub smooth: bool,
    /// Spikiness of autoscaled graphs (cpu/gpu/net/disk): >1 sharpens peaks and
    /// deepens valleys, 1.0 = linear. Fixed meters (mem/temp) ignore this.
    pub graph_gamma: f64,
    /// Background opacity 0.0 (transparent) .. 1.0 (opaque).
    pub opacity: f64,
}

impl Default for BarConfig {
    fn default() -> Self {
        Self {
            edge: Edge::Right,
            thickness: 150,
            length: BarLength::Full,
            align: Align::Center,
            monitor: 0,
            smooth: true,
            graph_gamma: 2.0,
            opacity: 0.88,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PowerConfig {
    /// On battery, every panel's interval is multiplied by this (battery-first).
    pub battery_interval_multiplier: f64,
}

impl Default for PowerConfig {
    fn default() -> Self {
        Self {
            battery_interval_multiplier: 2.0,
        }
    }
}

/// System-tray layout knobs.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TrayConfig {
    /// Max icons per row before wrapping to a new line.
    pub columns: i32,
    /// Tray icon size in px.
    pub icon_size: i32,
}

impl Default for TrayConfig {
    fn default() -> Self {
        Self {
            columns: 8,
            icon_size: 18,
        }
    }
}

/// Shell commands run by the header buttons (one-shot, on click — not polled).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Actions {
    pub lock: String,
    pub logout: String,
    pub reboot: String,
    pub shutdown: String,
    /// Launched when the volume meter is right-clicked.
    pub mixer: String,
}

impl Default for Actions {
    fn default() -> Self {
        Self {
            lock: "loginctl lock-session".into(),
            logout: "loginctl terminate-session \"$XDG_SESSION_ID\"".into(),
            reboot: "systemctl reboot".into(),
            shutdown: "systemctl poweroff".into(),
            mixer: "pavucontrol".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PanelConfig {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default = "default_interval", deserialize_with = "de_secs")]
    pub interval: f64,
    #[serde(default = "default_true")]
    pub graph: bool,
    /// clock only: strftime time/date formats (defaults give 12-hour AM/PM).
    #[serde(default)]
    pub time_format: Option<String>,
    #[serde(default)]
    pub date_format: Option<String>,
}

fn default_interval() -> f64 {
    1.0
}
fn default_true() -> bool {
    true
}

/// Accept either an integer or float for interval seconds.
#[derive(Deserialize)]
#[serde(untagged)]
enum Num {
    Int(i64),
    Float(f64),
}

fn de_secs<'de, D: serde::Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
    Ok(match Num::deserialize(d)? {
        Num::Int(i) => i as f64,
        Num::Float(f) => f,
    })
}

fn default_panels() -> Vec<PanelConfig> {
    [
        "header", "cpu", "mem", "gpu", "disk", "net", "temp", "bat", "vol", "bri", "top", "win",
        "tray",
    ]
    .iter()
    .map(|k| PanelConfig {
        kind: (*k).to_string(),
        interval: 1.0,
        graph: true,
        time_format: None,
        date_format: None,
    })
    .collect()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Name of the theme to load (`themes/<name>.toml`); "default" = built-in.
    #[serde(default = "default_theme")]
    pub theme: String,
    #[serde(default)]
    pub bar: BarConfig,
    #[serde(default)]
    pub power: PowerConfig,
    #[serde(default)]
    pub tray: TrayConfig,
    #[serde(default)]
    pub actions: Actions,
    #[serde(default = "default_panels")]
    pub panel: Vec<PanelConfig>,
}

fn default_theme() -> String {
    "default".into()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            theme: default_theme(),
            bar: BarConfig::default(),
            power: PowerConfig::default(),
            tray: TrayConfig::default(),
            actions: Actions::default(),
            panel: default_panels(),
        }
    }
}

pub fn config_path() -> PathBuf {
    let base = std::env::var("XDG_CONFIG_HOME")
        .unwrap_or_else(|_| format!("{}/.config", std::env::var("HOME").unwrap_or_default()));
    PathBuf::from(base).join("xerotop").join("config.toml")
}

/// Load config, writing a starter file on first run. Never panics.
pub fn load() -> Config {
    let path = config_path();
    match fs::read_to_string(&path) {
        Ok(s) => toml::from_str(&s).unwrap_or_else(|e| {
            eprintln!("xerotop: config parse error ({e}); using defaults");
            Config::default()
        }),
        Err(_) => {
            if let Some(dir) = path.parent() {
                let _ = fs::create_dir_all(dir);
            }
            let _ = fs::write(&path, DEFAULT_TOML);
            // Parse the file we just wrote so first launch matches every launch
            // after (DEFAULT_TOML's per-panel intervals differ from the struct
            // defaults used by default_panels()).
            toml::from_str(DEFAULT_TOML).unwrap_or_else(|_| Config::default())
        }
    }
}

pub const DEFAULT_TOML: &str = r#"# xerotop — system monitor config (TOML, stable by design)

# Visual theme (colors + font). "default" = built-in; any other name loads
# ~/.config/xerotop/themes/<name>.toml. Edit colors live in the prefs GUI.
theme = "default"

[bar]
edge = "right"      # left | right | top | bottom  (left/right = vertical bar)
thickness = 150     # px: width for vertical bars, height for horizontal
length = "full"     # "full"/"max" to fill the edge, or a pixel count (e.g. 600)
align = "center"    # start | center | end  (only used when length is fixed)
monitor = 0
smooth = true       # continuous graph scrolling; false = stepped (less battery)
graph_gamma = 2.0   # autoscaled-graph spikiness; >1 sharper peaks, 1.0 = linear
opacity = 0.88      # background opacity: 0.0 transparent .. 1.0 opaque

[power]
# On battery, multiply every panel's update interval by this (saves wakeups).
battery_interval_multiplier = 2.0

[tray]
columns = 8         # max tray icons per row before wrapping
icon_size = 18      # tray icon size in px

# Header buttons run these on click (one-shot, not polled).
[actions]
lock = "loginctl lock-session"
logout = "loginctl terminate-session \"$XDG_SESSION_ID\""
reboot = "systemctl reboot"
shutdown = "systemctl poweroff"
mixer = "pavucontrol"   # launched when the volume meter is right-clicked

# Panels render top-to-bottom (vertical) or left-to-right (horizontal).
# interval is seconds and may be fractional (e.g. 0.5 = 2 samples/sec).
[[panel]]
type = "header"            # power button | clock | lock button
interval = 1
time_format = "%I:%M %p"   # 12-hour AM/PM; use "%H:%M" for 24-hour
date_format = "%a %d %b"

[[panel]]
type = "cpu"
interval = 2
graph = true

[[panel]]
type = "mem"
interval = 1
graph = true

[[panel]]
type = "gpu"
interval = 2
graph = true

[[panel]]
type = "disk"
interval = 2
graph = true

[[panel]]
type = "net"
interval = 2
graph = true

[[panel]]
type = "temp"
interval = 2
graph = true

[[panel]]
type = "bat"
interval = 10

[[panel]]
type = "vol"
interval = 1

[[panel]]
type = "bri"
interval = 1

[[panel]]
type = "top"
interval = 3

[[panel]]
type = "win"        # taskbar (open windows via wlr-foreign-toplevel)

[[panel]]
type = "tray"       # system tray (StatusNotifier)
"#;
