//! Native metric samplers. All read /proc or /sys directly — no subprocesses,
//! which is the whole point (the ewwii version forked ~600 shells/sec).

use std::fs;
use std::path::Path;
use std::time::Instant;

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
        // user nice system idle iowait irq softirq steal ...
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

/// Used-memory percentage from /proc/meminfo.
pub fn mem_percent() -> f64 {
    let info = fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let mut total: f64 = 0.0;
    let mut avail: f64 = 0.0;
    for line in info.lines() {
        let mut it = line.split_whitespace();
        match it.next() {
            Some("MemTotal:") => total = it.next().and_then(|v| v.parse().ok()).unwrap_or(0.0),
            Some("MemAvailable:") => avail = it.next().and_then(|v| v.parse().ok()).unwrap_or(0.0),
            _ => {}
        }
    }
    if total > 0.0 {
        ((1.0 - avail / total) * 100.0).clamp(0.0, 100.0)
    } else {
        0.0
    }
}

/// CPU temperature in °C, from the first preferred hwmon sensor (falling back
/// to any sensor that exposes a temperature input).
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

/// Network throughput in KB/s, summed across non-loopback interfaces.
pub struct Net {
    prev: (u64, u64), // (rx, tx) bytes
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
        let mut rx = 0u64;
        let mut tx = 0u64;
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
            // rx bytes = col 0, tx bytes = col 8
            rx += cols.first().copied().unwrap_or(0);
            tx += cols.get(8).copied().unwrap_or(0);
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
