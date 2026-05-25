//! Native metric samplers. All read /proc, /sys, or a system API directly — no
//! subprocesses, which is the whole point (the ewwii version forked ~600
//! shells/sec). Volume uses ALSA; disk usage uses statvfs(3).

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::time::Instant;

fn read_u64(p: &Path) -> u64 {
    fs::read_to_string(p)
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

/// CPU busy percentage, computed from deltas of /proc/stat.
pub struct Cpu {
    prev: (u64, u64), // (idle, total)
}

impl Cpu {
    pub fn new() -> Self {
        Self { prev: Self::raw() }
    }

    fn raw() -> (u64, u64) {
        let stat = fs::read_to_string("/proc/stat").unwrap_or_default();
        let line = stat.lines().next().unwrap_or("");
        let vals: Vec<u64> = line
            .split_whitespace()
            .skip(1)
            .filter_map(|v| v.parse().ok())
            .collect();
        let idle = vals.get(3).copied().unwrap_or(0) + vals.get(4).copied().unwrap_or(0);
        let total: u64 = vals.iter().sum();
        (idle, total)
    }

    /// Busy % since last call, 0..=100.
    pub fn sample(&mut self) -> f64 {
        let (idle, total) = Self::raw();
        let di = idle.saturating_sub(self.prev.0) as f64;
        let dt = total.saturating_sub(self.prev.1) as f64;
        self.prev = (idle, total);
        if dt > 0.0 {
            ((1.0 - di / dt) * 100.0).clamp(0.0, 100.0)
        } else {
            0.0
        }
    }
}

/// Memory breakdown from /proc/meminfo: (used%, cache%). "used" follows the
/// ewwii script: (MemTotal - MemAvailable) / MemTotal.
pub fn mem_detail() -> (f64, f64) {
    let info = fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let field = |key: &str| -> f64 {
        for line in info.lines() {
            let mut it = line.split_whitespace();
            if it.next() == Some(key) {
                return it.next().and_then(|v| v.parse().ok()).unwrap_or(0.0);
            }
        }
        0.0
    };
    let total = field("MemTotal:");
    if total <= 0.0 {
        return (0.0, 0.0);
    }
    let cache = field("Buffers:") + field("Cached:") + field("SReclaimable:");
    let available = field("MemAvailable:");
    let used = if available > 0.0 {
        total - available
    } else {
        total - field("MemFree:") - cache
    }
    .max(0.0);
    (used / total * 100.0, cache / total * 100.0)
}

/// First hwmon whose `name` matches one of `names`.
fn hwmon_named(names: &[&str]) -> Option<std::path::PathBuf> {
    let mut paths: Vec<_> = fs::read_dir("/sys/class/hwmon")
        .ok()?
        .flatten()
        .map(|e| e.path())
        .collect();
    paths.sort();
    paths.into_iter().find(|p| {
        let n = fs::read_to_string(p.join("name")).unwrap_or_default();
        names.contains(&n.trim())
    })
}

/// Temperature (°C) of the first hwmon matching `names`.
fn temp_named(names: &[&str]) -> Option<f64> {
    read_temp_input(&hwmon_named(names)?)
}

pub fn temp_cpu() -> Option<f64> {
    temp_named(&["k10temp", "coretemp", "zenpower"])
}
pub fn temp_gpu() -> Option<f64> {
    temp_named(&["amdgpu", "radeon", "nouveau", "nvidia"])
}
pub fn temp_ssd() -> Option<f64> {
    temp_named(&["nvme", "drivetemp"])
}

/// First nonzero fan speed (RPM) found in hwmon.
pub fn fan_rpm() -> Option<f64> {
    for e in fs::read_dir("/sys/class/hwmon").ok()?.flatten() {
        for i in 1..=8 {
            if let Ok(s) = fs::read_to_string(e.path().join(format!("fan{i}_input")))
                && let Ok(v) = s.trim().parse::<f64>()
                && v > 0.0
            {
                return Some(v);
            }
        }
    }
    None
}

fn read_temp_input(dir: &Path) -> Option<f64> {
    for i in 1..=8 {
        if let Ok(s) = fs::read_to_string(dir.join(format!("temp{i}_input")))
            && let Ok(milli) = s.trim().parse::<f64>()
        {
            return Some(milli / 1000.0);
        }
    }
    None
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SensorKind {
    Temp,
    Fan,
}

/// A discovered hwmon sensor (temperature or fan).
pub struct SensorInfo {
    pub chip: String,          // hwmon `name`, e.g. "k10temp"
    pub input: String,         // e.g. "temp1" or "fan1"
    pub label: Option<String>, // hwmon `<input>_label`, e.g. "Tctl"
    pub kind: SensorKind,
    pub value: f64, // °C for temps, rpm for fans
}

fn input_kind(input: &str) -> SensorKind {
    if input.starts_with("fan") {
        SensorKind::Fan
    } else {
        SensorKind::Temp
    }
}

fn read_input(dir: &Path, input: &str) -> Option<f64> {
    let v: f64 = fs::read_to_string(dir.join(format!("{input}_input")))
        .ok()?
        .trim()
        .parse()
        .ok()?;
    Some(match input_kind(input) {
        SensorKind::Temp => v / 1000.0,
        SensorKind::Fan => v,
    })
}

fn hwmon_dirs() -> Vec<std::path::PathBuf> {
    let mut dirs: Vec<_> = fs::read_dir("/sys/class/hwmon")
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .collect();
    dirs.sort();
    dirs
}

/// Enumerate every readable hwmon temp/fan input with its chip name and label.
pub fn list_sensors() -> Vec<SensorInfo> {
    let mut out = Vec::new();
    for dir in hwmon_dirs() {
        let chip = fs::read_to_string(dir.join("name"))
            .unwrap_or_default()
            .trim()
            .to_string();
        if chip.is_empty() {
            continue;
        }
        let Ok(rd) = fs::read_dir(&dir) else { continue };
        let mut inputs: Vec<String> = rd
            .flatten()
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|n| (n.starts_with("temp") || n.starts_with("fan")) && n.ends_with("_input"))
            .map(|n| n.trim_end_matches("_input").to_string())
            .collect();
        inputs.sort();
        for input in inputs {
            let Some(value) = read_input(&dir, &input) else {
                continue;
            };
            let label = fs::read_to_string(dir.join(format!("{input}_label")))
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            out.push(SensorInfo {
                chip: chip.clone(),
                kind: input_kind(&input),
                input,
                label,
                value,
            });
        }
    }
    out
}

/// Read a sensor by chip name + input (e.g. "k10temp","temp1"). Resolves by the
/// chip's `name` because hwmonN numbers aren't stable across boots.
pub fn read_sensor(chip: &str, input: &str) -> Option<f64> {
    for dir in hwmon_dirs() {
        if fs::read_to_string(dir.join("name"))
            .unwrap_or_default()
            .trim()
            == chip
            && let Some(v) = read_input(&dir, input)
        {
            return Some(v);
        }
    }
    None
}

/// (percent, status) of the first BAT* supply, if any (None on desktops).
pub fn battery() -> Option<(f64, String)> {
    let entries = fs::read_dir("/sys/class/power_supply").ok()?;
    for entry in entries.flatten() {
        let p = entry.path();
        if fs::read_to_string(p.join("type"))
            .unwrap_or_default()
            .trim()
            == "Battery"
        {
            let cap = fs::read_to_string(p.join("capacity"))
                .ok()?
                .trim()
                .parse::<f64>()
                .ok()?;
            let status = fs::read_to_string(p.join("status"))
                .unwrap_or_default()
                .trim()
                .to_string();
            return Some((cap, status));
        }
    }
    None
}

/// Backlight brightness percentage from the first /sys/class/backlight device.
pub fn brightness() -> Option<f64> {
    let dir = fs::read_dir("/sys/class/backlight")
        .ok()?
        .flatten()
        .next()?
        .path();
    let cur: f64 = fs::read_to_string(dir.join("brightness"))
        .ok()?
        .trim()
        .parse()
        .ok()?;
    let max: f64 = fs::read_to_string(dir.join("max_brightness"))
        .ok()?
        .trim()
        .parse()
        .ok()?;
    (max > 0.0).then_some(cur / max * 100.0)
}

/// (volume %, muted) of the ALSA `Master` mixer on the default card.
pub fn volume() -> Option<(f64, bool)> {
    use alsa::mixer::{Mixer, SelemChannelId, SelemId};
    let mixer = Mixer::new("default", false).ok()?;
    let selem = mixer.find_selem(&SelemId::new("Master", 0))?;
    let (min, max) = selem.get_playback_volume_range();
    let raw = selem.get_playback_volume(SelemChannelId::FrontLeft).ok()?;
    let pct = if max > min {
        (raw - min) as f64 / (max - min) as f64 * 100.0
    } else {
        0.0
    };
    let muted = selem
        .get_playback_switch(SelemChannelId::FrontLeft)
        .map(|s| s == 0)
        .unwrap_or(false);
    Some((pct, muted))
}

/// Adjust ALSA Master volume by `delta_pct` (e.g. +5.0 / -5.0). Native, no subprocess.
pub fn add_volume(delta_pct: f64) {
    use alsa::mixer::{Mixer, SelemChannelId, SelemId};
    let Ok(mixer) = Mixer::new("default", false) else {
        return;
    };
    let Some(selem) = mixer.find_selem(&SelemId::new("Master", 0)) else {
        return;
    };
    let (min, max) = selem.get_playback_volume_range();
    let cur = selem
        .get_playback_volume(SelemChannelId::FrontLeft)
        .unwrap_or(min);
    let span = (max - min) as f64;
    let new = ((cur as f64 + delta_pct / 100.0 * span).round() as i64).clamp(min, max);
    let _ = selem.set_playback_volume_all(new);
}

/// Toggle ALSA Master mute.
pub fn toggle_mute() {
    use alsa::mixer::{Mixer, SelemChannelId, SelemId};
    let Ok(mixer) = Mixer::new("default", false) else {
        return;
    };
    let Some(selem) = mixer.find_selem(&SelemId::new("Master", 0)) else {
        return;
    };
    let on = selem
        .get_playback_switch(SelemChannelId::FrontLeft)
        .unwrap_or(1);
    let _ = selem.set_playback_switch_all(i32::from(on == 0));
}

/// Adjust backlight by `delta_pct` via brightnessctl (the /sys node is
/// root-owned, so a direct write fails; brightnessctl handles perms via
/// logind/udev). One-shot on a user scroll — not polled.
pub fn add_brightness(delta_pct: f64) {
    let step = delta_pct.abs().round() as i64;
    let arg = if delta_pct >= 0.0 {
        format!("{step}%+")
    } else {
        format!("{step}%-")
    };
    let _ = std::process::Command::new("brightnessctl")
        .arg("set")
        .arg(arg)
        .spawn();
}

/// (busy %, vram used GB, vram total GB) from the first GPU exposing busy %.
pub fn gpu() -> Option<(f64, f64, f64)> {
    let entries = fs::read_dir("/sys/class/drm").ok()?;
    for e in entries.flatten() {
        let dev = e.path().join("device");
        if let Ok(b) = fs::read_to_string(dev.join("gpu_busy_percent")) {
            let busy = b.trim().parse::<f64>().unwrap_or(0.0);
            let used = read_u64(&dev.join("mem_info_vram_used")) as f64 / 1e9;
            let total = read_u64(&dev.join("mem_info_vram_total")) as f64 / 1e9;
            return Some((busy, used, total));
        }
    }
    None
}

/// Disk usage of `/`: (percent, used GB, total GB) via statvfs(3).
pub fn disk_usage() -> Option<(f64, f64, f64)> {
    let path = std::ffi::CString::new("/").ok()?;
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(path.as_ptr(), &mut s) } != 0 {
        return None;
    }
    let frsize = s.f_frsize as f64;
    let total = s.f_blocks as f64 * frsize;
    let avail = s.f_bavail as f64 * frsize;
    let used = total - avail;
    (total > 0.0).then_some((used / total * 100.0, used / 1e9, total / 1e9))
}

/// Network throughput in KB/s for the default-route interface, falling back to
/// all non-loopback interfaces when no default route is visible.
pub struct Net {
    prev: (u64, u64),
    at: Instant,
}

impl Net {
    pub fn new() -> Self {
        Self {
            prev: Self::raw(),
            at: Instant::now(),
        }
    }

    fn default_iface() -> Option<String> {
        let routes = fs::read_to_string("/proc/net/route").ok()?;
        routes.lines().skip(1).find_map(|line| {
            let cols: Vec<&str> = line.split_whitespace().collect();
            (cols.get(1) == Some(&"00000000")).then(|| cols[0].to_string())
        })
    }

    fn raw() -> (u64, u64) {
        let dev = fs::read_to_string("/proc/net/dev").unwrap_or_default();
        let default_iface = Self::default_iface();
        let (mut rx, mut tx) = (0u64, 0u64);
        for line in dev.lines().skip(2) {
            let Some((iface, rest)) = line.split_once(':') else {
                continue;
            };
            let iface = iface.trim();
            if iface == "lo" {
                continue;
            }
            if let Some(default_iface) = &default_iface
                && iface != default_iface
            {
                continue;
            }
            let cols: Vec<u64> = rest
                .split_whitespace()
                .filter_map(|v| v.parse().ok())
                .collect();
            rx += cols.first().copied().unwrap_or(0); // rx bytes
            tx += cols.get(8).copied().unwrap_or(0); // tx bytes
        }
        (rx, tx)
    }

    /// (down, up) in KB/s since last call.
    pub fn sample(&mut self) -> (f64, f64) {
        let (rx, tx) = Self::raw();
        let secs = self.at.elapsed().as_secs_f64();
        let drx = rx.saturating_sub(self.prev.0) as f64;
        let dtx = tx.saturating_sub(self.prev.1) as f64;
        self.prev = (rx, tx);
        self.at = Instant::now();
        if secs > 0.0 {
            (drx / 1024.0 / secs, dtx / 1024.0 / secs)
        } else {
            (0.0, 0.0)
        }
    }
}

/// Disk throughput in KB/s, preferring physical whole-disk block devices and
/// falling back to all whole disks when running in a virtualized/container view.
pub struct Disk {
    prev: (u64, u64), // (sectors read, sectors written)
    at: Instant,
}

impl Disk {
    pub fn new() -> Self {
        Self {
            prev: Self::raw(),
            at: Instant::now(),
        }
    }

    fn raw() -> (u64, u64) {
        let stats = fs::read_to_string("/proc/diskstats").unwrap_or_default();
        let (mut all_rd, mut all_wr) = (0u64, 0u64);
        let (mut physical_rd, mut physical_wr) = (0u64, 0u64);
        let mut found_physical = false;
        for line in stats.lines() {
            let f: Vec<&str> = line.split_whitespace().collect();
            if f.len() < 10 {
                continue;
            }
            let sys = Path::new("/sys/block").join(f[2]);
            // Whole disks live in /sys/block; partitions are nested under them.
            if !sys.exists() {
                continue;
            }
            let rd = f[5].parse().unwrap_or(0);
            let wr = f[9].parse().unwrap_or(0);
            all_rd += rd;
            all_wr += wr;
            // Prefer physical devices, so dm-crypt/LVM/loop/zram do not double-count
            // IO already represented by the underlying disk.
            if sys.join("device").exists() {
                physical_rd += rd;
                physical_wr += wr;
                found_physical = true;
            }
        }
        if found_physical {
            (physical_rd, physical_wr)
        } else {
            (all_rd, all_wr)
        }
    }

    /// (read, write) in KB/s since last call (sectors are 512 bytes).
    pub fn sample(&mut self) -> (f64, f64) {
        let (rd, wr) = Self::raw();
        let secs = self.at.elapsed().as_secs_f64();
        let drd = rd.saturating_sub(self.prev.0) as f64 * 512.0;
        let dwr = wr.saturating_sub(self.prev.1) as f64 * 512.0;
        self.prev = (rd, wr);
        self.at = Instant::now();
        if secs > 0.0 {
            (drd / 1024.0 / secs, dwr / 1024.0 / secs)
        } else {
            (0.0, 0.0)
        }
    }
}

fn total_jiffies() -> u64 {
    fs::read_to_string("/proc/stat")
        .unwrap_or_default()
        .lines()
        .next()
        .unwrap_or("")
        .split_whitespace()
        .skip(1)
        .filter_map(|v| v.parse::<u64>().ok())
        .sum()
}

fn num_cpus() -> f64 {
    let stat = fs::read_to_string("/proc/stat").unwrap_or_default();
    stat.lines()
        .filter(|l| l.len() > 3 && l.starts_with("cpu") && l.as_bytes()[3].is_ascii_digit())
        .count()
        .max(1) as f64
}

/// Top processes by instantaneous CPU %, from per-PID /proc deltas (no `ps`).
pub struct Top {
    prev: HashMap<i32, u64>, // pid -> utime+stime ticks
    ema: HashMap<i32, f64>,  // pid -> smoothed cpu %
    prev_total: u64,
    ncpu: f64,
}

impl Top {
    pub fn new() -> Self {
        Self {
            prev: HashMap::new(),
            ema: HashMap::new(),
            prev_total: total_jiffies(),
            ncpu: num_cpus(),
        }
    }

    /// Returns up to `n` (command, cpu%) pairs, busiest first. 100% = one core.
    /// Values are exponentially smoothed so the list doesn't jump every sample.
    pub fn sample(&mut self, n: usize) -> Vec<(String, f64)> {
        const ALPHA: f64 = 0.3; // lower = smoother/calmer

        let total = total_jiffies();
        let dtotal = total.saturating_sub(self.prev_total).max(1) as f64;
        self.prev_total = total;

        let mut cur = HashMap::new();
        let mut ema = HashMap::new();
        let mut list: Vec<(String, f64)> = Vec::new();
        if let Ok(entries) = fs::read_dir("/proc") {
            for e in entries.flatten() {
                let Some(pid) = e.file_name().to_str().and_then(|s| s.parse::<i32>().ok()) else {
                    continue;
                };
                let Ok(stat) = fs::read_to_string(e.path().join("stat")) else {
                    continue;
                };
                // comm is parenthesized and may contain spaces/')'
                let (Some(open), Some(close)) = (stat.find('('), stat.rfind(')')) else {
                    continue;
                };
                let comm = stat[open + 1..close].to_string();
                let rest: Vec<&str> = stat[close + 1..].split_whitespace().collect();
                // post-comm fields: utime = field 14 -> index 11, stime = 15 -> 12
                let utime: u64 = rest.get(11).and_then(|v| v.parse().ok()).unwrap_or(0);
                let stime: u64 = rest.get(12).and_then(|v| v.parse().ok()).unwrap_or(0);
                let ticks = utime + stime;
                // include every process seen last round (even idle ones) so their
                // smoothed value decays gracefully instead of vanishing.
                if let Some(&p) = self.prev.get(&pid) {
                    let inst = ticks.saturating_sub(p) as f64 / dtotal * 100.0 * self.ncpu;
                    let prev_e = self.ema.get(&pid).copied().unwrap_or(inst);
                    let e = ALPHA * inst + (1.0 - ALPHA) * prev_e;
                    ema.insert(pid, e);
                    list.push((comm, e));
                }
                cur.insert(pid, ticks);
            }
        }
        self.prev = cur;
        self.ema = ema;
        list.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        list.truncate(n);
        list
    }
}
