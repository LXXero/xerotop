use gtk::glib;
use gtk::prelude::ObjectExt;
use notify::Watcher;
use std::path::PathBuf;
use std::time::Duration;

use crate::bar::BarHandle;

/// Initialise both the GNOME and KDE colour-scheme listeners.
pub fn init(handle: &BarHandle) {
    // GNOME path: poll org.gnome.desktop.interface color-scheme via GSettings.
    // GSettings signal callbacks require Send+Sync (which BarHandle's Rc isn't),
    // so we poll the value every 2 seconds from a main-thread timer instead.
    gnome_poll(handle);

    // KDE path: watch ~/.config/kdeglobals for writes and parse the active
    // ColorScheme name from the [General] section.
    if let Some(path) = kdeglobals_path() {
        let (tx, rx) = async_channel::unbounded::<PathBuf>();

        // Background thread: watch for file modifications and debounce.
        std::thread::spawn(move || watch_kdeglobals(path, tx));

        // Main-thread receiver: switch theme when notified.
        let h = handle.clone();
        glib::spawn_future_local(async move {
            while let Ok(path) = rx.recv().await {
                if !h.cfg.borrow().theme_switch.auto {
                    continue;
                }
                if let Some(name) = read_kde_color_scheme(&path) {
                    apply(&h, classify(&name));
                }
            }
        });
    }
}

/// Poll GSettings for the colour-scheme key every 2 seconds.
/// We can't use the "changed" signal because GSettings callbacks need Send+Sync,
/// and BarHandle contains Rc (neither Send nor Sync).
fn gnome_poll(handle: &BarHandle) {
    let h = handle.clone();
    let mut last: String = String::new();
    glib::timeout_add_local(Duration::from_secs(2), move || {
        let val: String = gtk::gio::Settings::new("org.gnome.desktop.interface")
            .property("color-scheme");
        if val != last {
            last.clone_from(&val);
            if h.cfg.borrow().theme_switch.auto {
                apply(&h, val == "prefer-dark");
            }
        }
        glib::ControlFlow::Continue
    });
}

fn kdeglobals_path() -> Option<PathBuf> {
    let base = std::env::var("XDG_CONFIG_HOME")
        .unwrap_or_else(|_| format!("{}/.config", std::env::var("HOME").unwrap_or_default()));
    Some(PathBuf::from(base).join("kdeglobals"))
}

fn watch_kdeglobals(path: PathBuf, tx: async_channel::Sender<PathBuf>) {
    let (notify_tx, notify_rx) = std::sync::mpsc::channel::<notify::Result<notify::Event>>();
    let mut watcher = match notify::RecommendedWatcher::new(notify_tx, notify::Config::default()) {
        Ok(w) => w,
        Err(_) => return,
    };
    if watcher
        .watch(&path, notify::RecursiveMode::NonRecursive)
        .is_err()
    {
        return;
    }

    loop {
        // Wait for any modify event.
        loop {
            match notify_rx.recv_timeout(Duration::from_secs(10)) {
                Ok(Ok(e)) if matches!(e.kind, notify::EventKind::Modify(_)) => break,
                Ok(_) => continue,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                Err(_) => return,
            }
        }
        // Debounce: KDE writes kdeglobals in bursts during a theme switch.
        // Keep extending the deadline as long as new events keep arriving.
        let mut deadline = std::time::Instant::now() + Duration::from_millis(200);
        loop {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            match notify_rx.recv_timeout(remaining) {
                Ok(Ok(e)) if matches!(e.kind, notify::EventKind::Modify(_)) => {
                    deadline = std::time::Instant::now() + Duration::from_millis(200);
                    continue;
                }
                _ => break,
            }
        }
        let _ = tx.send_blocking(path.clone());
    }
}

fn read_kde_color_scheme(path: &PathBuf) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    let mut in_general = false;
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_general = line.eq_ignore_ascii_case("[general]");
            continue;
        }
        if in_general && line.starts_with("ColorScheme=") {
            return Some(line["ColorScheme=".len()..].to_string());
        }
    }
    None
}

/// Heuristic: if the KDE colour-scheme name contains "dark" (case-insensitive),
/// treat it as a dark theme.  Works for all built-in KDE schemes (BreezeDark,
/// BreezeDarkBlue, …).  Custom schemes that don't follow this convention are a
/// known blind spot — the user should configure their mapping explicitly.
fn classify(scheme: &str) -> bool {
    scheme.to_lowercase().contains("dark")
}

/// Replicate the theme-switch logic from the prefs selector callback.
fn apply(handle: &BarHandle, is_dark: bool) {
    let name = if is_dark {
        handle.cfg.borrow().theme_switch.dark.clone()
    } else {
        handle.cfg.borrow().theme_switch.light.clone()
    };
    let t = crate::theme::resolve(&name);
    handle.cfg.borrow_mut().theme = name;
    if let Some(s) = &t.sensors {
        handle.cfg.borrow_mut().temp.sensors = s.clone();
    }
    if let Some(hdr) = &t.header {
        handle.cfg.borrow_mut().header = hdr.clone();
    }
    *handle.theme.borrow_mut() = t;
    handle.apply();
}
