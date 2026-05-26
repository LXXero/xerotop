//! Maildir unread/total counts. `cur/` can hold tens of thousands of files, so
//! counting runs on a background thread (like weather/tray) and streams a
//! snapshot to GTK — the readdir never stalls the UI loop.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

#[derive(Clone, Default)]
pub struct MailCount {
    pub new: usize,    // unread (Maildir new/)
    pub total: usize,  // new + cur
    pub present: bool, // false → no maildir on this host (panel hides)
}

#[derive(Clone)]
pub struct MailReq {
    pub dir: String, // maildir root; empty → ~/.maildir
    pub interval_s: f64,
}

fn maildir(dir: &str) -> PathBuf {
    if dir.trim().is_empty() {
        let home = std::env::var("HOME").unwrap_or_default();
        PathBuf::from(home).join(".maildir")
    } else if let Some(rest) = dir.strip_prefix("~/") {
        let home = std::env::var("HOME").unwrap_or_default();
        PathBuf::from(home).join(rest)
    } else {
        PathBuf::from(dir)
    }
}

fn count_files(p: &Path) -> usize {
    std::fs::read_dir(p)
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
                .count()
        })
        .unwrap_or(0)
}

fn sample(dir: &str) -> MailCount {
    let base = maildir(dir);
    if !base.join("new").is_dir() {
        return MailCount::default(); // present = false
    }
    let new = count_files(&base.join("new"));
    let total = new + count_files(&base.join("cur"));
    MailCount {
        new,
        total,
        present: true,
    }
}

/// Spawn the mail-counting thread; returns (snapshot receiver, request sender).
/// Send a new `MailReq` to re-point the maildir / interval.
pub fn spawn(initial: MailReq) -> (async_channel::Receiver<MailCount>, mpsc::Sender<MailReq>) {
    let (tx, rx) = async_channel::unbounded::<MailCount>();
    let (req_tx, req_rx) = mpsc::channel::<MailReq>();
    std::thread::spawn(move || {
        let mut req = initial;
        loop {
            if tx.send_blocking(sample(&req.dir)).is_err() {
                break;
            }
            let wait = Duration::from_secs_f64(req.interval_s.max(1.0));
            match req_rx.recv_timeout(wait) {
                Ok(new) => req = new,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
    });
    (rx, req_tx)
}
