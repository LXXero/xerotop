//! The preferences GUI — a normal toplevel window (not layer-shell) that edits
//! the running bar's config and theme *live*: every change mutates the shared
//! state on the `BarHandle` and calls `apply()`, so the bar re-renders as you
//! drag a slider or pick a color. "Save" persists config.toml / the theme file.

use crate::bar::BarHandle;
use crate::config::{
    Align, BarLength, Edge, HeaderButton, HeaderSlot, Layer, PanelConfig, TempSensor,
};
use crate::theme::{Theme, themes_dir};
use gtk::gdk::RGBA;
use gtk::glib;
use gtk::pango::FontDescription;
use gtk::prelude::*;
use gtk::{
    Box as GtkBox, Button, CheckButton, ColorDialog, ColorDialogButton, DropDown, Entry,
    FontDialog, FontDialogButton, Label, ListBox, Notebook, Orientation, Scale, SpinButton, Switch,
    Window,
};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

thread_local! {
    /// The single live prefs window, if open.
    static WINDOW: RefCell<Option<Window>> = const { RefCell::new(None) };
}

const PANEL_TYPES: [&str; 18] = [
    "header", "clock", "cpu", "cores", "mem", "gpu", "disk", "net", "temp", "weather", "uptime",
    "kbd", "bat", "vol", "bri", "top", "win", "tray",
];

/// Panel types that have a history graph (so the "graph" toggle is meaningful).
const GRAPH_TYPES: [&str; 6] = ["cpu", "mem", "gpu", "disk", "net", "temp"];

/// Open (or re-focus) the preferences window for the given bar.
pub fn open(handle: &BarHandle) {
    if let Some(w) = WINDOW.with(|w| w.borrow().clone()) {
        w.present();
        return;
    }

    let notebook = Notebook::new();
    notebook.append_page(&general_page(handle), Some(&Label::new(Some("General"))));
    notebook.append_page(&theme_page(handle), Some(&Label::new(Some("Theme"))));
    notebook.append_page(&panels_page(handle), Some(&Label::new(Some("Panels"))));
    notebook.append_page(&temp_page(handle), Some(&Label::new(Some("Sensors"))));
    notebook.append_page(&commands_page(handle), Some(&Label::new(Some("Commands"))));

    let window = Window::builder()
        .title("xerotop preferences")
        .default_width(440)
        .default_height(580)
        .child(&notebook)
        .build();
    window.connect_close_request(|_| {
        WINDOW.with(|w| *w.borrow_mut() = None);
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
    row(label, &entry)
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
        h.apply();
    });
    page.append(&row("Edge", &edge));

    // Thickness
    let thick = SpinButton::with_range(20.0, 600.0, 2.0);
    thick.set_value(cfg.bar.thickness as f64);
    let h = handle.clone();
    thick.connect_value_changed(move |s| {
        h.cfg.borrow_mut().bar.thickness = s.value() as i32;
        h.apply();
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
            h.apply();
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
    let mon = SpinButton::with_range(-1.0, 16.0, 1.0);
    mon.set_value(cfg.bar.monitor as f64);
    let h = handle.clone();
    mon.connect_value_changed(move |s| {
        h.cfg.borrow_mut().bar.monitor = s.value() as i32;
        h.apply();
    });
    page.append(&row("Monitor (-1 = auto)", &mon));

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
        h.apply();
    });
    page.append(&row("Layer (bottom = windows over bar)", &layer));

    // Opacity
    let opacity = Scale::with_range(Orientation::Horizontal, 0.0, 1.0, 0.01);
    opacity.set_value(cfg.bar.opacity);
    opacity.set_hexpand(true);
    opacity.set_draw_value(true);
    opacity.set_size_request(180, -1);
    let h = handle.clone();
    opacity.connect_value_changed(move |s| {
        h.cfg.borrow_mut().bar.opacity = s.value();
        h.apply();
    });
    page.append(&row("Opacity", &opacity));

    // Graph gamma
    let gamma = Scale::with_range(Orientation::Horizontal, 0.5, 4.0, 0.1);
    gamma.set_value(cfg.bar.graph_gamma);
    gamma.set_hexpand(true);
    gamma.set_draw_value(true);
    gamma.set_size_request(180, -1);
    let h = handle.clone();
    gamma.connect_value_changed(move |s| {
        h.cfg.borrow_mut().bar.graph_gamma = s.value();
        h.apply();
    });
    page.append(&row("Graph spikiness (gamma)", &gamma));

    // Smooth
    let smooth = Switch::new();
    smooth.set_active(cfg.bar.smooth);
    smooth.set_halign(gtk::Align::Start);
    let h = handle.clone();
    smooth.connect_active_notify(move |s| {
        h.cfg.borrow_mut().bar.smooth = s.is_active();
        h.apply();
    });
    page.append(&row("Smooth graph scroll", &smooth));

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

    // Tray layout
    let tray_cols = SpinButton::with_range(1.0, 32.0, 1.0);
    tray_cols.set_value(cfg.tray.columns as f64);
    let h = handle.clone();
    tray_cols.connect_value_changed(move |s| {
        h.cfg.borrow_mut().tray.columns = s.value() as i32;
        h.apply();
    });
    page.append(&row("Tray icons per row", &tray_cols));

    let tray_size = SpinButton::with_range(8.0, 64.0, 1.0);
    tray_size.set_value(cfg.tray.icon_size as f64);
    let h = handle.clone();
    tray_size.connect_value_changed(move |s| {
        h.cfg.borrow_mut().tray.icon_size = s.value() as i32;
        h.apply();
    });
    page.append(&row("Tray icon size (px)", &tray_size));

    // Weather (for the "weather" panel; fetched from wttr.in).
    let wx_loc = Entry::new();
    wx_loc.set_text(&cfg.weather.location);
    wx_loc.set_hexpand(true);
    wx_loc.set_placeholder_text(Some("city or lat,lon — blank = auto by IP"));
    wx_loc.set_tooltip_text(Some("Press Enter to apply"));
    let h = handle.clone();
    wx_loc.connect_changed(move |e| h.cfg.borrow_mut().weather.location = e.text().to_string());
    let h = handle.clone();
    wx_loc.connect_activate(move |_| h.apply());
    page.append(&row("Weather location", &wx_loc));

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
    page.append(&row("Weather units", &wx_units));

    let wx_iv = SpinButton::with_range(5.0, 240.0, 5.0);
    wx_iv.set_value(cfg.weather.interval_min);
    let h = handle.clone();
    wx_iv.connect_value_changed(move |s| {
        h.cfg.borrow_mut().weather.interval_min = s.value();
        h.apply();
    });
    page.append(&row("Weather refresh (min)", &wx_iv));

    let wx_cond = CheckButton::with_label(
        "Show condition text (else icon + temp only; full report on hover)",
    );
    wx_cond.set_active(cfg.weather.show_condition);
    let h = handle.clone();
    wx_cond.connect_toggled(move |c| {
        h.cfg.borrow_mut().weather.show_condition = c.is_active();
        h.apply();
    });
    page.append(&wx_cond);

    drop(cfg);
    page.append(&save_bar(handle));
    page
}

// ---- Theme page ------------------------------------------------------------

type Getter = fn(&Theme) -> String;
type Setter = fn(&mut Theme, String);

#[allow(clippy::type_complexity)]
const COLOR_FIELDS: [(&str, Getter, Setter); 11] = [
    (
        "Background",
        |t| t.background.clone(),
        |t, v| t.background = v,
    ),
    (
        "Foreground",
        |t| t.foreground.clone(),
        |t, v| t.foreground = v,
    ),
    ("Header / accent", |t| t.label.clone(), |t, v| t.label = v),
    ("Muted", |t| t.muted.clone(), |t, v| t.muted = v),
    (
        "Lock accent",
        |t| t.accent_lock.clone(),
        |t, v| t.accent_lock = v,
    ),
    (
        "Power accent",
        |t| t.accent_power.clone(),
        |t, v| t.accent_power = v,
    ),
    ("Graph green", |t| t.green.clone(), |t, v| t.green = v),
    ("Graph cyan", |t| t.cyan.clone(), |t, v| t.cyan = v),
    ("Graph amber", |t| t.amber.clone(), |t, v| t.amber = v),
    ("Graph red", |t| t.red.clone(), |t, v| t.red = v),
    ("Graph violet", |t| t.violet.clone(), |t, v| t.violet = v),
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
            h.apply();
        }
    });
    page.append(&row("Font", &font_btn));

    // Font-size tiers (gkrellm-style small/normal/large).
    let font_size_spin = |get: fn(&Theme) -> i32, set: fn(&mut Theme, i32)| {
        let sp = SpinButton::with_range(4.0, 96.0, 1.0);
        sp.set_value(get(&handle.theme.borrow()) as f64);
        let h = handle.clone();
        let ld = loading.clone();
        sp.connect_value_changed(move |s| {
            if ld.get() {
                return;
            }
            {
                let mut t = h.theme.borrow_mut();
                set(&mut t, s.value() as i32);
            }
            h.apply();
        });
        sp
    };
    let f_small = font_size_spin(|t| t.font_small, |t, v| t.font_small = v);
    let f_normal = font_size_spin(|t| t.font_normal, |t, v| t.font_normal = v);
    let f_large = font_size_spin(|t| t.font_large, |t, v| t.font_large = v);
    page.append(&row("Font small (sub, date)", &f_small));
    page.append(&row("Font normal (labels, values)", &f_normal));
    page.append(&row("Font large (clock)", &f_large));

    // One color button per theme color; remembered so loading a theme refreshes them.
    let buttons: Rc<RefCell<Vec<(Getter, ColorDialogButton)>>> = Rc::new(RefCell::new(Vec::new()));
    for (name, get, set) in COLOR_FIELDS {
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
            h.apply();
        });
        page.append(&row(name, &btn));
        buttons.borrow_mut().push((get, btn));
    }

    // Loading a theme: resolve the file, swap it in, refresh every widget.
    let h = handle.clone();
    let buttons_c = buttons.clone();
    let font_c = font_btn.clone();
    let (fs_c, fn_c, fl_c) = (f_small.clone(), f_normal.clone(), f_large.clone());
    let ld = loading.clone();
    selector.connect_selected_notify(move |d| {
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
        ld.set(true);
        font_c.set_font_desc(&FontDescription::from_string(&t.font_family));
        fs_c.set_value(t.font_small as f64);
        fn_c.set_value(t.font_normal as f64);
        fl_c.set_value(t.font_large as f64);
        for (get, btn) in buttons_c.borrow().iter() {
            btn.set_rgba(&hex_to_rgba(&get(&t)));
        }
        *h.theme.borrow_mut() = t;
        ld.set(false);
        h.apply();
    });
    // Prepend the selector row at the top of the page.
    page.prepend(&row("Theme", &selector));

    // Save-theme-as bar.
    let save_row = GtkBox::new(Orientation::Horizontal, 8);
    save_row.set_margin_top(8);
    let name_entry = Entry::new();
    name_entry.set_placeholder_text(Some("theme name"));
    name_entry.set_hexpand(true);
    let save_theme = Button::with_label("Save theme as…");
    let h = handle.clone();
    let entry_c = name_entry.clone();
    let selector_c = selector.clone();
    save_theme.connect_clicked(move |_| {
        let name = entry_c.text().trim().to_string();
        // Reject "default" and anything that isn't a strict slug (path-safe).
        if name == "default" || !crate::theme::is_valid_name(&name) {
            entry_c.set_text("");
            entry_c.set_placeholder_text(Some("use letters, digits, - or _ (not 'default')"));
            return;
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
    });
    save_row.append(&name_entry);
    save_row.append(&save_theme);
    page.append(&save_row);

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

fn panels_page(handle: &BarHandle) -> GtkBox {
    let page = page_box();
    page.append(&Label::new(Some(
        "Panels render top-to-bottom in this order.",
    )));

    // Column header explaining each control.
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
    head.append(&mk("panel", 8, false));
    head.append(&mk("interval (s)", 12, false));
    head.append(&mk("", 0, true)); // graph column / spacer
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
            time_format: None,
            date_format: None,
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
    let (kind, interval, graph) = {
        let cfg = handle.cfg.borrow();
        let p = &cfg.panel[i];
        (p.kind.clone(), p.interval, p.graph)
    };

    let name = Label::new(Some(&kind));
    name.set_xalign(0.0);
    name.set_width_chars(8);
    r.append(&name);

    let iv = SpinButton::with_range(0.1, 600.0, 0.5);
    iv.set_digits(1);
    iv.set_value(interval);
    iv.set_width_chars(11);
    iv.set_tooltip_text(Some("Seconds between updates for this panel"));
    let h = handle.clone();
    iv.connect_value_changed(move |s| {
        h.cfg.borrow_mut().panel[i].interval = s.value();
        h.apply();
    });
    r.append(&iv);

    // Only panels that actually draw a history graph get the toggle.
    if GRAPH_TYPES.contains(&kind.as_str()) {
        let g = CheckButton::with_label("graph");
        g.set_active(graph);
        let h = handle.clone();
        g.connect_toggled(move |c| {
            h.cfg.borrow_mut().panel[i].graph = c.is_active();
            h.apply();
        });
        r.append(&g);
    }

    let spacer = GtkBox::new(Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    r.append(&spacer);

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

    let del = Button::from_icon_name("user-trash-symbolic");
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
        })
        .collect()
}

fn temp_page(handle: &BarHandle) -> GtkBox {
    // Nothing configured yet = auto-detect. Make those defaults visible/editable
    // by seeding the list with them, so adding a sensor appends instead of
    // silently replacing the whole auto set. (Reproduces the same sensors, so
    // the bar looks identical; only persisted if the user hits Save.)
    if handle.cfg.borrow().temp.sensors.is_empty() {
        handle.cfg.borrow_mut().temp.sensors = default_temp_sensors();
    }

    let page = page_box();
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
        let n = h.cfg.borrow().temp.sensors.len();
        h.cfg.borrow_mut().temp.sensors.push(TempSensor {
            chip: chip.clone(),
            input: input.clone(),
            label: label.clone(),
            color: SENSOR_COLORS[n % SENSOR_COLORS.len()].to_string(),
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

    page.append(&save_bar(handle));
    page
}

fn populate_temp_list(handle: &BarHandle, list: &ListBox) {
    while let Some(c) = list.first_child() {
        list.remove(&c);
    }
    let n = handle.cfg.borrow().temp.sensors.len();
    for i in 0..n {
        list.append(&temp_sensor_row(handle, list, i, n));
    }
}

fn temp_sensor_row(handle: &BarHandle, list: &ListBox, i: usize, n: usize) -> GtkBox {
    let (chip, input, label, color) = {
        let cfg = handle.cfg.borrow();
        let s = &cfg.temp.sensors[i];
        (
            s.chip.clone(),
            s.input.clone(),
            s.label.clone(),
            s.color.clone(),
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
        h.cfg.borrow_mut().temp.sensors[i].color = rgba_to_hex(&b.rgba());
        h.apply();
    });
    r.append(&color_btn);

    let spacer = GtkBox::new(Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    r.append(&spacer);

    let up = Button::from_icon_name("go-up-symbolic");
    up.set_sensitive(i > 0);
    let h = handle.clone();
    let list_c = list.clone();
    up.connect_clicked(move |_| {
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
        h.cfg.borrow_mut().temp.sensors.swap(i, i + 1);
        h.apply();
        populate_temp_list(&h, &list_c);
    });
    r.append(&down);

    let del = Button::from_icon_name("user-trash-symbolic");
    let h = handle.clone();
    let list_c = list.clone();
    del.connect_clicked(move |_| {
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

    let glyph = Entry::new();
    glyph.set_text(&icon0);
    glyph.set_width_chars(3);
    glyph.set_placeholder_text(Some("glyph"));
    r.append(&glyph);

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

    // Store on edit (no apply per keystroke); apply on Enter / color pick.
    let store = {
        let h = handle.clone();
        let glyph_c = glyph.clone();
        let cmd_c = cmd.clone();
        let color_c = color.clone();
        move || {
            set_header_slot(
                &mut h.cfg.borrow_mut().header,
                slot,
                glyph_c.text().to_string(),
                cmd_c.text().to_string(),
                rgba_to_hex(&color_c.rgba()),
            );
        }
    };
    let s = store.clone();
    glyph.connect_changed(move |_| s());
    let s = store.clone();
    cmd.connect_changed(move |_| s());
    let h = handle.clone();
    color.connect_rgba_notify(move |_| {
        store();
        h.apply();
    });
    let h = handle.clone();
    glyph.connect_activate(move |_| h.apply());
    let h = handle.clone();
    cmd.connect_activate(move |_| h.apply());
    r
}

fn commands_page(handle: &BarHandle) -> GtkBox {
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
    page.append(&Label::new(Some(
        "Header icons — a glyph + command per slot. '@menu' = power popover; blank = none.",
    )));
    page.append(&header_slot_row(handle, HeaderSlot::TimeLeft, "time ◀"));
    page.append(&header_slot_row(handle, HeaderSlot::TimeRight, "time ▶"));
    page.append(&header_slot_row(handle, HeaderSlot::DateLeft, "date ◀"));
    page.append(&header_slot_row(handle, HeaderSlot::DateRight, "date ▶"));

    let sep = Label::new(Some("Commands (the power menu's items + the volume mixer)"));
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
    page.append(&command_row(
        handle,
        "Volume mixer",
        &handle.cfg.borrow().actions.mixer,
        |a, v| a.mixer = v,
    ));

    page.append(&save_bar(handle));
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
    let body = toml::to_string_pretty(&*handle.cfg.borrow()).map_err(std::io::Error::other)?;
    std::fs::write(&path, body)?;
    Ok(path)
}
