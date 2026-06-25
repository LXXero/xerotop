use gtk::glib;
use gtk::prelude::ObjectExt;
use notify::Watcher;
use std::path::PathBuf;
use std::time::Duration;

use crate::bar::BarHandle;
use crate::prefs;

/// Initialise colour-scheme listener based on the desktop environment.
pub fn init(handle: &BarHandle) {
    let de = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    if de.to_uppercase().contains("KDE") {
        start_kde_watcher(handle);
    } else {
        // Fallback: GNOME, Budgie, Cinnamon, or any other DE that writes
        // org.gnome.desktop.interface color-scheme.
        start_gnome_poller(handle);
    }
}

/// Poll GSettings for the colour-scheme key every 2 seconds.
/// We can't use the "changed" signal because GSettings callbacks need Send+Sync,
/// and BarHandle contains Rc (neither Send nor Sync).
fn start_gnome_poller(handle: &BarHandle) {
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

/// Watch ~/.config/kdedefaults/kdeglobals for writes and switch theme on change.
/// Watches the parent directory to survive atomic renames (KDE writes via
/// temp-file + rename, which changes the inode).
fn start_kde_watcher(handle: &BarHandle) {
    if let Some(target) = kdeglobals_path() {
        let dir = target.parent().unwrap_or(&target).to_path_buf();
        let filename = target
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let (tx, rx) = async_channel::unbounded::<PathBuf>();

        std::thread::spawn(move || watch_kdeglobals_dir(dir, filename, target, tx));

        let h = handle.clone();
        glib::spawn_future_local(async move {
            while let Ok(target) = rx.recv().await {
                if !h.cfg.borrow().theme_switch.auto {
                    continue;
                }
                if let Some(name) = read_kde_color_scheme(&target) {
                    apply(&h, classify(&name));
                }
            }
        });
    }
}

fn kdeglobals_path() -> Option<PathBuf> {
    let base = std::env::var("XDG_CONFIG_HOME")
        .unwrap_or_else(|_| format!("{}/.config", std::env::var("HOME").unwrap_or_default()));
    Some(PathBuf::from(base).join("kdedefaults/kdeglobals"))
}

/// Watch a directory for changes to a specific filename, using inotify-on-dir
/// to survive atomic renames.
fn watch_kdeglobals_dir(
    dir: PathBuf,
    filename: String,
    target: PathBuf,
    tx: async_channel::Sender<PathBuf>,
) {
    let (notify_tx, notify_rx) = std::sync::mpsc::channel::<notify::Result<notify::Event>>();
    let mut watcher = match notify::RecommendedWatcher::new(notify_tx, notify::Config::default()) {
        Ok(w) => w,
        Err(_) => return,
    };
    if watcher.watch(&dir, notify::RecursiveMode::NonRecursive).is_err() {
        return;
    }

    loop {
        // Wait for an event on the target file.
        loop {
            match notify_rx.recv_timeout(Duration::from_secs(10)) {
                Ok(Ok(e))
                    if matches!(e.kind, notify::EventKind::Modify(_) | notify::EventKind::Create(_))
                        && e.paths.iter().any(|p| p.ends_with(&filename)) =>
                {
                    break;
                }
                Ok(_) => continue,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                Err(_) => return,
            }
        }
        // Debounce: KDE writes kdeglobals in bursts during a theme switch.
        let mut deadline = std::time::Instant::now() + Duration::from_millis(200);
        loop {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            match notify_rx.recv_timeout(remaining) {
                Ok(Ok(e))
                    if matches!(e.kind, notify::EventKind::Modify(_) | notify::EventKind::Create(_))
                        && e.paths.iter().any(|p| p.ends_with(&filename)) =>
                {
                    deadline = std::time::Instant::now() + Duration::from_millis(200);
                    continue;
                }
                _ => break,
            }
        }
        let _ = tx.send_blocking(target.clone());
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
    prefs::theme_changed();
}
