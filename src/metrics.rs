//! Native metric samplers. All read /proc, /sys, or a system API directly — no
//! subprocesses, which is the whole point (the ewwii version forked ~600
//! shells/sec). Volume uses ALSA; disk usage uses statvfs(3).

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

/// Memory breakdown from /proc/meminfo: (used%, cache%). "used" excludes
/// reclaimable memory; "cache" = Buffers + Cached + SReclaimable.
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
    let used = (total - field("MemFree:") - cache).max(0.0);
    (used / total * 100.0, cache / total * 100.0)
}

/// CPU temperature in °C from the first preferred hwmon sensor.
pub fn cpu_temp() -> f64 {
    const PREFERRED: [&str; 3] = ["k10temp", "coretemp", "zenpower"];
    let Ok(hwmons) = fs::read_dir("/sys/class/hwmon") else {
        return 0.0;
    };
    let mut paths: Vec<_> = hwmons.flatten().map(|e| e.path()).collect();
    paths.sort();
    for want_preferred in [true, false] {
        for p in &paths {
            let name = fs::read_to_string(p.join("name")).unwrap_or_default();
            let is_preferred = PREFERRED.contains(&name.trim());
            if is_preferred == want_preferred
                && let Some(t) = read_temp_input(p)
            {
                return t;
            }
        }
    }
    0.0
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

/// Adjust backlight by `delta_pct`. Writes /sys directly (needs `video`-group /
/// udev write access; otherwise a no-op).
pub fn add_brightness(delta_pct: f64) {
    let Some(dir) = fs::read_dir("/sys/class/backlight")
        .ok()
        .and_then(|mut e| e.next())
        .and_then(|e| e.ok())
        .map(|e| e.path())
    else {
        return;
    };
    let max = read_u64(&dir.join("max_brightness")) as f64;
    let cur = read_u64(&dir.join("brightness")) as f64;
    if max <= 0.0 {
        return;
    }
    let new = (cur + delta_pct / 100.0 * max).clamp(0.0, max).round() as u64;
    let _ = fs::write(dir.join("brightness"), new.to_string());
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

/// Network throughput in KB/s, summed across non-loopback interfaces.
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

    fn raw() -> (u64, u64) {
        let dev = fs::read_to_string("/proc/net/dev").unwrap_or_default();
        let (mut rx, mut tx) = (0u64, 0u64);
        for line in dev.lines().skip(2) {
            let Some((iface, rest)) = line.split_once(':') else {
                continue;
            };
            if iface.trim() == "lo" {
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

/// Disk throughput in KB/s, summed across whole-disk block devices.
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
        let (mut rd, mut wr) = (0u64, 0u64);
        for line in stats.lines() {
            let f: Vec<&str> = line.split_whitespace().collect();
            if f.len() < 10 {
                continue;
            }
            // whole disks live in /sys/block; partitions are nested under them
            if !Path::new("/sys/block").join(f[2]).exists() {
                continue;
            }
            rd += f[5].parse().unwrap_or(0); // sectors read
            wr += f[9].parse().unwrap_or(0); // sectors written
        }
        (rd, wr)
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
