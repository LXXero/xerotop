use std::cell::RefCell;

use gtk::glib;

use crate::bar::BarHandle;
use crate::prefs;
use crate::theme::Theme;

/// How long to wait for the portal's `Read` reply before giving up.
const READ_TIMEOUT_MS: i32 = 1000;

thread_local! {
    /// Session-bus connection opened once by `init()` and reused for every
    /// portal call, so `sync_once` doesn't reconnect each time it runs.
    static CONN: RefCell<Option<gtk::gio::DBusConnection>> = const { RefCell::new(None) };
}

/// Initialise colour-scheme listener via the D-Bus portal and apply the
/// current scheme immediately so the bar and prefs window match from startup.
pub fn init(handle: &BarHandle) {
    let address = match std::env::var("DBUS_SESSION_BUS_ADDRESS") {
        Ok(a) if !a.is_empty() => a,
        _ => return,
    };
    let conn = match gtk::gio::DBusConnection::for_address_sync(
        &address,
        gtk::gio::DBusConnectionFlags::AUTHENTICATION_CLIENT
            | gtk::gio::DBusConnectionFlags::MESSAGE_BUS_CONNECTION,
        None::<&gtk::gio::DBusAuthObserver>,
        None::<&gtk::gio::Cancellable>,
    ) {
        Ok(c) => c,
        Err(_) => return,
    };
    subscribe_signals(handle, &conn);
    CONN.with(|c| *c.borrow_mut() = Some(conn));
    sync_once(handle);
}

/// Read the current colour-scheme via the portal and apply it.
/// The call is asynchronous so a slow or missing portal never blocks the
/// GTK main thread; the reply is applied from the callback.
pub(crate) fn sync_once(handle: &BarHandle) {
    if !handle.cfg.borrow().theme_switch.auto {
        return;
    }
    let Some(conn) = CONN.with(|c| c.borrow().clone()) else {
        return;
    };
    let params = glib::Variant::tuple_from_iter([
        glib::Variant::from("org.freedesktop.appearance"),
        glib::Variant::from("color-scheme"),
    ]);
    let h = handle.clone();
    conn.call(
        Some("org.freedesktop.portal.Desktop"),
        "/org/freedesktop/portal/desktop",
        "org.freedesktop.portal.Settings",
        "Read",
        Some(&params),
        None::<&glib::VariantTy>,
        gtk::gio::DBusCallFlags::NONE,
        READ_TIMEOUT_MS,
        None::<&gtk::gio::Cancellable>,
        move |reply| {
            // The setting may have been turned off while the call was pending.
            if !h.cfg.borrow().theme_switch.auto {
                return;
            }
            if let Some(is_dark) = reply.ok().as_ref().and_then(read_is_dark) {
                apply(&h, is_dark);
            }
        },
    );
}

/// Extract the colour-scheme from a `Read` reply, which is `(v)` holding a u32
/// (1 = prefer dark).
fn read_is_dark(reply: &glib::Variant) -> Option<bool> {
    if reply.n_children() < 1 {
        return None;
    }
    unbox(reply.child_value(0)).get::<u32>().map(|v| v == 1)
}

/// Subscribe to the portal's SettingChanged signal for live updates.
/// The returned SignalSubscription is leaked — it must live for the entire
/// process lifetime to keep the D-Bus signal subscription active.
fn subscribe_signals(handle: &BarHandle, conn: &gtk::gio::DBusConnection) {
    let h = handle.clone();
    let sub = conn.subscribe_to_signal::<_>(
        Some("org.freedesktop.portal.Desktop"),
        Some("org.freedesktop.portal.Settings"),
        Some("SettingChanged"),
        Some("/org/freedesktop/portal/desktop"),
        None::<&str>,
        gtk::gio::DBusSignalFlags::NONE,
        move |signal| {
            let params = signal.parameters;
            if params.n_children() < 3 {
                return;
            }
            if params.child_value(0).get::<String>()
                .as_deref() != Some("org.freedesktop.appearance")
                || params.child_value(1).get::<String>()
                    .as_deref() != Some("color-scheme")
            {
                return;
            }
            if !h.cfg.borrow().theme_switch.auto {
                return;
            }
            let is_dark = unbox(params.child_value(2))
                .get::<u32>()
                .is_some_and(|v| v == 1);
            apply(&h, is_dark);
        },
    );
    // Leak: the subscription must live for the entire process lifetime so that
    // SettingChanged signals continue to be received.
    std::mem::forget(sub);
}

/// Return the desktop-environment style label for CSS class selection.
#[allow(dead_code)]
pub fn desktop_style() -> &'static str {
    let de = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    if de.to_uppercase().contains("KDE") {
        "kde"
    } else {
        "gnome"
    }
}

/// Peel off variant-type (`v`) layers until we hit a concrete type.
/// Portal backends nest the colour-scheme payload at varying depths
/// between `Read` replies and `SettingChanged` signals, so a loop
/// is more robust than hardcoding a fixed number of unboxes.
fn unbox(mut v: glib::Variant) -> glib::Variant {
    while v.is_type(glib::VariantTy::VARIANT) {
        v = v.as_variant().expect("type was just verified");
    }
    v
}

/// How much of the bar a theme switch has to redo.
#[derive(Debug, PartialEq)]
enum Refresh {
    /// The resolved theme is already active.
    Nothing,
    /// Only CSS colors/fonts differ — `restyle()` keeps graph history.
    Restyle,
    /// Graph colors (baked into the graphs) or bundled sensor/header colors
    /// differ — needs a full `apply()`.
    Rebuild,
}

fn refresh_for(current_name: &str, current: &Theme, name: &str, next: &Theme) -> Refresh {
    if name == current_name {
        return Refresh::Nothing;
    }
    let same_graphs = current.green == next.green
        && current.cyan == next.cyan
        && current.amber == next.amber
        && current.red == next.red
        && current.violet == next.violet;
    if same_graphs && next.sensors.is_none() && next.header.is_none() {
        Refresh::Restyle
    } else {
        Refresh::Rebuild
    }
}

/// Replicate the theme-switch logic from the prefs selector callback, using the
/// lightest live-apply tier so a no-op switch doesn't reset graph history.
fn apply(handle: &BarHandle, is_dark: bool) {
    let name = if is_dark {
        handle.cfg.borrow().theme_switch.dark.clone()
    } else {
        handle.cfg.borrow().theme_switch.light.clone()
    };
    let t = crate::theme::resolve(&name);
    let refresh = refresh_for(&handle.cfg.borrow().theme, &handle.theme.borrow(), &name, &t);
    if refresh == Refresh::Nothing {
        return;
    }
    handle.cfg.borrow_mut().theme = name;
    if let Some(s) = &t.sensors {
        handle.cfg.borrow_mut().temp.sensors = s.clone();
    }
    if let Some(hdr) = &t.header {
        handle.cfg.borrow_mut().header = hdr.clone();
    }
    *handle.theme.borrow_mut() = t;
    if refresh == Refresh::Restyle {
        handle.restyle();
    } else {
        handle.apply();
    }
    prefs::theme_changed();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unbox_peels_all_variant_layers() {
        let inner = glib::Variant::from(1u32);
        let wrapped_once = glib::Variant::from_variant(&inner);
        let wrapped_twice = glib::Variant::from_variant(&wrapped_once);
        assert_eq!(unbox(inner.clone()).get::<u32>(), Some(1));
        assert_eq!(unbox(wrapped_once).get::<u32>(), Some(1));
        assert_eq!(unbox(wrapped_twice).get::<u32>(), Some(1));
    }

    #[test]
    fn read_is_dark_parses_portal_reply() {
        let reply = |v: u32| {
            let boxed = glib::Variant::from_variant(&glib::Variant::from(v));
            glib::Variant::tuple_from_iter([glib::Variant::from_variant(&boxed)])
        };
        assert_eq!(read_is_dark(&reply(1)), Some(true));
        assert_eq!(read_is_dark(&reply(2)), Some(false));
        assert_eq!(read_is_dark(&reply(0)), Some(false));
        assert_eq!(read_is_dark(&glib::Variant::tuple_from_iter(Vec::<glib::Variant>::new())), None);
    }

    #[test]
    fn refresh_for_picks_lightest_tier() {
        let cur = Theme::default();
        // Same theme name (e.g. light dropdown changed while dark is active).
        assert_eq!(refresh_for("nyz", &cur, "nyz", &cur), Refresh::Nothing);
        // Different theme, only CSS colors/fonts differ.
        let mut css_only = cur.clone();
        css_only.background = "#123456".into();
        css_only.font_family = "Other Font".into();
        assert_eq!(refresh_for("a", &cur, "b", &css_only), Refresh::Restyle);
        // Graph color differs → rebuild.
        let mut graph = cur.clone();
        graph.cyan = "#000000".into();
        assert_eq!(refresh_for("a", &cur, "b", &graph), Refresh::Rebuild);
        // Bundled header colors → rebuild.
        let mut bundled = cur.clone();
        bundled.header = Some(Vec::new());
        assert_eq!(refresh_for("a", &cur, "b", &bundled), Refresh::Rebuild);
    }
}
