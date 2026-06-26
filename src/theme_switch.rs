use gtk::glib;

use crate::bar::BarHandle;
use crate::prefs;

/// Initialise colour-scheme listener via the D-Bus portal and apply the
/// current scheme immediately so the bar and prefs window match from startup.
pub fn init(handle: &BarHandle) {
    sync_once(handle);
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
}

/// Read the current colour-scheme via the portal and apply it.
/// Creates a temporary D-Bus connection for the one-off call.
pub(crate) fn sync_once(handle: &BarHandle) {
    if !handle.cfg.borrow().theme_switch.auto {
        return;
    }
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
    let params = glib::Variant::tuple_from_iter([
        glib::Variant::from("org.freedesktop.appearance"),
        glib::Variant::from("color-scheme"),
    ]);
    if let Ok(reply) = conn.call_sync(
        Some("org.freedesktop.portal.Desktop"),
        "/org/freedesktop/portal/desktop",
        "org.freedesktop.portal.Settings",
        "Read",
        Some(&params),
        None::<&glib::VariantTy>,
        gtk::gio::DBusCallFlags::NONE,
        -1,
        None::<&gtk::gio::Cancellable>,
    ) {
        // reply is (v) — extract the u32 value
        if let Some(val) = reply.child_value(0).get::<u32>() {
            apply(handle, val == 1);
        }
    }
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
            // params.child_value(2) is a variant wrapping a u32
            let is_dark = params
                .child_value(2)
                .child_value(0)
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
