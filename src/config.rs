//! Declarative TOML config. Boring and stable on purpose — no config *language*,
//! just data. Missing fields fall back to sane defaults; a starter file is
//! written on first run.

use serde::Deserialize;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
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

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct BarConfig {
    pub edge: Edge,
    /// Bar thickness in px (width for vertical bars, height for horizontal).
    pub thickness: i32,
    pub monitor: i32,
}

impl Default for BarConfig {
    fn default() -> Self {
        Self {
            edge: Edge::Right,
            thickness: 150,
            monitor: 0,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
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

#[derive(Debug, Clone, Deserialize)]
pub struct PanelConfig {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default = "default_interval")]
    pub interval: u32,
    #[serde(default = "default_true")]
    pub graph: bool,
}

fn default_interval() -> u32 {
    1
}
fn default_true() -> bool {
    true
}

fn default_panels() -> Vec<PanelConfig> {
    ["clock", "cpu", "mem", "temp", "net", "bat"]
        .iter()
        .map(|k| PanelConfig {
            kind: (*k).to_string(),
            interval: 1,
            graph: true,
        })
        .collect()
}

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub bar: BarConfig,
    #[serde(default)]
    pub power: PowerConfig,
    #[serde(default = "default_panels")]
    pub panel: Vec<PanelConfig>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            bar: BarConfig::default(),
            power: PowerConfig::default(),
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
            Config::default()
        }
    }
}

pub const DEFAULT_TOML: &str = r#"# xerotop — system monitor config (TOML, stable by design)

[bar]
edge = "right"      # left | right | top | bottom  (left/right = vertical bar)
thickness = 150     # px: width for vertical bars, height for horizontal
monitor = 0

[power]
# On battery, multiply every panel's update interval by this (saves wakeups).
battery_interval_multiplier = 2.0

# Panels render top-to-bottom (vertical) or left-to-right (horizontal).
[[panel]]
type = "clock"
interval = 1

[[panel]]
type = "cpu"
interval = 1
graph = true

[[panel]]
type = "mem"
interval = 2
graph = true

[[panel]]
type = "temp"
interval = 5
graph = true

[[panel]]
type = "net"
interval = 1
graph = true

[[panel]]
type = "bat"
interval = 10
graph = false
"#;
