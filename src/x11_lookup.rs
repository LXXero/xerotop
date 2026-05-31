//! XWayland icon-resolution helper.
//!
//! labwc (and some other wlroots compositors) expose XWayland windows over
//! `wlr-foreign-toplevel-management` with an `app_id` set to the X11 WM_CLASS
//! *instance* — not the *class* — portion. Firefox is the canonical victim:
//! its X11 WM_CLASS is `"Navigator", "Firefox"`, so the foreign-toplevel
//! app_id arrives as `"Navigator"` and matches no icon or .desktop file.
//!
//! When the regular icon lookups (theme + .desktop scan) fail, this helper
//! opens the X display and walks `_NET_CLIENT_LIST` looking for a top-level
//! window whose WM_CLASS instance matches the foreign-toplevel app_id. The
//! window's WM_CLASS *class* portion (which firefox.desktop's StartupWMClass
//! does match) is returned so the caller can retry the lookup.
//!
//! Results are cached so we only hit X once per first-seen unresolved app_id.

use std::cell::RefCell;
use std::collections::HashMap;

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as _};

/// For an XWayland-reported foreign-toplevel `app_id`, ask X11 for the
/// `WM_CLASS` of a matching window and return its *class* portion. Cached.
pub fn class_for_instance(instance: &str) -> Option<String> {
    if instance.is_empty() {
        return None;
    }
    thread_local! {
        static CACHE: RefCell<HashMap<String, Option<String>>> = RefCell::new(HashMap::new());
    }
    let key = instance.to_string();
    if let Some(v) = CACHE.with(|c| c.borrow().get(&key).cloned()) {
        return v;
    }
    let resolved = query(instance);
    CACHE.with(|c| c.borrow_mut().insert(key, resolved.clone()));
    resolved
}

fn query(instance: &str) -> Option<String> {
    // No DISPLAY = no XWayland in this session; nothing to do.
    std::env::var_os("DISPLAY")?;
    let (conn, screen_num) = x11rb::connect(None).ok()?;
    let root = conn.setup().roots.get(screen_num)?.root;
    let net_client_list = conn
        .intern_atom(false, b"_NET_CLIENT_LIST")
        .ok()?
        .reply()
        .ok()?
        .atom;

    // List of all toplevel X11 windows.
    let list = conn
        .get_property(false, root, net_client_list, AtomEnum::WINDOW, 0, u32::MAX)
        .ok()?
        .reply()
        .ok()?;
    let windows = list.value32()?;

    let want = instance.to_ascii_lowercase();
    for win in windows {
        // WM_CLASS = "<instance>\0<class>\0"
        let reply = match conn
            .get_property(false, win, AtomEnum::WM_CLASS, AtomEnum::STRING, 0, u32::MAX)
            .ok()
            .and_then(|c| c.reply().ok())
        {
            Some(r) if !r.value.is_empty() => r,
            _ => continue,
        };
        let mut parts = reply.value.split(|&b| b == 0).filter(|s| !s.is_empty());
        let inst = parts.next().and_then(|b| std::str::from_utf8(b).ok())?;
        let class = parts.next().and_then(|b| std::str::from_utf8(b).ok());
        if inst.eq_ignore_ascii_case(&want)
            && let Some(c) = class
            && !c.is_empty()
        {
            return Some(c.to_string());
        }
    }
    None
}
