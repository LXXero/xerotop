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

/// The wlr-layer-shell layer the bar lives on. `top` (default) draws above
/// normal windows; `bottom`/`background` let windows be dragged over the bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Layer {
    Background,
    Bottom,
    #[default]
    Top,
    Overlay,
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
    /// Stacking layer: top (above windows) / bottom / background / overlay.
    pub layer: Layer,
    pub monitor: i32,
    /// Continuous graph scrolling on AC (smoother, but redraws per frame).
    /// Off = stepped.
    pub smooth: bool,
    /// Continuous graph scrolling while on battery. Default off — stepped
    /// graphs have no per-frame wakeups. The bar rebuilds on AC<->battery
    /// transitions to switch between this and `smooth`.
    pub smooth_battery: bool,
    /// Thickness (px) of the level meter bars (bat/vol/bri/disk/sensor fans).
    pub meter_thickness: i32,
    /// Spikiness of autoscaled graphs: 1.0 matches ewwii, >1 sharpens peaks
    /// and deepens valleys.
    pub graph_gamma: f64,
    /// Background opacity 0.0 (transparent) .. 1.0 (opaque).
    pub opacity: f64,
    /// Reverse the panel order. Most useful on a horizontal bar, where the
    /// clock/header conventionally belongs at the end rather than the start.
    #[serde(default)]
    pub reverse: bool,
}

impl Default for BarConfig {
    fn default() -> Self {
        Self {
            edge: Edge::Right,
            thickness: 150,
            length: BarLength::Full,
            align: Align::Center,
            layer: Layer::Top,
            monitor: 0,
            smooth: true,
            smooth_battery: false,
            meter_thickness: 7,
            graph_gamma: 1.0,
            opacity: 0.88,
            reverse: false,
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

/// One sensor shown in the TEMP panel, identified by hwmon chip name + input
/// (stable across reboots, unlike hwmonN numbers).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TempSensor {
    pub chip: String,  // hwmon `name`, e.g. "k10temp"
    pub input: String, // "temp1" / "fan1"
    /// Display label; empty → derived from the chip/hwmon label.
    #[serde(default)]
    pub label: String,
    /// Fill color (hex `#rrggbb`); empty → a default from the palette.
    #[serde(default)]
    pub color: String,
    /// Fan rows only: RPM mapped to a full level bar. 0 → default (FAN_MAX_RPM).
    /// Ignored for temperature rows (those scale 0..100 °C).
    #[serde(default)]
    pub fan_max: f64,
}

/// TEMP panel sensor selection. The list may include a special averaging row
/// (chip = "@avg") positioned anywhere, which averages the other temps.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TempConfig {
    /// Explicit sensor list. Empty → auto-detect cpu/gpu/ssd + first fan.
    #[serde(rename = "sensor")]
    pub sensors: Vec<TempSensor>,
}

/// Weather panel config (fetched from wttr.in).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WeatherConfig {
    /// Location (city / "lat,lon"); empty = auto-locate by IP.
    pub location: String,
    /// "auto" | "c" | "f".
    pub units: String,
    /// Minutes between fetches.
    pub interval_min: f64,
    /// Show the condition text next to the icon+temp (off = compact, tooltip only).
    pub show_condition: bool,
}

impl Default for WeatherConfig {
    fn default() -> Self {
        Self {
            location: String::new(),
            units: "auto".into(),
            interval_min: 30.0,
            show_condition: false,
        }
    }
}

/// One of the four header icon slots: left/right of the time, left/right of the
/// date.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HeaderSlot {
    TimeLeft,
    TimeRight,
    DateLeft,
    DateRight,
}

/// A configurable header icon button in one of the four slots.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeaderButton {
    pub slot: HeaderSlot,
    /// Nerd Font glyph shown on the button.
    pub icon: String,
    /// Click action: "@menu" opens the power popover (logout/reboot/shutdown
    /// from [actions]); anything else is run as a shell command.
    pub command: String,
    /// Icon color (hex `#rrggbb`); empty → the default header/accent color.
    #[serde(default)]
    pub color: String,
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
    /// Show the panel's label/value header row. Off → just the graphic (e.g. a
    /// `cores` panel under `cpu` reads as one block, no repeated "CPU" header).
    #[serde(default = "default_true")]
    pub show_label: bool,
    /// Override the graph height (px) for this panel. None = the panel-type
    /// default (tall for cpu/mem/…; short for cores/sensors trends).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graph_height: Option<i32>,
    /// Override the graph's time window (seconds of history shown). None = the
    /// built-in ~60s.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graph_window: Option<f64>,
    /// Override the graph's width (px). Only used on a horizontal bar, where
    /// graphs are fixed-width (so fluctuating value text can't resize the
    /// panel); a vertical bar always fills the width. None = a sensible default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graph_width: Option<i32>,
    /// top panel only: how many processes to list. None = default (5).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<usize>,
    /// uptime panel only: also show the 1/5/15-min load averages.
    #[serde(default)]
    pub show_load: bool,
    /// vol/bri panels only: scroll-wheel step in % (None = vol 2, bri 5).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scroll_step: Option<f64>,
    /// clock only: strftime time/date formats (defaults give 12-hour AM/PM).
    #[serde(default)]
    pub time_format: Option<String>,
    #[serde(default)]
    pub date_format: Option<String>,
    /// net panel only: rescale transfer speeds to K/M/G once the raw KB/s would
    /// exceed 4 digits, so the panel stops stretching on big transfers. None or
    /// true = humanize; false = always raw KB/s.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub human_readable: Option<bool>,
    /// tasks panel only: drop window titles and lay the app icons out in a grid
    /// (dock/macOS-style) instead of icon+title rows.
    #[serde(default)]
    pub icons_only: bool,
    /// tasks/tray-style panels: icons per row before wrapping. tasks only uses
    /// it in `icons_only` mode. None = 1 (a single column).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub columns: Option<i32>,
    /// tasks panel only: app-icon size in px. None = 16.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_size: Option<i32>,
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

/// MAIL panel: maildir unread/total + a click action. Hidden if no maildir.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MailConfig {
    /// Maildir root (contains new/ + cur/); empty → ~/.maildir. "~/" expands.
    pub dir: String,
    /// Shell command run on click (e.g. open mutt in a terminal).
    pub command: String,
    /// Seconds between recounts (the count runs off the UI thread).
    pub interval_s: f64,
}

impl Default for MailConfig {
    fn default() -> Self {
        Self {
            dir: String::new(),
            command: "xfce4-terminal -e mutt".into(),
            interval_s: 15.0,
        }
    }
}

/// Font-size overrides (px). `None` = use the active theme's size for that tier.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct FontConfig {
    pub small: Option<i32>,
    pub normal: Option<i32>,
    pub large: Option<i32>,
}

fn default_panels() -> Vec<PanelConfig> {
    [
        "header",
        "cpu",
        "memory",
        "gpu",
        "disk",
        "net",
        "sensors",
        "battery",
        "volume",
        "brightness",
        "top",
        "tasks",
        "tray",
    ]
    .iter()
    .map(|k| PanelConfig {
        kind: (*k).to_string(),
        // temp does slow hwmon I/O (now off-thread) and temps move slowly, so
        // it samples less often; everything else stays responsive.
        interval: match *k {
            "sensors" => 5.0,
            "cpu" | "memory" | "gpu" | "disk" | "top" => 2.0,
            "battery" => 10.0,
            _ => 1.0,
        },
        graph: true,
        show_label: true,
        graph_height: None,
        graph_window: None,
        graph_width: None,
        count: None,
        show_load: false,
        scroll_step: None,
        time_format: None,
        date_format: None,
        human_readable: None,
        icons_only: false,
        columns: None,
        icon_size: None,
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
    /// Optional font-size overrides that win over the theme's defaults, so
    /// changing sizes doesn't require editing (or get wiped by) the theme.
    #[serde(default)]
    pub font: FontConfig,
    #[serde(default)]
    pub power: PowerConfig,
    #[serde(default)]
    pub tray: TrayConfig,
    #[serde(default)]
    pub temp: TempConfig,
    #[serde(default)]
    pub weather: WeatherConfig,
    #[serde(default)]
    pub mail: MailConfig,
    /// Header icon buttons (the 4 slots). Empty → default power-menu + lock.
    #[serde(default, rename = "header_button")]
    pub header: Vec<HeaderButton>,
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
            font: FontConfig::default(),
            power: PowerConfig::default(),
            tray: TrayConfig::default(),
            temp: TempConfig::default(),
            weather: WeatherConfig::default(),
            mail: MailConfig::default(),
            header: Vec::new(),
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
/// Sibling backup path for the config (`config.toml.bak`).
pub fn backup_path() -> PathBuf {
    let mut p = config_path().into_os_string();
    p.push(".bak");
    PathBuf::from(p)
}

/// Deserialize a TOML table into `T`, tolerating individual fields whose value
/// a code change made incompatible. Build up from the `required` keys, then add
/// each remaining key only while the result still deserializes — an
/// incompatible key is dropped, so its field falls back to its serde default.
/// Returns `None` only if even the required-key base can't parse.
///
/// This is generic: it probes the *real* struct rather than a hand-maintained
/// field list, so adding a field needs no upkeep here.
fn lenient<T: serde::de::DeserializeOwned>(tbl: &toml::Table, required: &[&str]) -> Option<T> {
    let parses = |t: &toml::Table| -> bool {
        let r: Result<T, _> = toml::Value::Table(t.clone()).try_into();
        r.is_ok()
    };
    let mut acc = toml::Table::new();
    for r in required {
        acc.insert((*r).to_string(), tbl.get(*r)?.clone());
    }
    if !parses(&acc) {
        return None; // the required keys themselves are broken — unsalvageable
    }
    for (k, v) in tbl {
        if required.contains(&k.as_str()) {
            continue;
        }
        let mut probe = acc.clone();
        probe.insert(k.clone(), v.clone());
        if parses(&probe) {
            acc = probe;
        }
    }
    let r: Result<T, _> = toml::Value::Table(acc).try_into();
    r.ok()
}

/// Recover as much as possible from a config that failed a strict parse.
///
/// serde parsing is all-or-nothing: one value whose *type* a code change made
/// incompatible (an old saved value vs a new field type) fails the whole
/// `Config`, which would otherwise reset the bar to the reduced default panel
/// set — silently dropping user-added panels (cores/uptime/…). Instead, recover
/// each section and panel field-by-field (see `lenient`): only the genuinely
/// incompatible *field* falls back to its default, so nothing is lost and no
/// manual restore is needed.
fn salvage(s: &str) -> Config {
    let mut cfg = Config::default();
    let Ok(t) = s.parse::<toml::Table>() else {
        return cfg;
    };
    let tbl = |k: &str| match t.get(k) {
        Some(toml::Value::Table(x)) => Some(x.clone()),
        _ => None,
    };
    if let Some(v) = t.get("theme").and_then(|v| v.as_str()) {
        cfg.theme = v.to_string();
    }
    if let Some(x) = tbl("bar").and_then(|x| lenient(&x, &[])) {
        cfg.bar = x;
    }
    if let Some(x) = tbl("font").and_then(|x| lenient(&x, &[])) {
        cfg.font = x;
    }
    if let Some(x) = tbl("power").and_then(|x| lenient(&x, &[])) {
        cfg.power = x;
    }
    if let Some(x) = tbl("tray").and_then(|x| lenient(&x, &[])) {
        cfg.tray = x;
    }
    if let Some(x) = tbl("temp").and_then(|x| lenient(&x, &[])) {
        cfg.temp = x;
    }
    if let Some(x) = tbl("weather").and_then(|x| lenient(&x, &[])) {
        cfg.weather = x;
    }
    if let Some(x) = tbl("mail").and_then(|x| lenient(&x, &[])) {
        cfg.mail = x;
    }
    if let Some(x) = tbl("actions").and_then(|x| lenient(&x, &[])) {
        cfg.actions = x;
    }
    if let Some(toml::Value::Array(a)) = t.get("header_button") {
        cfg.header = a.iter().filter_map(|h| h.clone().try_into().ok()).collect();
    }
    // Recover every panel, defaulting only its incompatible fields; drop a panel
    // only if even its `type` is unrecoverable. Empty recovery → keep defaults.
    if let Some(toml::Value::Array(a)) = t.get("panel") {
        let kept: Vec<PanelConfig> = a
            .iter()
            .filter_map(|p| match p {
                toml::Value::Table(x) => lenient(x, &["type"]),
                _ => None,
            })
            .collect();
        if !kept.is_empty() {
            cfg.panel = kept;
        }
    }
    cfg
}

pub fn load() -> Config {
    let path = config_path();
    match fs::read_to_string(&path) {
        Ok(s) => toml::from_str(&s).unwrap_or_else(|e| {
            eprintln!("xerotop: config parse error ({e}); salvaging what's valid");
            // Preserve the original before anything (a later Save) can clobber
            // it, then recover every still-valid section/panel rather than
            // resetting to the reduced default set.
            let _ = fs::copy(&path, backup_path());
            salvage(&s)
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
layer = "top"       # top | bottom | background | overlay  (bottom = windows over bar)
monitor = 0
reverse = false     # reverse panel order (handy on horizontal bars: clock at the end)
smooth = true       # continuous graph scrolling on AC; false = stepped (less battery)
smooth_battery = false  # smooth scrolling while on battery (default off = no per-frame wakeups)
graph_gamma = 1.0   # autoscaled-graph spikiness; 1.0 = ewwii, >1 sharper peaks
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
# graph_height = 24  # px (optional)
# graph_window = 60  # seconds of history shown (optional)
# graph_width  = 80  # px, horizontal bars only (fixed so values can't resize it)

[[panel]]
type = "memory"
interval = 2
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
interval = 1
graph = true

[[panel]]
type = "sensors"
interval = 5
graph = true

[[panel]]
type = "battery"
interval = 10

[[panel]]
type = "volume"
interval = 1

[[panel]]
type = "brightness"
interval = 1

[[panel]]
type = "top"
interval = 3
# count = 5          # processes shown
# columns = 1        # horizontal bars: items per column before wrapping
# show_label = true  # the "TOP" header

[[panel]]
type = "tasks"      # open windows via wlr-foreign-toplevel
# icons_only = false # true = dock-style icon grid (titles on hover), no text
# columns = 1        # icons per row before wrapping (icons_only mode)
# icon_size = 16     # app-icon size in px
# show_label = true  # the "TASKS" header row

[[panel]]
type = "tray"       # system tray (StatusNotifier)
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_toml_parses() {
        toml::from_str::<Config>(DEFAULT_TOML).expect("DEFAULT_TOML must parse");
    }

    #[test]
    fn default_toml_intervals_are_what_first_run_gets() {
        // load() writes DEFAULT_TOML then parses it, so first launch must match
        // these per-panel intervals (regression guard for the first-run bug).
        let cfg = toml::from_str::<Config>(DEFAULT_TOML).unwrap();
        let iv = |kind: &str| {
            cfg.panel
                .iter()
                .find(|p| p.kind == kind)
                .map(|p| p.interval)
        };
        assert_eq!(iv("cpu"), Some(2.0));
        assert_eq!(iv("battery"), Some(10.0));
        assert_eq!(iv("top"), Some(3.0));
        assert_eq!(iv("memory"), Some(2.0));
        assert_eq!(iv("net"), Some(1.0));
    }

    #[test]
    fn empty_config_uses_serde_defaults() {
        let cfg = toml::from_str::<Config>("").unwrap();
        assert_eq!(cfg.theme, "default");
        assert_eq!(cfg.bar.thickness, 150);
        assert_eq!(cfg.actions.mixer, "pavucontrol");
        assert!(!cfg.panel.is_empty());
    }

    #[test]
    fn salvage_keeps_the_panel_and_defaults_only_the_bad_field() {
        // `show_load` is a bool; a string is the kind of mismatch a code change
        // (field retype/format change) introduces into an old saved config.
        let toml = "[[panel]]\ntype = \"uptime\"\nshow_load = \"nope\"\ncount = 7\n";
        assert!(toml::from_str::<Config>(toml).is_err()); // strict parse fails
        let cfg = salvage(toml);
        assert_eq!(cfg.panel.len(), 1); // panel preserved, not dropped
        assert_eq!(cfg.panel[0].kind, "uptime");
        assert_eq!(cfg.panel[0].count, Some(7)); // the *valid* field survives
        assert!(!cfg.panel[0].show_load); // only the bad field falls to default
    }

    #[test]
    fn salvage_survives_a_universal_field_retype() {
        // The worst case: a retyped field present on *every* panel. Per-field
        // salvage must keep all panels (defaulting just that field), not reset.
        let toml = "\
[[panel]]\ntype = \"cpu\"\nshow_label = \"x\"\n\
[[panel]]\ntype = \"cores\"\nshow_label = \"x\"\n\
[[panel]]\ntype = \"uptime\"\nshow_label = \"x\"\n";
        assert!(toml::from_str::<Config>(toml).is_err());
        let cfg = salvage(toml);
        let kinds: Vec<&str> = cfg.panel.iter().map(|p| p.kind.as_str()).collect();
        assert_eq!(kinds, ["cpu", "cores", "uptime"]); // all three kept
    }

    #[test]
    fn salvage_isolates_a_bad_section_from_panels() {
        // A bad [bar] value must not cost the user their panels.
        let toml = "\
[bar]\nthickness = \"huge\"\n\
[[panel]]\ntype = \"cpu\"\n\
[[panel]]\ntype = \"cores\"\n";
        assert!(toml::from_str::<Config>(toml).is_err());
        let cfg = salvage(toml);
        assert_eq!(cfg.bar.thickness, 150); // bad section → its default
        assert_eq!(cfg.panel.len(), 2); // panels survive
    }

    #[test]
    fn interval_accepts_int_or_float_or_missing() {
        let int: PanelConfig = toml::from_str("type = \"cpu\"\ninterval = 2").unwrap();
        assert_eq!(int.interval, 2.0);
        let flt: PanelConfig = toml::from_str("type = \"cpu\"\ninterval = 0.5").unwrap();
        assert_eq!(flt.interval, 0.5);
        let none: PanelConfig = toml::from_str("type = \"cpu\"").unwrap();
        assert_eq!(none.interval, 1.0); // default_interval
        assert!(none.graph); // default_true
    }

    #[test]
    fn bar_length_parses_word_or_pixels() {
        let full: BarConfig = toml::from_str("length = \"full\"").unwrap();
        assert_eq!(full.length, BarLength::Full);
        let max: BarConfig = toml::from_str("length = \"max\"").unwrap();
        assert_eq!(max.length, BarLength::Full);
        let px: BarConfig = toml::from_str("length = 600").unwrap();
        assert_eq!(px.length, BarLength::Px(600));
        // round-trips back to a word/number
        assert!(toml::to_string(&full).unwrap().contains("\"full\""));
    }

    #[test]
    fn align_and_layer_parse() {
        let bc: BarConfig = toml::from_str("align = \"end\"\nlayer = \"bottom\"").unwrap();
        assert_eq!(bc.align, Align::End);
        assert_eq!(bc.layer, Layer::Bottom);
    }

    #[test]
    fn config_round_trips_through_toml() {
        // Serialize defaults and parse them back without error (Save path).
        let s = toml::to_string_pretty(&Config::default()).unwrap();
        let back = toml::from_str::<Config>(&s).unwrap();
        assert_eq!(back.bar.opacity, Config::default().bar.opacity);
        assert_eq!(back.panel.len(), Config::default().panel.len());
    }
}
