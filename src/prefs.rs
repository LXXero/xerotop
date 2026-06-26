//! The preferences GUI — a normal toplevel window (not layer-shell) that edits
//! the running bar's config and theme *live*: every change mutates the shared
//! state on the `BarHandle` and calls `apply()`, so the bar re-renders as you
//! drag a slider or pick a color. "Save" persists config.toml / the theme file.

use crate::bar::BarHandle;
use crate::config::{
    Align, BarLength, CornerMode, Edge, HeaderButton, HeaderSlot, Layer, PanelConfig, TempSensor,
};
use crate::theme::{Theme, themes_dir};
use gtk::gdk::RGBA;
use gtk::glib;
use gtk::pango::FontDescription;
use gtk::prelude::*;
use gtk::{
    Box as GtkBox, Button, CheckButton, ColorDialog, ColorDialogButton, DropDown, Entry,
    FontDialog, FontDialogButton, Label, ListBox, ListItem, Notebook, Orientation, Scale,
    SignalListItemFactory, SpinButton, StringList, StringObject, Switch, Window,
};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

thread_local! {
    /// The single live prefs window, if open.
    static WINDOW: RefCell<Option<Window>> = const { RefCell::new(None) };
    /// Callback invoked when the theme is changed externally (e.g. by the desktop
    /// colour-scheme auto-switcher) so the open prefs window can refresh its widgets.
    static THEME_CHANGED_CB: RefCell<Option<Box<dyn Fn()>>> = const { RefCell::new(None) };
}

/// Notify the open prefs window that the theme was changed externally.
pub fn theme_changed() {
    THEME_CHANGED_CB.with(|cb| {
        if let Some(f) = cb.borrow().as_ref() {
            f();
        }
    });
}

const PANEL_TYPES: [&str; 19] = [
    "header",
    "clock",
    "cpu",
    "cores",
    "memory",
    "gpu",
    "disk",
    "net",
    "sensors",
    "weather",
    "mail",
    "uptime",
    "keyboard",
    "battery",
    "volume",
    "brightness",
    "top",
    "tasks",
    "tray",
];

/// Panel types that have a history graph (so the "graph" toggle is meaningful).
const GRAPH_TYPES: [&str; 6] = ["cpu", "memory", "gpu", "disk", "net", "sensors"];

/// Panel types whose label/value header row can be hidden (they build it via
/// the shared `header()` helper, which honors the `show_label` flag).
const LABEL_TYPES: [&str; 6] = ["cpu", "memory", "gpu", "disk", "net", "cores"];

/// Open (or re-focus) the preferences window for the given bar.
pub fn open(handle: &BarHandle) {
    if let Some(w) = WINDOW.with(|w| w.borrow().clone()) {
        w.present();
        return;
    }

    let notebook = Notebook::new();
    notebook.append_page(&general_page(handle), Some(&Label::new(Some("General"))));
    notebook.append_page(&theme_page(handle), Some(&Label::new(Some("Theme"))));
    notebook.append_page(&layout_page(handle), Some(&Label::new(Some("Layout"))));
    notebook.append_page(
        &panels_config_page(handle),
        Some(&Label::new(Some("Panels"))),
    );

    let window = Window::builder()
        .title("xerotop preferences")
        .default_width(600)
        .default_height(620)
        .child(&notebook)
        .build();
    window.connect_close_request(|_| {
        WINDOW.with(|w| *w.borrow_mut() = None);
        THEME_CHANGED_CB.with(|cb| *cb.borrow_mut() = None);
        glib::Propagation::Proceed
    });
    WINDOW.with(|w| *w.borrow_mut() = Some(window.clone()));
    window.present();
}

// ---- layout helpers --------------------------------------------------------

fn page_box() -> GtkBox {
    let b = GtkBox::new(Orientation::Vertical, 6);
    b.set_margin_top(12);
    b.set_margin_bottom(12);
    b.set_margin_start(12);
    b.set_margin_end(12);
    b
}

fn row(text: &str, widget: &impl IsA<gtk::Widget>) -> GtkBox {
    let r = GtkBox::new(Orientation::Horizontal, 10);
    let l = Label::new(Some(text));
    l.set_xalign(0.0);
    l.set_hexpand(true);
    r.append(&l);
    r.append(widget);
    r
}

/// A labeled text entry for a shell command in `actions`. Stores on every edit;
/// applies (rebinds the captured command in the running bar) on Enter.
fn command_row(
    handle: &BarHandle,
    label: &str,
    initial: &str,
    set: fn(&mut crate::config::Actions, String),
) -> GtkBox {
    let entry = Entry::new();
    entry.set_text(initial);
    entry.set_hexpand(true);
    entry.set_tooltip_text(Some("Press Enter to apply to the running bar"));
    let h = handle.clone();
    entry.connect_changed(move |e| set(&mut h.cfg.borrow_mut().actions, e.text().to_string()));
    let h = handle.clone();
    entry.connect_activate(move |_| h.apply());
    // Fixed-width label column so stacked command rows (Lock/Logout/Reboot/
    // Shutdown) line their entries up, instead of each starting after its own
    // label's width.
    let r = GtkBox::new(Orientation::Horizontal, 10);
    let l = Label::new(Some(label));
    l.set_xalign(0.0);
    l.set_width_chars(9);
    r.append(&l);
    r.append(&entry);
    r
}

// ---- color conversion ------------------------------------------------------

fn hex_to_rgba(hex: &str) -> RGBA {
    let h = hex.trim().trim_start_matches('#');
    let p = |i: usize| u8::from_str_radix(h.get(i..i + 2).unwrap_or("80"), 16).unwrap_or(128);
    if h.len() == 6 {
        RGBA::new(
            p(0) as f32 / 255.0,
            p(2) as f32 / 255.0,
            p(4) as f32 / 255.0,
            1.0,
        )
    } else {
        RGBA::new(0.5, 0.5, 0.5, 1.0)
    }
}

fn rgba_to_hex(c: &RGBA) -> String {
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", q(c.red()), q(c.green()), q(c.blue()))
}

// ---- General page ----------------------------------------------------------

fn edge_index(e: Edge) -> u32 {
    match e {
        Edge::Left => 0,
        Edge::Right => 1,
        Edge::Top => 2,
        Edge::Bottom => 3,
    }
}
fn edge_from_index(i: u32) -> Edge {
    match i {
        0 => Edge::Left,
        2 => Edge::Top,
        3 => Edge::Bottom,
        _ => Edge::Right,
    }
}

fn general_page(handle: &BarHandle) -> GtkBox {
    let page = page_box();
    let cfg = handle.cfg.borrow();

    // Edge
    let edge = DropDown::from_strings(&["left", "right", "top", "bottom"]);
    edge.set_selected(edge_index(cfg.bar.edge));
    let h = handle.clone();
    edge.connect_selected_notify(move |d| {
        h.cfg.borrow_mut().bar.edge = edge_from_index(d.selected());
        // Full apply, not just relayout: edge flips the bar's orientation, and
        // tasks/tray bake orientation in at build time (set_horizontal), so the
        // panels must be rebuilt — relayout() alone would leave them stale.
        h.apply();
    });
    page.append(&row("Edge", &edge));

    // Reverse panel order (clock/header at the end — handy on horizontal bars).
    let reverse = Switch::new();
    reverse.set_active(cfg.bar.reverse);
    reverse.set_valign(gtk::Align::Center);
    let h = handle.clone();
    reverse.connect_active_notify(move |s| {
        h.cfg.borrow_mut().bar.reverse = s.is_active();
        h.apply();
    });
    page.append(&row("Reverse panel order", &reverse));

    // Thickness
    let thick = SpinButton::with_range(20.0, 600.0, 2.0);
    thick.set_value(cfg.bar.thickness as f64);
    let h = handle.clone();
    thick.connect_value_changed(move |s| {
        h.cfg.borrow_mut().bar.thickness = s.value() as i32;
        h.relayout();
    });
    page.append(&row("Thickness (px)", &thick));

    // Length: full vs a fixed pixel count, plus alignment when fixed.
    let cur_len = cfg.bar.length;
    let cur_align = cfg.bar.align;
    let length_mode = DropDown::from_strings(&["full", "fixed (px)"]);
    length_mode.set_selected(if matches!(cur_len, BarLength::Full) {
        0
    } else {
        1
    });
    let length_px = SpinButton::with_range(40.0, 8000.0, 10.0);
    length_px.set_value(match cur_len {
        BarLength::Px(n) => n as f64,
        BarLength::Full => 600.0,
    });
    let align = DropDown::from_strings(&["start", "center", "end"]);
    align.set_selected(match cur_align {
        Align::Start => 0,
        Align::Center => 1,
        Align::End => 2,
    });
    let fixed = matches!(cur_len, BarLength::Px(_));
    length_px.set_sensitive(fixed);
    align.set_sensitive(fixed);

    // One closure recomputes length/align from the three widgets and applies.
    let apply_len = {
        let h = handle.clone();
        let mode = length_mode.clone();
        let px = length_px.clone();
        let al = align.clone();
        std::rc::Rc::new(move || {
            let fixed = mode.selected() == 1;
            px.set_sensitive(fixed);
            al.set_sensitive(fixed);
            {
                let mut c = h.cfg.borrow_mut();
                c.bar.length = if fixed {
                    BarLength::Px(px.value() as i32)
                } else {
                    BarLength::Full
                };
                c.bar.align = match al.selected() {
                    0 => Align::Start,
                    2 => Align::End,
                    _ => Align::Center,
                };
            }
            h.relayout();
        })
    };
    {
        let f = apply_len.clone();
        length_mode.connect_selected_notify(move |_| f());
    }
    {
        let f = apply_len.clone();
        length_px.connect_value_changed(move |_| f());
    }
    {
        let f = apply_len.clone();
        align.connect_selected_notify(move |_| f());
    }
    page.append(&row("Length", &length_mode));
    page.append(&row("Length (px, if fixed)", &length_px));
    page.append(&row("Align (if fixed)", &align));

    // Monitor
    let mon_store = StringList::new(&[]);
    mon_store.append("auto");
    if let Some(display) = gtk::gdk::Display::default() {
        let ml = display.monitors();
        for i in 0..ml.n_items() {
            if let Some(m) = ml.item(i).and_downcast::<gtk::gdk::Monitor>() {
                let name = m
                    .connector()
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| format!("Monitor {i}"));
                mon_store.append(&name);
            }
        }
    }
    let expr = gtk::PropertyExpression::new(
        StringObject::static_type(),
        None::<gtk::Expression>,
        "string",
    );
    let mon_dd = DropDown::new(Some(mon_store.clone()), Some(expr));
    let pos = if cfg.bar.monitor == "auto" {
        0
    } else {
        let mut found = 0;
        for i in 1..mon_store.n_items() {
            if mon_store.string(i).as_deref() == Some(&cfg.bar.monitor) {
                found = i;
                break;
            }
        }
        found
    };
    mon_dd.set_selected(pos);
    let h = handle.clone();
    mon_dd.connect_selected_notify(move |dd| {
        let idx = dd.selected();
        let name = if idx == 0 {
            "auto".to_string()
        } else if let Some(s) = mon_store.string(idx) {
            s.to_string()
        } else {
            return;
        };
        h.cfg.borrow_mut().bar.monitor = name;
        h.relayout();
    });
    page.append(&row("Monitor", &mon_dd));

    // Stacking layer
    let layer = DropDown::from_strings(&["background", "bottom", "top", "overlay"]);
    layer.set_selected(match cfg.bar.layer {
        Layer::Background => 0,
        Layer::Bottom => 1,
        Layer::Top => 2,
        Layer::Overlay => 3,
    });
    let h = handle.clone();
    layer.connect_selected_notify(move |d| {
        h.cfg.borrow_mut().bar.layer = match d.selected() {
            0 => Layer::Background,
            1 => Layer::Bottom,
            3 => Layer::Overlay,
            _ => Layer::Top,
        };
        h.relayout();
    });
    page.append(&row("Layer (bottom = windows over bar)", &layer));

    // Opacity
    let opacity = Scale::with_range(Orientation::Horizontal, 0.0, 1.0, 0.01);
    opacity.set_value(cfg.bar.opacity);
    opacity.set_hexpand(true);
    opacity.set_draw_value(true);
    opacity.set_size_request(180, 36);
    let h = handle.clone();
    opacity.connect_value_changed(move |s| {
        h.cfg.borrow_mut().bar.opacity = s.value();
        h.restyle();
    });
    page.append(&row("Opacity", &opacity));

    // Graph gamma
    let gamma = Scale::with_range(Orientation::Horizontal, 0.5, 4.0, 0.1);
    gamma.set_value(cfg.bar.graph_gamma);
    gamma.set_hexpand(true);
    gamma.set_draw_value(true);
    gamma.set_size_request(180, 36);
    let h = handle.clone();
    gamma.connect_value_changed(move |s| {
        h.cfg.borrow_mut().bar.graph_gamma = s.value();
        h.apply();
    });
    page.append(&row("Graph spikiness (gamma)", &gamma));

    // Level-meter bar thickness (bat/vol/bri/disk/sensor fans).
    let meter_h = SpinButton::with_range(2.0, 40.0, 1.0);
    meter_h.set_value(cfg.bar.meter_thickness as f64);
    let h = handle.clone();
    meter_h.connect_value_changed(move |s| {
        h.cfg.borrow_mut().bar.meter_thickness = s.value() as i32;
        h.apply();
    });
    page.append(&row("Meter bar thickness (px)", &meter_h));

    // Corner radius
    let corner_r = SpinButton::with_range(0.0, 30.0, 1.0);
    corner_r.set_value(cfg.bar.corner_radius as f64);
    let h = handle.clone();
    corner_r.connect_value_changed(move |s| {
        h.cfg.borrow_mut().bar.corner_radius = s.value() as i32;
        h.restyle();
    });
    page.append(&row("Corner radius (px)", &corner_r));

    // Corner mode
    let corner_mode = DropDown::from_strings(&["uniform", "edge aware"]);
    corner_mode.set_selected(match cfg.bar.corner_mode {
        CornerMode::Uniform => 0,
        CornerMode::EdgeAware => 1,
    });
    let h = handle.clone();
    corner_mode.connect_selected_notify(move |d| {
        h.cfg.borrow_mut().bar.corner_mode = match d.selected() {
            1 => CornerMode::EdgeAware,
            _ => CornerMode::Uniform,
        };
        h.restyle();
    });
    page.append(&row("Corner mode", &corner_mode));

    // Smooth scroll — separate switches for AC vs battery (battery off = no
    // per-frame wakeups). The bar rebuilds on AC<->battery to apply the change.
    // Toggle smooth in place (no rebuild, so graph history survives); only the
    // switch matching the current power state changes what's on screen now.
    let smooth_ac = Switch::new();
    smooth_ac.set_active(cfg.bar.smooth);
    smooth_ac.set_valign(gtk::Align::Center);
    let h = handle.clone();
    smooth_ac.connect_active_notify(move |s| {
        let on = s.is_active();
        h.cfg.borrow_mut().bar.smooth = on;
        if !crate::power::on_battery() {
            crate::panels::set_all_smooth(on);
        }
    });
    let smooth_bat = Switch::new();
    smooth_bat.set_active(cfg.bar.smooth_battery);
    smooth_bat.set_valign(gtk::Align::Center);
    let h = handle.clone();
    smooth_bat.connect_active_notify(move |s| {
        let on = s.is_active();
        h.cfg.borrow_mut().bar.smooth_battery = on;
        if crate::power::on_battery() {
            crate::panels::set_all_smooth(on);
        }
    });
    let smooth_row = GtkBox::new(Orientation::Horizontal, 8);
    let lbl = Label::new(Some("Smooth graph scroll"));
    lbl.set_xalign(0.0);
    lbl.set_hexpand(true);
    smooth_row.append(&lbl);
    let on_ac = Label::new(Some("on AC"));
    on_ac.add_css_class("label");
    smooth_row.append(&on_ac);
    smooth_row.append(&smooth_ac);
    let on_bat = Label::new(Some("on battery"));
    on_bat.add_css_class("label");
    smooth_row.append(&on_bat);
    smooth_row.append(&smooth_bat);
    page.append(&smooth_row);

    // Battery interval multiplier
    let mult = SpinButton::with_range(1.0, 10.0, 0.5);
    mult.set_digits(1);
    mult.set_value(cfg.power.battery_interval_multiplier);
    let h = handle.clone();
    mult.connect_value_changed(move |s| {
        h.cfg.borrow_mut().power.battery_interval_multiplier = s.value();
        h.apply();
    });
    page.append(&row("On-battery interval ×", &mult));

    // --- Desktop colour-scheme auto-switch ---
    let names = theme_names();
    let auto = CheckButton::with_label("Follow desktop colour scheme");
    auto.set_active(cfg.theme_switch.auto);
    page.append(&auto);

    // Light-theme dropdown.
    let light = DropDown::from_strings(&names.iter().map(|s| s.as_str()).collect::<Vec<_>>());
    let cur_light = names
        .iter()
        .position(|n| *n == cfg.theme_switch.light)
        .unwrap_or(0);
    light.set_selected(cur_light as u32);
    light.set_sensitive(cfg.theme_switch.auto);
    let h = handle.clone();
    light.connect_selected_notify(move |d| {
        let Some(name) = d
            .selected_item()
            .and_downcast::<gtk::StringObject>()
            .map(|o| o.string().to_string())
        else {
            return;
        };
        h.cfg.borrow_mut().theme_switch.light = name;
    });
    page.append(&row("Light theme", &light));

    // Dark-theme dropdown.
    let dark = DropDown::from_strings(&names.iter().map(|s| s.as_str()).collect::<Vec<_>>());
    let cur_dark = names
        .iter()
        .position(|n| *n == cfg.theme_switch.dark)
        .unwrap_or(0);
    dark.set_selected(cur_dark as u32);
    dark.set_sensitive(cfg.theme_switch.auto);
    let h = handle.clone();
    dark.connect_selected_notify(move |d| {
        let Some(name) = d
            .selected_item()
            .and_downcast::<gtk::StringObject>()
            .map(|o| o.string().to_string())
        else {
            return;
        };
        h.cfg.borrow_mut().theme_switch.dark = name;
    });
    page.append(&row("Dark theme", &dark));

    // Wire the checkbox to toggle dropdown sensitivity.
    auto.connect_toggled({
        let h = handle.clone();
        let light = light.clone();
        let dark = dark.clone();
        move |c| {
            h.cfg.borrow_mut().theme_switch.auto = c.is_active();
            light.set_sensitive(c.is_active());
            dark.set_sensitive(c.is_active());
        }
    });

    drop(cfg);
    page.append(&save_bar(handle));
    page
}

// ---- per-panel detail fragments (used by the Panels config tab) -------------
// Tray and weather config used to live on the General page; they're now part of
// the relevant panel's detail pane, alongside that panel's interval/graph.

/// Tray panel options: icons-per-row + icon size.
fn tray_detail(handle: &BarHandle) -> GtkBox {
    let page = page_box();
    let cfg = handle.cfg.borrow();

    let tray_cols = SpinButton::with_range(1.0, 32.0, 1.0);
    tray_cols.set_width_chars(5);
    tray_cols.set_value(cfg.tray.columns as f64);
    let h = handle.clone();
    tray_cols.connect_value_changed(move |s| {
        h.cfg.borrow_mut().tray.columns = s.value() as i32;
        h.apply();
    });
    page.append(&row("Icons per row", &tray_cols));

    let tray_size = SpinButton::with_range(8.0, 64.0, 1.0);
    tray_size.set_width_chars(5);
    tray_size.set_value(cfg.tray.icon_size as f64);
    let h = handle.clone();
    tray_size.connect_value_changed(move |s| {
        h.cfg.borrow_mut().tray.icon_size = s.value() as i32;
        h.apply();
    });
    page.append(&row("Icon size (px)", &tray_size));
    page
}

/// Tasks panel options: header toggle + icon+title rows vs an icons-only grid.
fn tasks_detail(handle: &BarHandle, i: usize) -> GtkBox {
    let page = page_box();

    // The "TASKS" header row (reuses the shared show_label knob).
    page.append(&show_label_check(handle, i));

    let cfg = handle.cfg.borrow();

    // Layout: icon+title rows (off) vs an icons-only grid (on).
    let icons_only = CheckButton::with_label("Icons only (dock-style grid, titles on hover)");
    icons_only.set_active(cfg.panel.get(i).map(|p| p.icons_only).unwrap_or(false));

    let cols = SpinButton::with_range(1.0, 32.0, 1.0);
    cols.set_width_chars(5);
    cols.set_value(cfg.panel.get(i).and_then(|p| p.columns).unwrap_or(1) as f64);
    let cols_row = row("Icons per row", &cols);

    let size = SpinButton::with_range(8.0, 64.0, 1.0);
    size.set_width_chars(5);
    size.set_value(cfg.panel.get(i).and_then(|p| p.icon_size).unwrap_or(16) as f64);
    drop(cfg);

    // "Icons per row" only applies to the grid layout — grey it out otherwise.
    cols_row.set_sensitive(icons_only.is_active());
    {
        let cols_row = cols_row.clone();
        let h = handle.clone();
        icons_only.connect_toggled(move |c| {
            cols_row.set_sensitive(c.is_active());
            if let Some(p) = h.cfg.borrow_mut().panel.get_mut(i) {
                p.icons_only = c.is_active();
            }
            h.apply();
        });
    }
    {
        let h = handle.clone();
        cols.connect_value_changed(move |s| {
            if let Some(p) = h.cfg.borrow_mut().panel.get_mut(i) {
                p.columns = Some(s.value() as i32);
            }
            h.apply();
        });
    }
    {
        let h = handle.clone();
        size.connect_value_changed(move |s| {
            if let Some(p) = h.cfg.borrow_mut().panel.get_mut(i) {
                p.icon_size = Some(s.value() as i32);
            }
            h.apply();
        });
    }

    page.append(&icons_only);
    page.append(&cols_row);
    page.append(&row("Icon size (px)", &size));
    page
}

/// Weather panel options: location / units / refresh / condition text.
fn weather_detail(handle: &BarHandle) -> GtkBox {
    let page = page_box();
    let cfg = handle.cfg.borrow();

    let wx_loc = Entry::new();
    wx_loc.set_text(&cfg.weather.location);
    wx_loc.set_hexpand(true);
    wx_loc.set_placeholder_text(Some("city or lat,lon — blank = auto by IP"));
    wx_loc.set_tooltip_text(Some("Press Enter to apply"));
    let h = handle.clone();
    wx_loc.connect_changed(move |e| h.cfg.borrow_mut().weather.location = e.text().to_string());
    let h = handle.clone();
    wx_loc.connect_activate(move |_| h.apply());
    page.append(&row("Location", &wx_loc));

    let wx_units = DropDown::from_strings(&["auto", "°C", "°F"]);
    wx_units.set_selected(match cfg.weather.units.as_str() {
        "c" => 1,
        "f" => 2,
        _ => 0,
    });
    let h = handle.clone();
    wx_units.connect_selected_notify(move |d| {
        h.cfg.borrow_mut().weather.units = match d.selected() {
            1 => "c",
            2 => "f",
            _ => "auto",
        }
        .into();
        h.apply();
    });
    page.append(&row("Units", &wx_units));

    let wx_iv = SpinButton::with_range(5.0, 240.0, 5.0);
    wx_iv.set_value(cfg.weather.interval_min);
    let h = handle.clone();
    wx_iv.connect_value_changed(move |s| {
        h.cfg.borrow_mut().weather.interval_min = s.value();
        h.apply();
    });
    page.append(&row("Refresh (min)", &wx_iv));

    let wx_cond =
        CheckButton::with_label("Show condition text (else icon + temp only; report on hover)");
    wx_cond.set_active(cfg.weather.show_condition);
    let h = handle.clone();
    wx_cond.connect_toggled(move |c| {
        h.cfg.borrow_mut().weather.show_condition = c.is_active();
        h.apply();
    });
    page.append(&wx_cond);
    page
}

/// Mail panel options: maildir path / click command / recount interval.
fn mail_detail(handle: &BarHandle) -> GtkBox {
    let page = page_box();
    let cfg = handle.cfg.borrow();

    let dir = Entry::new();
    dir.set_text(&cfg.mail.dir);
    dir.set_hexpand(true);
    dir.set_placeholder_text(Some("maildir root — blank = ~/.maildir"));
    dir.set_tooltip_text(Some("Press Enter to apply"));
    let h = handle.clone();
    dir.connect_changed(move |e| h.cfg.borrow_mut().mail.dir = e.text().to_string());
    let h = handle.clone();
    dir.connect_activate(move |_| h.apply());
    page.append(&row("Maildir", &dir));

    let cmd = Entry::new();
    cmd.set_text(&cfg.mail.command);
    cmd.set_hexpand(true);
    cmd.set_placeholder_text(Some("command run on click"));
    cmd.set_tooltip_text(Some("Press Enter to apply"));
    let h = handle.clone();
    cmd.connect_changed(move |e| h.cfg.borrow_mut().mail.command = e.text().to_string());
    let h = handle.clone();
    cmd.connect_activate(move |_| h.apply());
    page.append(&row("Click command", &cmd));

    let iv = SpinButton::with_range(1.0, 600.0, 1.0);
    iv.set_value(cfg.mail.interval_s);
    let h = handle.clone();
    iv.connect_value_changed(move |s| {
        h.cfg.borrow_mut().mail.interval_s = s.value();
        h.apply();
    });
    page.append(&row("Recount every (s)", &iv));
    page
}

// ---- Theme page ------------------------------------------------------------

type Getter = fn(&Theme) -> String;
type Setter = fn(&mut Theme, String);

// Each color: name, getter, setter, and whether it's a graph-palette color.
// Graph colors are baked into the canvas-drawn graphs at build time, so editing
// one needs a full rebuild; the rest are pure CSS and only need a restyle (no
// rebuild → graph history survives).
#[allow(clippy::type_complexity)]
const COLOR_FIELDS: [(&str, Getter, Setter, bool); 11] = [
    (
        "Background",
        |t| t.background.clone(),
        |t, v| t.background = v,
        false,
    ),
    (
        "Foreground",
        |t| t.foreground.clone(),
        |t, v| t.foreground = v,
        false,
    ),
    (
        "Header / accent",
        |t| t.label.clone(),
        |t, v| t.label = v,
        false,
    ),
    ("Muted", |t| t.muted.clone(), |t, v| t.muted = v, false),
    ("Graph green", |t| t.green.clone(), |t, v| t.green = v, true),
    ("Graph cyan", |t| t.cyan.clone(), |t, v| t.cyan = v, true),
    ("Graph amber", |t| t.amber.clone(), |t, v| t.amber = v, true),
    ("Graph red", |t| t.red.clone(), |t, v| t.red = v, true),
    (
        "Graph violet",
        |t| t.violet.clone(),
        |t, v| t.violet = v,
        true,
    ),
    (
        "Keyboard LED",
        |t| t.led_on.clone(),
        |t, v| t.led_on = v,
        false,
    ),
    (
        "Graph background",
        |t| t.graph_background.clone(),
        |t, v| t.graph_background = v,
        false,
    ),
];

fn theme_page(handle: &BarHandle) -> GtkBox {
    let page = page_box();
    // Guard so loading a theme into the buttons doesn't feed back as 12 edits.
    let loading = Rc::new(Cell::new(false));

    // Theme selector (built-in default + every themes/<name>.toml).
    let names = theme_names();
    let cur = handle.cfg.borrow().theme.clone();
    let selector = DropDown::from_strings(&names.iter().map(|s| s.as_str()).collect::<Vec<_>>());
    selector.set_selected(names.iter().position(|n| *n == cur).unwrap_or(0) as u32);

    // Font picker.
    let font_btn = FontDialogButton::new(Some(FontDialog::new()));
    font_btn.set_font_desc(&FontDescription::from_string(
        &handle.theme.borrow().font_family,
    ));
    let h = handle.clone();
    let ld = loading.clone();
    font_btn.connect_font_desc_notify(move |b| {
        if ld.get() {
            return;
        }
        if let Some(desc) = b.font_desc()
            && let Some(fam) = desc.family()
        {
            h.theme.borrow_mut().font_family = fam.to_string();
            h.restyle();
        }
    });
    page.append(&row("Font", &font_btn));

    // Font-size tiers (gkrellm-style small/normal/large). These are config-level
    // overrides — they win over the theme's defaults and persist independently,
    // so switching themes doesn't wipe your sizing. Initial value = override if
    // set, else the active theme's size.
    let font_size_spin =
        |theme_get: fn(&Theme) -> i32,
         cfg_get: fn(&crate::config::FontConfig) -> Option<i32>,
         cfg_set: fn(&mut crate::config::FontConfig, Option<i32>)| {
            let sp = SpinButton::with_range(4.0, 96.0, 1.0);
            let initial = cfg_get(&handle.cfg.borrow().font)
                .unwrap_or_else(|| theme_get(&handle.theme.borrow()));
            sp.set_value(initial as f64);
            let h = handle.clone();
            let ld = loading.clone();
            sp.connect_value_changed(move |s| {
                if ld.get() {
                    return;
                }
                cfg_set(&mut h.cfg.borrow_mut().font, Some(s.value() as i32));
                h.restyle();
            });
            sp
        };
    let f_small = font_size_spin(|t| t.font_small, |f| f.small, |f, v| f.small = v);
    let f_normal = font_size_spin(|t| t.font_normal, |f| f.normal, |f, v| f.normal = v);
    let f_large = font_size_spin(|t| t.font_large, |f| f.large, |f, v| f.large = v);
    page.append(&row("Font small (sub, date)", &f_small));
    page.append(&row("Font normal (labels, values)", &f_normal));
    page.append(&row("Font large (clock)", &f_large));

    // One color button per theme color, laid out in two columns so the list
    // doesn't run off the bottom of the tab. Labels are left-aligned (the label
    // columns expand) so the swatches line up at the right of each column.
    // Remembered so loading a theme refreshes every button.
    let buttons: Rc<RefCell<Vec<(Getter, ColorDialogButton)>>> = Rc::new(RefCell::new(Vec::new()));
    let grid = gtk::Grid::new();
    grid.set_row_spacing(6);
    grid.set_column_spacing(16);
    grid.set_margin_top(4);
    for (i, (name, get, set, graph)) in COLOR_FIELDS.into_iter().enumerate() {
        let btn = ColorDialogButton::new(Some(ColorDialog::new()));
        btn.set_rgba(&hex_to_rgba(&get(&handle.theme.borrow())));
        let h = handle.clone();
        let ld = loading.clone();
        btn.connect_rgba_notify(move |b| {
            if ld.get() {
                return;
            }
            let hex = rgba_to_hex(&b.rgba());
            {
                let mut t = h.theme.borrow_mut();
                set(&mut t, hex);
            }
            // Graph colors are baked into the graphs → rebuild; the rest are CSS.
            if graph {
                h.apply();
            } else {
                h.restyle();
            }
        });
        let lbl = Label::new(Some(name));
        lbl.set_xalign(0.0);
        lbl.set_hexpand(true);
        let r = (i / 2) as i32;
        let c = (i % 2) as i32 * 2; // left pair = cols 0/1, right pair = cols 2/3
        grid.attach(&lbl, c, r, 1, 1);
        grid.attach(&btn, c + 1, r, 1, 1);
        buttons.borrow_mut().push((get, btn));
    }
    page.append(&grid);

    // Graph background opacity
    let gb_opacity = Scale::with_range(Orientation::Horizontal, 0.0, 1.0, 0.01);
    gb_opacity.set_value(handle.theme.borrow().graph_background_opacity);
    gb_opacity.set_hexpand(true);
    gb_opacity.set_draw_value(true);
    gb_opacity.set_size_request(180, 36);
    let h = handle.clone();
    let ld = loading.clone();
    gb_opacity.connect_value_changed(move |s| {
        if ld.get() {
            return;
        }
        h.theme.borrow_mut().graph_background_opacity = s.value();
        h.restyle();
    });
    let gb_opacity_c = gb_opacity.clone();
    page.append(&row("Graph bg opacity", &gb_opacity));

    // Loading a theme: resolve the file, swap it in, refresh every widget.
    let h = handle.clone();
    let buttons_c = buttons.clone();
    let font_c = font_btn.clone();
    let (fs_c, fn_c, fl_c) = (f_small.clone(), f_normal.clone(), f_large.clone());
    let ld = loading.clone();
    selector.connect_selected_notify(move |d| {
        if ld.get() {
            return;
        }
        // Read the name from the model object, not a captured Vec — the model is
        // rebuilt on "Save theme as…", so a stale index could panic.
        let Some(name) = d
            .selected_item()
            .and_downcast::<gtk::StringObject>()
            .map(|o| o.string().to_string())
        else {
            return;
        };
        let t = crate::theme::resolve(&name);
        h.cfg.borrow_mut().theme = name.clone();
        // A theme that bundles panel colors applies them to the live config
        // (sensor colors / header buttons), so switching themes can carry a full
        // look. Themes without a bundle leave your panels untouched.
        if let Some(s) = &t.sensors {
            h.cfg.borrow_mut().temp.sensors = s.clone();
        }
        if let Some(hdr) = &t.header {
            h.cfg.borrow_mut().header = hdr.clone();
        }
        ld.set(true);
        font_c.set_font_desc(&FontDescription::from_string(&t.font_family));
        fs_c.set_value(t.font_small as f64);
        fn_c.set_value(t.font_normal as f64);
        fl_c.set_value(t.font_large as f64);
        gb_opacity_c.set_value(t.graph_background_opacity);
        for (get, btn) in buttons_c.borrow().iter() {
            btn.set_rgba(&hex_to_rgba(&get(&t)));
        }
        *h.theme.borrow_mut() = t;
        ld.set(false);
        h.apply();
    });
    // Prepend the selector row at the top of the page.
    page.prepend(&row("Theme", &selector));

    // Save-theme-as bar — two buttons: palette+fonts only, or also bundling the
    // current panel colors (sensor colors + header buttons) into the theme file.
    let save_row = GtkBox::new(Orientation::Horizontal, 8);
    save_row.set_margin_top(8);
    let name_entry = Entry::new();
    name_entry.set_placeholder_text(Some("theme name"));
    name_entry.set_hexpand(true);

    // Shared save routine; `bundle_panels` decides whether the theme carries the
    // current sensor/header colors. Stamping/clearing the bundle on the live
    // theme keeps later "Save to config.toml" re-saves consistent.
    let save_with: Rc<dyn Fn(bool)> = {
        let h = handle.clone();
        let entry_c = name_entry.clone();
        let selector_c = selector.clone();
        Rc::new(move |bundle_panels: bool| {
            let name = entry_c.text().trim().to_string();
            // Reject "default" and anything that isn't a strict slug (path-safe).
            if name == "default" || !crate::theme::is_valid_name(&name) {
                entry_c.set_text("");
                entry_c.set_placeholder_text(Some("use letters, digits, - or _ (not 'default')"));
                return;
            }
            {
                let sensors = h.cfg.borrow().temp.sensors.clone();
                let header = h.cfg.borrow().header.clone();
                let mut t = h.theme.borrow_mut();
                if bundle_panels {
                    t.sensors = Some(sensors);
                    t.header = Some(header);
                } else {
                    t.sensors = None;
                    t.header = None;
                }
            }
            if let Err(e) = save_theme_file(&name, &h.theme.borrow()) {
                eprintln!("xerotop: save theme failed: {e}");
                return;
            }
            h.cfg.borrow_mut().theme = name.clone();
            // Rebuild the selector to include the new name and select it.
            let names = theme_names();
            selector_c.set_model(Some(&gtk::StringList::new(
                &names.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
            )));
            selector_c.set_selected(names.iter().position(|n| *n == name).unwrap_or(0) as u32);
        })
    };

    let save_theme = Button::with_label("Save theme");
    save_theme.set_tooltip_text(Some("Write palette + fonts to themes/<name>.toml"));
    let sw = save_with.clone();
    save_theme.connect_clicked(move |_| sw(false));

    let save_panels = Button::with_label("Save + panel colors");
    save_panels.set_tooltip_text(Some(
        "Also bundle the current sensor colors + header buttons, so loading this \
         theme restores them",
    ));
    let sw = save_with.clone();
    save_panels.connect_clicked(move |_| sw(true));

    save_row.append(&name_entry);
    save_row.append(&save_theme);
    save_row.append(&save_panels);
    page.append(&save_row);

    // Register a callback so the prefs widgets refresh when the theme changes
    // externally (desktop colour-scheme auto-switcher).
    {
        let names = theme_names();
        let selector = selector.clone();
        let ld = loading.clone();
        let font_c = font_btn.clone();
        let fs_c = f_small.clone();
        let fn_c = f_normal.clone();
        let fl_c = f_large.clone();
        let gb_opacity_c = gb_opacity.clone();
        let buttons_c = buttons.clone();
        let h = handle.clone();
        THEME_CHANGED_CB.with(|cb| {
            *cb.borrow_mut() = Some(Box::new(move || {
                let cur = h.cfg.borrow().theme.clone();
                let Some(pos) = names.iter().position(|n| *n == cur) else {
                    return;
                };
                ld.set(true);
                selector.set_selected(pos as u32);
                let t = h.theme.borrow();
                font_c.set_font_desc(&FontDescription::from_string(&t.font_family));
                fs_c.set_value(t.font_small as f64);
                fn_c.set_value(t.font_normal as f64);
                fl_c.set_value(t.font_large as f64);
                gb_opacity_c.set_value(t.graph_background_opacity);
                for (get, btn) in buttons_c.borrow().iter() {
                    btn.set_rgba(&hex_to_rgba(&get(&t)));
                }
                drop(t);
                ld.set(false);
            }));
        });
    }

    page.append(&save_bar(handle));
    page
}

fn theme_names() -> Vec<String> {
    let mut v = vec!["default".to_string()];
    if let Ok(entries) = std::fs::read_dir(themes_dir()) {
        for e in entries.flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "toml")
                && let Some(stem) = p.file_stem().and_then(|s| s.to_str())
                && stem != "default"
            {
                v.push(stem.to_string());
            }
        }
    }
    v
}

fn save_theme_file(name: &str, theme: &Theme) -> std::io::Result<()> {
    let dir = themes_dir();
    std::fs::create_dir_all(&dir)?;
    let body = toml::to_string_pretty(theme).map_err(std::io::Error::other)?;
    std::fs::write(dir.join(format!("{name}.toml")), body)
}

// ---- Panels page -----------------------------------------------------------

fn layout_page(handle: &BarHandle) -> GtkBox {
    ensure_prefs_css();
    let page = page_box();
    page.append(&Label::new(Some(
        "Which panels are on the bar, and their order (top-to-bottom). \
         Drag the handle to reorder. Per-panel options live in the Panels tab.",
    )));

    // Column header.
    let head = GtkBox::new(Orientation::Horizontal, 6);
    let mk = |text: &str, chars: i32, expand: bool| {
        let l = Label::new(Some(text));
        l.add_css_class("label");
        l.set_xalign(0.0);
        if chars > 0 {
            l.set_width_chars(chars);
        }
        l.set_hexpand(expand);
        l
    };
    head.append(&mk("#  ⠿ panel", 12, false));
    head.append(&mk("", 0, true)); // spacer
    head.append(&mk("move · delete", 0, false));
    page.append(&head);

    let list = ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::None);
    populate_panels(handle, &list);
    page.append(&list);

    // Add a panel.
    let add_row = GtkBox::new(Orientation::Horizontal, 8);
    add_row.set_margin_top(8);
    let kinds = DropDown::from_strings(&PANEL_TYPES);
    add_row.append(&kinds); // show the type chooser (was missing → always added "header")
    let add = Button::with_label("Add panel");
    let h = handle.clone();
    let list_c = list.clone();
    add.connect_clicked(move |_| {
        let kind = PANEL_TYPES[kinds.selected() as usize].to_string();
        h.cfg.borrow_mut().panel.push(PanelConfig {
            kind,
            interval: 1.0,
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
            show_capacity: true,
            show_disk_total: false,
            core: -1,
            show_hostname: false,
            short_hostname: false,
            show_kernel: false,
        });
        h.apply();
        populate_panels(&h, &list_c);
    });
    add_row.append(&add);
    page.append(&add_row);

    page.append(&save_bar(handle));
    page
}

/// Rebuild the panel-list rows from the current config.
fn populate_panels(handle: &BarHandle, list: &ListBox) {
    while let Some(c) = list.first_child() {
        list.remove(&c);
    }
    let n = handle.cfg.borrow().panel.len();
    for i in 0..n {
        list.append(&panel_row(handle, list, i, n));
    }
}

fn panel_row(handle: &BarHandle, list: &ListBox, i: usize, n: usize) -> GtkBox {
    let r = GtkBox::new(Orientation::Horizontal, 6);
    r.set_margin_top(2);
    r.set_margin_bottom(2);
    let kind = handle.cfg.borrow().panel[i].kind.clone();

    // Index (monospace, right-aligned) so the rows line up like the Panels tab.
    let num = Label::new(Some(&i.to_string()));
    num.add_css_class("panel-idx");
    num.set_width_chars(2);
    num.set_xalign(1.0);
    r.append(&num);

    // Grab handle — the drag affordance.
    let grip = Label::new(Some("\u{2807}")); // ⠇-ish dotted grip
    grip.add_css_class("drag-handle");
    r.append(&grip);

    let name = Label::new(Some(&kind));
    name.set_xalign(0.0);
    name.set_width_chars(8);
    r.append(&name);

    let spacer = GtkBox::new(Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    r.append(&spacer);

    // --- drag to reorder -----------------------------------------------------
    // The row is a drag source carrying its own index; every row is also a drop
    // target that moves the dragged panel to this position, then rebuilds.
    let drag = gtk::DragSource::new();
    drag.set_actions(gtk::gdk::DragAction::MOVE);
    drag.connect_prepare(move |_, _, _| {
        Some(gtk::gdk::ContentProvider::for_value(&(i as i32).to_value()))
    });
    r.add_controller(drag);

    let drop_target = gtk::DropTarget::new(i32::static_type(), gtk::gdk::DragAction::MOVE);
    let h = handle.clone();
    let list_c = list.clone();
    drop_target.connect_drop(move |_, value, _, _| {
        let Ok(from) = value.get::<i32>() else {
            return false;
        };
        let from = from as usize;
        if from != i {
            {
                let mut cfg = h.cfg.borrow_mut();
                if from < cfg.panel.len() {
                    let item = cfg.panel.remove(from);
                    let to = i.min(cfg.panel.len());
                    cfg.panel.insert(to, item);
                }
            }
            h.apply();
            populate_panels(&h, &list_c);
        }
        true
    });
    r.add_controller(drop_target);

    let up = Button::from_icon_name("go-up-symbolic");
    up.set_sensitive(i > 0);
    let h = handle.clone();
    let list_c = list.clone();
    up.connect_clicked(move |_| {
        h.cfg.borrow_mut().panel.swap(i, i - 1);
        h.apply();
        populate_panels(&h, &list_c);
    });
    r.append(&up);

    let down = Button::from_icon_name("go-down-symbolic");
    down.set_sensitive(i + 1 < n);
    let h = handle.clone();
    let list_c = list.clone();
    down.connect_clicked(move |_| {
        h.cfg.borrow_mut().panel.swap(i, i + 1);
        h.apply();
        populate_panels(&h, &list_c);
    });
    r.append(&down);

    let del = Button::from_icon_name("edit-delete-symbolic");
    let h = handle.clone();
    let list_c = list.clone();
    del.connect_clicked(move |_| {
        h.cfg.borrow_mut().panel.remove(i);
        h.apply();
        populate_panels(&h, &list_c);
    });
    r.append(&del);

    r
}

// ---- Temp page -------------------------------------------------------------

/// Default fill colors for newly-added sensors (cycled), matching the palette.
const SENSOR_COLORS: [&str; 5] = ["#ff7366", "#c78cff", "#66ccff", "#66ff66", "#ffbf4d"];

/// The auto-detected default sensor list (cpu/gpu/ssd + fan) as explicit entries.
fn default_temp_sensors() -> Vec<TempSensor> {
    crate::metrics::default_sensors()
        .into_iter()
        .map(|(chip, input, label, color)| TempSensor {
            chip,
            input,
            label: label.to_string(),
            color: color.to_string(),
            fan_max: 0.0,
        })
        .collect()
}

/// Rows to display in the Sensors editor: the explicit config list, or the
/// auto-detected defaults as a *view model* when nothing's configured. The
/// defaults are NOT written to config here — so merely opening the tab (and
/// later saving for some unrelated reason) won't pin this machine's hwmon names.
fn effective_sensors(handle: &BarHandle) -> Vec<TempSensor> {
    let cfg = handle.cfg.borrow();
    if cfg.temp.sensors.is_empty() {
        default_temp_sensors()
    } else {
        cfg.temp.sensors.clone()
    }
}

/// Commit the displayed defaults into config the first time the user actually
/// edits a sensor, so the rows become real, indexable config entries (and the
/// indices line up with what's on screen). No-op once a list exists.
fn materialize_sensors(handle: &BarHandle) {
    if handle.cfg.borrow().temp.sensors.is_empty() {
        handle.cfg.borrow_mut().temp.sensors = default_temp_sensors();
    }
}

fn temp_detail(handle: &BarHandle, i: usize) -> GtkBox {
    // When nothing's configured we show the auto-detected defaults as a view
    // model (see effective_sensors) without writing them to config; they only
    // get committed once the user actually edits a row (materialize_sensors).
    let page = page_box();
    page.append(&interval_row(handle, i));
    page.append(&show_label_check(handle, i));
    let graph = CheckButton::with_label("Show history graph");
    graph.set_active(handle.cfg.borrow().panel[i].graph);
    let h = handle.clone();
    graph.connect_toggled(move |c| {
        if let Some(p) = h.cfg.borrow_mut().panel.get_mut(i) {
            p.graph = c.is_active();
        }
        h.apply();
    });
    page.append(&graph);
    page.append(&graph_height_row(handle, i, 14)); // MINI_H default
    page.append(&graph_window_row(handle, i));
    if handle.cfg.borrow().bar.edge.is_horizontal() {
        page.append(&graph_width_row(handle, i));
    }
    page.append(&Label::new(Some(
        "Add sensors (or an 'average' row) from the dropdown; reorder/label/color each. Empty = auto-detect.",
    )));

    let list = ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::None);
    populate_temp_list(handle, &list);
    page.append(&list);

    // Add row: an "average" pseudo-sensor plus every discovered sensor.
    let add_row = GtkBox::new(Orientation::Horizontal, 8);
    add_row.set_margin_top(8);
    let discovered = crate::metrics::list_sensors();
    // ids[0] is the special average row; the rest mirror `discovered`.
    let mut ids: Vec<(String, String, String)> = vec![(
        crate::panels::AVG_CHIP.to_string(),
        String::new(),
        "avg".to_string(),
    )];
    let mut labels: Vec<String> = vec!["average (of temps)".to_string()];
    for s in &discovered {
        ids.push((
            s.chip.clone(),
            s.input.clone(),
            s.label.clone().unwrap_or_default(),
        ));
        let v = match s.kind {
            crate::metrics::SensorKind::Temp => format!("{:.0}°", s.value),
            crate::metrics::SensorKind::Fan => format!("{:.0}rpm", s.value),
            crate::metrics::SensorKind::Voltage => format!("{:.2}V", s.value),
        };
        labels.push(match &s.label {
            Some(l) => format!("{}/{} {l} ({v})", s.chip, s.input),
            None => format!("{}/{} ({v})", s.chip, s.input),
        });
    }
    let kinds = DropDown::from_strings(&labels.iter().map(|s| s.as_str()).collect::<Vec<_>>());
    add_row.append(&kinds);
    let add = Button::with_label("Add");
    let h = handle.clone();
    let list_c = list.clone();
    add.connect_clicked(move |_| {
        let Some((chip, input, label)) = ids.get(kinds.selected() as usize) else {
            return;
        };
        materialize_sensors(&h); // append to the auto defaults, don't replace them
        let n = h.cfg.borrow().temp.sensors.len();
        h.cfg.borrow_mut().temp.sensors.push(TempSensor {
            chip: chip.clone(),
            input: input.clone(),
            label: label.clone(),
            color: SENSOR_COLORS[n % SENSOR_COLORS.len()].to_string(),
            fan_max: 0.0,
        });
        h.apply();
        populate_temp_list(&h, &list_c);
    });
    add_row.append(&add);

    // Reset to the auto-detected defaults.
    let reset = Button::with_label("Reset to defaults");
    let h = handle.clone();
    let list_c = list.clone();
    reset.connect_clicked(move |_| {
        h.cfg.borrow_mut().temp.sensors = default_temp_sensors();
        h.apply();
        populate_temp_list(&h, &list_c);
    });
    add_row.append(&reset);
    page.append(&add_row);
    page
}

fn populate_temp_list(handle: &BarHandle, list: &ListBox) {
    while let Some(c) = list.first_child() {
        list.remove(&c);
    }
    let n = effective_sensors(handle).len();
    for i in 0..n {
        list.append(&temp_sensor_row(handle, list, i, n));
    }
}

fn temp_sensor_row(handle: &BarHandle, list: &ListBox, i: usize, n: usize) -> GtkBox {
    let (chip, input, label, color, fan_max) = {
        let sensors = effective_sensors(handle);
        let s = &sensors[i];
        (
            s.chip.clone(),
            s.input.clone(),
            s.label.clone(),
            s.color.clone(),
            s.fan_max,
        )
    };
    let r = GtkBox::new(Orientation::Horizontal, 6);
    r.set_margin_top(2);
    r.set_margin_bottom(2);

    let id_text = if chip == crate::panels::AVG_CHIP {
        "average".to_string()
    } else {
        format!("{chip}/{input}")
    };
    let id = Label::new(Some(&id_text));
    id.set_xalign(0.0);
    id.set_width_chars(15);
    id.set_ellipsize(gtk::pango::EllipsizeMode::End);
    r.append(&id);

    let label_entry = Entry::new();
    label_entry.set_text(&label);
    label_entry.set_placeholder_text(Some("label"));
    label_entry.set_width_chars(6);
    label_entry.set_tooltip_text(Some("Press Enter to apply"));
    let h = handle.clone();
    label_entry.connect_changed(move |e| {
        materialize_sensors(&h);
        h.cfg.borrow_mut().temp.sensors[i].label = e.text().to_string();
    });
    let h = handle.clone();
    label_entry.connect_activate(move |_| h.apply()); // Enter applies
    // Also apply on focus-out, so clicking away (not just Enter) re-renders the
    // bar — matches the instant-apply color picker next to it.
    let h = handle.clone();
    let focus = gtk::EventControllerFocus::new();
    focus.connect_leave(move |_| h.apply());
    label_entry.add_controller(focus);
    r.append(&label_entry);

    let color_btn = ColorDialogButton::new(Some(ColorDialog::new()));
    let init = if color.is_empty() {
        SENSOR_COLORS[i % SENSOR_COLORS.len()]
    } else {
        &color
    };
    color_btn.set_rgba(&hex_to_rgba(init));
    let h = handle.clone();
    color_btn.connect_rgba_notify(move |b| {
        materialize_sensors(&h);
        h.cfg.borrow_mut().temp.sensors[i].color = rgba_to_hex(&b.rgba());
        h.apply();
    });
    r.append(&color_btn);

    // Fan rows only: full-scale RPM for the level bar (next to the color picker).
    if input.starts_with("fan") {
        let max = gtk::SpinButton::with_range(500.0, 8000.0, 100.0);
        max.set_value(if fan_max > 0.0 { fan_max } else { 3000.0 });
        max.set_tooltip_text(Some("Fan RPM mapped to a full bar"));
        let h = handle.clone();
        max.connect_value_changed(move |s| {
            materialize_sensors(&h);
            h.cfg.borrow_mut().temp.sensors[i].fan_max = s.value();
            h.apply();
        });
        r.append(&max);
    }

    let spacer = GtkBox::new(Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    r.append(&spacer);

    let up = Button::from_icon_name("go-up-symbolic");
    up.set_sensitive(i > 0);
    let h = handle.clone();
    let list_c = list.clone();
    up.connect_clicked(move |_| {
        materialize_sensors(&h);
        h.cfg.borrow_mut().temp.sensors.swap(i, i - 1);
        h.apply();
        populate_temp_list(&h, &list_c);
    });
    r.append(&up);

    let down = Button::from_icon_name("go-down-symbolic");
    down.set_sensitive(i + 1 < n);
    let h = handle.clone();
    let list_c = list.clone();
    down.connect_clicked(move |_| {
        materialize_sensors(&h);
        h.cfg.borrow_mut().temp.sensors.swap(i, i + 1);
        h.apply();
        populate_temp_list(&h, &list_c);
    });
    r.append(&down);

    let del = Button::from_icon_name("edit-delete-symbolic");
    let h = handle.clone();
    let list_c = list.clone();
    del.connect_clicked(move |_| {
        materialize_sensors(&h);
        h.cfg.borrow_mut().temp.sensors.remove(i);
        h.apply();
        populate_temp_list(&h, &list_c);
    });
    r.append(&del);

    r
}

// ---- Commands page ---------------------------------------------------------

/// Replace/clear the header button for `slot` in the config vec.
fn set_header_slot(
    buttons: &mut Vec<HeaderButton>,
    slot: HeaderSlot,
    icon: String,
    command: String,
    color: String,
) {
    buttons.retain(|b| b.slot != slot);
    if !icon.is_empty() || !command.is_empty() {
        buttons.push(HeaderButton {
            slot,
            icon,
            command,
            color,
        });
    }
}

fn header_slot_row(handle: &BarHandle, slot: HeaderSlot, name: &str) -> GtkBox {
    let (icon0, cmd0, color0) = {
        let cfg = handle.cfg.borrow();
        cfg.header
            .iter()
            .find(|b| b.slot == slot)
            .map(|b| (b.icon.clone(), b.command.clone(), b.color.clone()))
            .unwrap_or_default()
    };
    let r = GtkBox::new(Orientation::Horizontal, 6);
    let lbl = Label::new(Some(name));
    lbl.set_xalign(0.0);
    lbl.set_width_chars(9);
    r.append(&lbl);

    // Effective saved glyph: blank icon falls back to the command's default
    // (power for @menu, lock for a lock command, …; "" for blank/unknown).
    let icon0 = if icon0.is_empty() {
        crate::panels::default_glyph(&cmd0).to_string()
    } else {
        icon0
    };

    // Glyph picker rows: "none" first, the common glyphs, then "custom…" last.
    let glyphs = crate::panels::HEADER_GLYPHS;
    let none_idx: u32 = 0;
    let custom_idx = (glyphs.len() + 1) as u32; // after none + all glyphs
    let mut rows: Vec<String> = vec!["— none —".to_string()];
    rows.extend(glyphs.iter().map(|(n, g)| format!("{g}  {n}")));
    rows.push("custom…".to_string());
    let picker = DropDown::from_strings(&rows.iter().map(|s| s.as_str()).collect::<Vec<_>>());
    picker.add_css_class("glyphpick"); // force the Nerd Font
    picker.set_tooltip_text(Some(
        "Pick a glyph, 'none' to disable, or 'custom…' to type one",
    ));
    // Custom factory: enlarge just the leading glyph (rows are "<glyph>  name"),
    // leaving the name at the normal font size.
    let factory = SignalListItemFactory::new();
    factory.connect_setup(|_, item| {
        let label = Label::new(None);
        label.set_xalign(0.0);
        if let Some(item) = item.downcast_ref::<ListItem>() {
            item.set_child(Some(&label));
        }
    });
    factory.connect_bind(|_, item| {
        let Some(item) = item.downcast_ref::<ListItem>() else {
            return;
        };
        let text = item
            .item()
            .and_then(|o| o.downcast::<StringObject>().ok())
            .map(|s| s.string().to_string())
            .unwrap_or_default();
        let Some(label) = item.child().and_downcast::<Label>() else {
            return;
        };
        // "<glyph>  name" → big glyph + normal name; other rows render plain.
        let mut chars = text.chars();
        let first = chars.next();
        let rest: String = chars.collect();
        if let Some(g) = first.filter(|_| rest.starts_with("  ")) {
            label.set_markup(&format!(
                "<span size='180%'>{}</span>{}",
                glib::markup_escape_text(&g.to_string()),
                glib::markup_escape_text(&rest),
            ));
        } else {
            label.set_text(&text);
        }
    });
    picker.set_factory(Some(&factory));

    let custom = Entry::new();
    custom.add_css_class("glyphpick");
    custom.set_width_chars(4);
    custom.set_placeholder_text(Some("glyph"));

    // Initial selection: empty → none; a known glyph → its row; else → custom.
    if icon0.is_empty() {
        picker.set_selected(none_idx);
        custom.set_visible(false);
    } else if let Some(i) = glyphs.iter().position(|(_, g)| *g == icon0) {
        picker.set_selected(i as u32 + 1); // +1 for the leading "none" row
        custom.set_visible(false);
    } else {
        picker.set_selected(custom_idx);
        custom.set_text(&icon0);
        custom.set_visible(true);
    }
    r.append(&picker);
    r.append(&custom);

    let cmd = Entry::new();
    cmd.set_text(&cmd0);
    cmd.set_hexpand(true);
    cmd.set_placeholder_text(Some("command, or @menu, or blank"));
    cmd.set_tooltip_text(Some("Press Enter to apply"));
    r.append(&cmd);

    let color = ColorDialogButton::new(Some(ColorDialog::new()));
    color.set_rgba(&hex_to_rgba(if color0.is_empty() {
        "#99aabb"
    } else {
        &color0
    }));
    r.append(&color);

    // The effective glyph: the custom entry when "custom…" is selected,
    // otherwise the picked row's glyph.
    let glyph_value = {
        let picker_c = picker.clone();
        let custom_c = custom.clone();
        move || -> String {
            let sel = picker_c.selected();
            if sel == none_idx {
                String::new()
            } else if sel == custom_idx {
                custom_c.text().to_string()
            } else {
                // rows are offset by 1 (leading "none")
                glyphs
                    .get(sel as usize - 1)
                    .map(|(_, g)| g.to_string())
                    .unwrap_or_default()
            }
        }
    };
    let store = {
        let h = handle.clone();
        let cmd_c = cmd.clone();
        let color_c = color.clone();
        let glyph_value = glyph_value.clone();
        move || {
            set_header_slot(
                &mut h.cfg.borrow_mut().header,
                slot,
                glyph_value(),
                cmd_c.text().to_string(),
                rgba_to_hex(&color_c.rgba()),
            );
        }
    };

    // Picking a row: show the custom entry only for "custom…", then store+apply.
    let s = store.clone();
    let h = handle.clone();
    let custom_c = custom.clone();
    picker.connect_selected_notify(move |p| {
        custom_c.set_visible(p.selected() == custom_idx);
        s();
        h.apply();
    });
    // Typing a custom glyph: store live, apply on Enter / focus-out.
    let s = store.clone();
    custom.connect_changed(move |_| s());
    let h = handle.clone();
    custom.connect_activate(move |_| h.apply());
    let h = handle.clone();
    let focus = gtk::EventControllerFocus::new();
    focus.connect_leave(move |_| h.apply());
    custom.add_controller(focus);

    let s = store.clone();
    cmd.connect_changed(move |_| s());
    let h = handle.clone();
    color.connect_rgba_notify(move |_| {
        store();
        h.apply();
    });
    let h = handle.clone();
    cmd.connect_activate(move |_| h.apply());
    r
}

/// A labeled entry editing an `Option<String>` field of `panel[i]` (the clock
/// formats). Blank → `None` (falls back to the panel's built-in default).
fn fmt_row(
    handle: &BarHandle,
    label: &str,
    placeholder: &str,
    initial: Option<&str>,
    set: fn(&mut PanelConfig, Option<String>),
) -> GtkBox {
    let entry = Entry::new();
    entry.set_text(initial.unwrap_or(""));
    entry.set_hexpand(true);
    entry.set_placeholder_text(Some(placeholder));
    entry.set_tooltip_text(Some("strftime format; Enter to apply"));
    let h = handle.clone();
    let idx = current_panel_index();
    entry.connect_changed(move |e| {
        let t = e.text().to_string();
        let v = if t.trim().is_empty() { None } else { Some(t) };
        if let Some(p) = h.cfg.borrow_mut().panel.get_mut(idx) {
            set(p, v);
        }
    });
    let h = handle.clone();
    entry.connect_activate(move |_| h.apply());
    row(label, &entry)
}

// The detail builders are passed the selected panel index implicitly via this
// thread-local, set just before a detail is built. Simpler than threading the
// index through header_slot_row's existing signature.
thread_local! {
    static CUR_PANEL: Cell<usize> = const { Cell::new(0) };
}
fn current_panel_index() -> usize {
    CUR_PANEL.with(|c| c.get())
}

/// Header panel detail: clock formats, the four icon slots, and the power-menu
/// command strings the `@menu` popover runs.
fn header_detail(handle: &BarHandle, i: usize) -> GtkBox {
    // Seed default header buttons so the slots show what's currently on the bar.
    if handle.cfg.borrow().header.is_empty() {
        let lock = handle.cfg.borrow().actions.lock.clone();
        handle.cfg.borrow_mut().header = vec![
            HeaderButton {
                slot: HeaderSlot::TimeLeft,
                icon: "\u{f011}".into(),
                command: crate::panels::HEADER_MENU.into(),
                color: "#5fd75f".into(), // green
            },
            HeaderButton {
                slot: HeaderSlot::TimeRight,
                icon: "\u{f023}".into(),
                command: lock,
                color: "#d4af37".into(), // brass
            },
        ];
    }

    let page = page_box();

    // Clock formats (strftime). Stored on this panel's config.
    let (tf, df) = {
        let cfg = handle.cfg.borrow();
        let p = &cfg.panel[i];
        (p.time_format.clone(), p.date_format.clone())
    };
    page.append(&fmt_row(
        handle,
        "Time format",
        "%I:%M %p",
        tf.as_deref(),
        |p, v| p.time_format = v,
    ));
    page.append(&fmt_row(
        handle,
        "Date format",
        "%a %d %b",
        df.as_deref(),
        |p, v| p.date_format = v,
    ));

    // Show hostname
    let hh = CheckButton::with_label("Show hostname");
    hh.set_active(handle.cfg.borrow().panel[i].show_hostname);
    {
        let h = handle.clone();
        hh.connect_toggled(move |c| {
            if let Some(p) = h.cfg.borrow_mut().panel.get_mut(i) {
                p.show_hostname = c.is_active();
            }
            h.apply();
        });
    }
    page.append(&hh);

    // Short hostname
    let sh = CheckButton::with_label("Short hostname (strip domain)");
    sh.set_active(handle.cfg.borrow().panel[i].short_hostname);
    {
        let h = handle.clone();
        sh.connect_toggled(move |c| {
            if let Some(p) = h.cfg.borrow_mut().panel.get_mut(i) {
                p.short_hostname = c.is_active();
            }
            h.apply();
        });
    }
    page.append(&sh);

    // Show kernel
    let hk = CheckButton::with_label("Show kernel version");
    hk.set_active(handle.cfg.borrow().panel[i].show_kernel);
    {
        let h = handle.clone();
        hk.connect_toggled(move |c| {
            if let Some(p) = h.cfg.borrow_mut().panel.get_mut(i) {
                p.show_kernel = c.is_active();
            }
            h.apply();
        });
    }
    page.append(&hk);

    let sep = Label::new(Some(
        "Icons — a glyph + command per slot. '@menu' = power popover; blank = none.",
    ));
    sep.set_xalign(0.0);
    sep.set_margin_top(10);
    page.append(&sep);
    page.append(&header_slot_row(handle, HeaderSlot::TimeLeft, "time ◀"));
    page.append(&header_slot_row(handle, HeaderSlot::TimeRight, "time ▶"));
    page.append(&header_slot_row(handle, HeaderSlot::DateLeft, "date ◀"));
    page.append(&header_slot_row(handle, HeaderSlot::DateRight, "date ▶"));

    let sep = Label::new(Some("Power-menu commands (run by the @menu popover)"));
    sep.set_xalign(0.0);
    sep.set_margin_top(10);
    page.append(&sep);
    page.append(&command_row(
        handle,
        "Lock",
        &handle.cfg.borrow().actions.lock,
        |a, v| a.lock = v,
    ));
    page.append(&command_row(
        handle,
        "Logout",
        &handle.cfg.borrow().actions.logout,
        |a, v| a.logout = v,
    ));
    page.append(&command_row(
        handle,
        "Reboot",
        &handle.cfg.borrow().actions.reboot,
        |a, v| a.reboot = v,
    ));
    page.append(&command_row(
        handle,
        "Shutdown",
        &handle.cfg.borrow().actions.shutdown,
        |a, v| a.shutdown = v,
    ));
    page
}

/// Volume panel detail: the mixer launched on right-click, plus interval.
fn vol_detail(handle: &BarHandle, i: usize) -> GtkBox {
    let page = page_box();
    page.append(&interval_row(handle, i));
    page.append(&scroll_step_row(handle, i, 2.0));
    page.append(&command_row(
        handle,
        "Right-click mixer",
        &handle.cfg.borrow().actions.mixer,
        |a, v| a.mixer = v,
    ));
    page
}

/// Scroll-wheel step (%) spinner for the vol/bri panels.
fn scroll_step_row(handle: &BarHandle, i: usize, default: f64) -> GtkBox {
    let sp = SpinButton::with_range(0.5, 50.0, 0.5);
    sp.set_digits(1);
    sp.set_width_chars(5); // uniform with the other detail spinners so they align
    sp.set_value(handle.cfg.borrow().panel[i].scroll_step.unwrap_or(default));
    sp.set_tooltip_text(Some("Percent change per scroll-wheel notch"));
    let h = handle.clone();
    sp.connect_value_changed(move |s| {
        if let Some(p) = h.cfg.borrow_mut().panel.get_mut(i) {
            p.scroll_step = Some(s.value());
        }
        h.apply();
    });
    row("Scroll step (%)", &sp)
}

/// The per-panel update interval spinner (seconds).
fn interval_row(handle: &BarHandle, i: usize) -> GtkBox {
    let iv = SpinButton::with_range(0.1, 600.0, 0.5);
    iv.set_digits(1);
    iv.set_width_chars(5);
    iv.set_value(handle.cfg.borrow().panel[i].interval);
    iv.set_tooltip_text(Some("Seconds between updates for this panel"));
    let h = handle.clone();
    iv.connect_value_changed(move |s| {
        if let Some(p) = h.cfg.borrow_mut().panel.get_mut(i) {
            p.interval = s.value();
        }
        h.apply();
    });
    row("Update interval (s)", &iv)
}

/// The per-panel "show label row" toggle (the panel's "LABEL  value" header).
fn show_label_check(handle: &BarHandle, i: usize) -> CheckButton {
    let c = CheckButton::with_label("Show label row");
    c.set_active(handle.cfg.borrow().panel[i].show_label);
    c.set_tooltip_text(Some(
        "Off: just the graphic, no label/value header (e.g. cores under cpu)",
    ));
    let h = handle.clone();
    c.connect_toggled(move |c| {
        if let Some(p) = h.cfg.borrow_mut().panel.get_mut(i) {
            p.show_label = c.is_active();
        }
        h.apply();
    });
    c
}

/// A graph-height spinner for a panel. `default_h` is shown when unset (the
/// panel-type default). Changing it rebuilds the panel (height is baked into
/// the graph canvas), but that only happens on this explicit edit.
fn graph_height_row(handle: &BarHandle, i: usize, default_h: i32) -> GtkBox {
    let sp = SpinButton::with_range(6.0, 200.0, 1.0);
    sp.set_width_chars(5);
    let init = handle.cfg.borrow().panel[i]
        .graph_height
        .unwrap_or(default_h);
    sp.set_value(init as f64);
    sp.set_tooltip_text(Some("Graph height in pixels"));
    let h = handle.clone();
    sp.connect_value_changed(move |s| {
        if let Some(p) = h.cfg.borrow_mut().panel.get_mut(i) {
            p.graph_height = Some(s.value() as i32);
        }
        h.apply();
    });
    row("Graph height (px)", &sp)
}

/// A graph time-window spinner (seconds of history shown). Rebuilds the panel.
fn graph_window_row(handle: &BarHandle, i: usize) -> GtkBox {
    let sp = SpinButton::with_range(10.0, 600.0, 5.0);
    sp.set_width_chars(5);
    sp.set_value(handle.cfg.borrow().panel[i].graph_window.unwrap_or(60.0));
    sp.set_tooltip_text(Some("Seconds of history the graph spans"));
    let h = handle.clone();
    sp.connect_value_changed(move |s| {
        if let Some(p) = h.cfg.borrow_mut().panel.get_mut(i) {
            p.graph_window = Some(s.value());
        }
        h.apply();
    });
    row("Graph window (s)", &sp)
}

/// Graph width (px) — horizontal bars only (vertical bars fill the width).
fn graph_width_row(handle: &BarHandle, i: usize) -> GtkBox {
    let sp = SpinButton::with_range(16.0, 400.0, 4.0);
    sp.set_width_chars(5);
    sp.set_value(handle.cfg.borrow().panel[i].graph_width.unwrap_or(80) as f64);
    sp.set_tooltip_text(Some("Fixed graph width on a horizontal bar"));
    let h = handle.clone();
    sp.connect_value_changed(move |s| {
        if let Some(p) = h.cfg.borrow_mut().panel.get_mut(i) {
            p.graph_width = Some(s.value() as i32);
        }
        h.apply();
    });
    row("Graph width (px)", &sp)
}

/// Generic detail for the metric panels: interval, label toggle, plus a graph
/// toggle + height + window for the types that actually draw a history graph.
fn generic_detail(handle: &BarHandle, i: usize, kind: &str) -> GtkBox {
    let page = page_box();
    page.append(&interval_row(handle, i));
    if LABEL_TYPES.contains(&kind) {
        page.append(&show_label_check(handle, i));
    }
    if GRAPH_TYPES.contains(&kind) {
        let g = CheckButton::with_label("Show history graph");
        g.set_active(handle.cfg.borrow().panel[i].graph);
        let h = handle.clone();
        g.connect_toggled(move |c| {
            if let Some(p) = h.cfg.borrow_mut().panel.get_mut(i) {
                p.graph = c.is_active();
            }
            h.apply();
        });
        page.append(&g);
        page.append(&graph_height_row(handle, i, 24)); // GRAPH_H default
        page.append(&graph_window_row(handle, i));
        if handle.cfg.borrow().bar.edge.is_horizontal() {
            page.append(&graph_width_row(handle, i));
        }
    }
    page
}

// ---- Panels config tab (master list ▸ per-panel detail) --------------------

/// Build the detail pane for the panel at index `i`. Rebuilt on each selection.
fn panel_detail(handle: &BarHandle, i: usize) -> GtkBox {
    CUR_PANEL.with(|c| c.set(i));
    let kind = match handle.cfg.borrow().panel.get(i) {
        Some(p) => p.kind.clone(),
        None => return page_box(),
    };
    match kind.as_str() {
        "header" => header_detail(handle, i),
        "sensors" => temp_detail(handle, i),
        "weather" | "wx" => weather_detail(handle),
        "mail" => mail_detail(handle),
        "tray" => tray_detail(handle),
        "volume" => vol_detail(handle, i),
        "brightness" => {
            let page = page_box();
            page.append(&interval_row(handle, i));
            page.append(&scroll_step_row(handle, i, 5.0));
            page
        }
        // clock shares the header's format fields but has no icons.
        "clock" => {
            let page = page_box();
            let (tf, df) = {
                let cfg = handle.cfg.borrow();
                let p = &cfg.panel[i];
                (p.time_format.clone(), p.date_format.clone())
            };
            page.append(&fmt_row(
                handle,
                "Time format",
                "%I:%M %p",
                tf.as_deref(),
                |p, v| p.time_format = v,
            ));
            page.append(&fmt_row(
                handle,
                "Date format",
                "%a %d %b",
                df.as_deref(),
                |p, v| p.date_format = v,
            ));
            page
        }
        "top" => {
            let page = page_box();
            page.append(&interval_row(handle, i));
            page.append(&show_label_check(handle, i));
            let n = SpinButton::with_range(1.0, 20.0, 1.0);
            n.set_width_chars(5);
            n.set_value(handle.cfg.borrow().panel[i].count.unwrap_or(5) as f64);
            let h = handle.clone();
            n.connect_value_changed(move |s| {
                if let Some(p) = h.cfg.borrow_mut().panel.get_mut(i) {
                    p.count = Some(s.value() as usize);
                }
                h.apply();
            });
            page.append(&row("Processes shown", &n));

            // Horizontal bars only: wrap the entries into columns of this many
            // (a single row otherwise). Pointless on a vertical bar — it's
            // already a column — so only show it there.
            if handle.cfg.borrow().bar.edge.is_horizontal() {
                let c = SpinButton::with_range(1.0, 20.0, 1.0);
                c.set_width_chars(5);
                c.set_value(handle.cfg.borrow().panel[i].columns.unwrap_or(1) as f64);
                let h = handle.clone();
                c.connect_value_changed(move |s| {
                    if let Some(p) = h.cfg.borrow_mut().panel.get_mut(i) {
                        p.columns = Some(s.value() as i32);
                    }
                    h.apply();
                });
                page.append(&row("Items per column", &c));
            }
            page
        }
        "net" => {
            let page = generic_detail(handle, i, "net");
            let hr = CheckButton::with_label(
                "Human-readable (K→M/G suffix once raw exceeds 4 digits)",
            );
            hr.set_active(handle.cfg.borrow().panel[i].human_readable.unwrap_or(true));
            let h = handle.clone();
            hr.connect_toggled(move |c| {
                if let Some(p) = h.cfg.borrow_mut().panel.get_mut(i) {
                    p.human_readable = Some(c.is_active());
                }
                h.apply();
            });
            page.append(&hr);
            page
        }
        "disk" => {
            let page = generic_detail(handle, i, "disk");
            let cb = CheckButton::with_label("Show capacity (usage bar + used/total)");
            cb.set_active(handle.cfg.borrow().panel[i].show_capacity);
            let h = handle.clone();
            cb.connect_toggled(move |c| {
                if let Some(p) = h.cfg.borrow_mut().panel.get_mut(i) {
                    p.show_capacity = c.is_active();
                }
                h.apply();
            });
            page.append(&cb);
            let cb = CheckButton::with_label("Show R/W rate");
            cb.set_active(handle.cfg.borrow().panel[i].show_disk_total);
            let h = handle.clone();
            cb.connect_toggled(move |c| {
                if let Some(p) = h.cfg.borrow_mut().panel.get_mut(i) {
                    p.show_disk_total = c.is_active();
                }
                h.apply();
            });
            page.append(&cb);
            page
        }
        "cpu" => {
            let page = generic_detail(handle, i, "cpu");
            let s = SpinButton::with_range(-1.0, 255.0, 1.0);
            s.set_width_chars(5);
            s.set_value(handle.cfg.borrow().panel[i].core as f64);
            let h = handle.clone();
            s.connect_value_changed(move |s| {
                if let Some(p) = h.cfg.borrow_mut().panel.get_mut(i) {
                    p.core = s.value() as i32;
                }
                h.apply();
            });
            page.append(&row("Core (-1 = all, 0 = CPU0, 1 = CPU1, …)", &s));
            page
        }
        "uptime" => {
            let page = page_box();
            page.append(&interval_row(handle, i));
            let c = CheckButton::with_label("Show load averages (1 / 5 / 15 min)");
            c.set_active(handle.cfg.borrow().panel[i].show_load);
            let h = handle.clone();
            c.connect_toggled(move |c| {
                if let Some(p) = h.cfg.borrow_mut().panel.get_mut(i) {
                    p.show_load = c.is_active();
                }
                h.apply();
            });
            page.append(&c);
            page
        }
        // Panels with nothing to configure beyond presence/order.
        "tasks" => tasks_detail(handle, i),
        // Everything else is a metric panel: interval (+ graph where relevant).
        _ => generic_detail(handle, i, &kind),
    }
}

/// Install (once) a tiny stylesheet for prefs-window chrome that the ambient
/// GTK theme gets wrong — notably the master panel list, which Adwaita only
/// shades in its `:backdrop` (unfocused) state, so it *blends* into the window
/// exactly when focused. Pin a consistent background + separator either way.
fn ensure_prefs_css() {
    thread_local! { static DONE: Cell<bool> = const { Cell::new(false) }; }
    if DONE.with(|d| d.replace(true)) {
        return;
    }
    let provider = gtk::CssProvider::new();
    provider.load_from_data(
        ".panel-master, .panel-master:backdrop {\
           background-color: rgba(255,255,255,0.045);\
           border-right: 1px solid rgba(128,128,128,0.15); }\
         .panel-idx { font-family: monospace; }",
    );
    if let Some(display) = gtk::gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

fn panels_config_page(handle: &BarHandle) -> GtkBox {
    ensure_prefs_css();
    let page = page_box();
    page.append(&Label::new(Some(
        "Pick a panel on the left to configure it. Add / remove / reorder panels in the Layout tab.",
    )));

    let split = GtkBox::new(Orientation::Horizontal, 12);
    split.set_vexpand(true);

    // Master: the list of enabled panels (selectable).
    let master = ListBox::new();
    master.set_selection_mode(gtk::SelectionMode::Single);
    master.set_size_request(130, -1);
    master.add_css_class("panel-master");
    let master_scroll = gtk::ScrolledWindow::new();
    master_scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    master_scroll.set_child(Some(&master));

    // Detail: rebuilt whenever the selection changes.
    let detail_scroll = gtk::ScrolledWindow::new();
    detail_scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    detail_scroll.set_hexpand(true);
    detail_scroll.set_vexpand(true);

    split.append(&master_scroll);
    split.append(&detail_scroll);
    page.append(&split);

    // (Re)fill the master list from the current panel config. Stored as a
    // closure so the page's `map` handler can refresh it after Layout edits.
    let fill = {
        let handle = handle.clone();
        let master = master.clone();
        move || {
            let prev = master.selected_row().map(|r| r.index());
            while let Some(c) = master.first_child() {
                master.remove(&c);
            }
            let kinds: Vec<String> = handle
                .cfg
                .borrow()
                .panel
                .iter()
                .map(|p| p.kind.clone())
                .collect();
            for (i, kind) in kinds.iter().enumerate() {
                // Number in its own right-aligned monospace column so single- vs
                // double-digit indices don't shove the names out of alignment.
                let rrow = GtkBox::new(Orientation::Horizontal, 8);
                rrow.set_margin_top(4);
                rrow.set_margin_bottom(4);
                rrow.set_margin_start(6);
                rrow.set_margin_end(6);
                let num = Label::new(Some(&i.to_string()));
                num.add_css_class("panel-idx");
                num.set_width_chars(2);
                num.set_xalign(1.0);
                let name = Label::new(Some(kind));
                name.set_xalign(0.0);
                rrow.append(&num);
                rrow.append(&name);
                master.append(&rrow);
            }
            // Restore selection (clamped), else select the first row.
            let n = kinds.len() as i32;
            if n > 0 {
                let want = prev.unwrap_or(0).min(n - 1);
                if let Some(rrow) = master.row_at_index(want) {
                    master.select_row(Some(&rrow));
                }
            }
        }
    };

    // Selecting a row rebuilds the detail pane.
    {
        let handle = handle.clone();
        let detail_scroll = detail_scroll.clone();
        master.connect_row_selected(move |_, row| match row {
            Some(r) => detail_scroll.set_child(Some(&panel_detail(&handle, r.index() as usize))),
            None => detail_scroll.set_child(None::<&gtk::Widget>),
        });
    }

    // Keep the master list in sync with Layout-tab edits: refill each time this
    // page is shown (Notebook maps the visible page's child on tab switch).
    {
        let fill = fill.clone();
        master.connect_map(move |_| fill());
    }
    fill();

    // The split already claims the vertical space; don't let the save bar also
    // vexpand (it would steal ~half the height from the master/detail split).
    let sb = save_bar(handle);
    sb.set_vexpand(false);
    page.append(&sb);
    page
}

// ---- shared save bar -------------------------------------------------------

fn save_bar(handle: &BarHandle) -> GtkBox {
    let bar = GtkBox::new(Orientation::Horizontal, 8);
    bar.set_margin_top(12);
    bar.set_valign(gtk::Align::End);
    bar.set_vexpand(true);
    let status = Label::new(None);
    status.set_xalign(0.0);
    status.set_hexpand(true);
    // Ellipsize + zero natural width so a long save message can't widen the
    // window; the full path goes in the tooltip.
    status.set_ellipsize(gtk::pango::EllipsizeMode::End);
    status.set_width_chars(0);
    let save = Button::with_label("Save to config.toml");
    let h = handle.clone();
    let status_c = status.clone();
    save.connect_clicked(move |_| match save_config(&h) {
        Ok(p) => {
            status_c.set_text("Saved ✓");
            status_c.set_tooltip_text(Some(&format!("Saved to {}", p.display())));
        }
        Err(e) => status_c.set_text(&format!("Save failed: {e}")),
    });
    bar.append(&status);
    bar.append(&save);
    bar
}

fn save_config(handle: &BarHandle) -> std::io::Result<std::path::PathBuf> {
    let path = crate::config::config_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // Back up the previous config first, so a save can never be a one-way loss
    // (e.g. saving over a config the bar had degraded to defaults).
    if path.exists() {
        let _ = std::fs::copy(&path, crate::config::backup_path());
    }
    let body = toml::to_string_pretty(&*handle.cfg.borrow()).map_err(std::io::Error::other)?;
    std::fs::write(&path, body)?;
    // Also persist the active theme to its file, so font-size/color edits to
    // the current theme (incl. "default") survive a restart — config.toml only
    // stores the theme *name*.
    let name = handle.cfg.borrow().theme.clone();
    let name = if name.is_empty() {
        "default".to_string()
    } else {
        name
    };
    if crate::theme::is_valid_name(&name) {
        // If the active theme bundles panel colors, refresh that bundle from the
        // live config so it doesn't go stale relative to edits made since the
        // last "Save + panel colors". A plain theme (bundle = None) stays plain.
        if handle.theme.borrow().sensors.is_some() {
            let sensors = handle.cfg.borrow().temp.sensors.clone();
            let header = handle.cfg.borrow().header.clone();
            let mut t = handle.theme.borrow_mut();
            t.sensors = Some(sensors);
            t.header = Some(header);
        }
        // Propagate a theme-write failure: theme edits (colors/fonts) live only
        // in the theme file, so swallowing this would let the UI say "Saved"
        // while those edits are silently lost.
        save_theme_file(&name, &handle.theme.borrow())?;
    }
    Ok(path)
}
