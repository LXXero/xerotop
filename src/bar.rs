//! Assembles a layer-shell bar from config (orientation-agnostic) and runs the
//! single central scheduler. The whole bar is re-rendered in place from the
//! in-memory `Config` whenever `BarHandle::apply` is called, so the prefs GUI
//! can change anything live without restarting.

use crate::config::{Align, BarLength as Length, Config, Edge, Layer};
use crate::panels::{self, Panel};
use crate::power;
use crate::theme::Theme;
use gtk::prelude::*;
use gtk::{Application, ApplicationWindow, Box as GtkBox, Orientation, glib};
use gtk4_layer_shell::{Edge as LsEdge, Layer as LsLayer, LayerShell};

fn ls_layer(l: Layer) -> LsLayer {
    match l {
        Layer::Background => LsLayer::Background,
        Layer::Bottom => LsLayer::Bottom,
        Layer::Top => LsLayer::Top,
        Layer::Overlay => LsLayer::Overlay,
    }
}
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

/// A live handle to the running bar. Clone it freely; all clones share the same
/// window, config and styling. Mutate `cfg`, then call `apply()` to re-render.
#[derive(Clone)]
pub struct BarHandle {
    pub window: ApplicationWindow,
    pub cfg: Rc<RefCell<Config>>,
    /// Active visual theme; edited live by the prefs GUI, applied by `apply()`.
    pub theme: Rc<RefCell<Theme>>,
    root: GtkBox,
    theme_css: gtk::CssProvider,
    /// Bumped on every `apply()`; the live scheduler breaks when it sees a newer
    /// generation, so stale timers from a previous layout stop on their next tick.
    generation: Rc<Cell<u64>>,
}

/// The layer-shell edges relative to a bar edge: (main, opposite, start, end).
/// `start`/`end` are the two perpendicular edges (top/bottom for a vertical bar,
/// left/right for a horizontal one).
fn edges(edge: Edge) -> (LsEdge, LsEdge, LsEdge, LsEdge) {
    match edge {
        Edge::Right => (LsEdge::Right, LsEdge::Left, LsEdge::Top, LsEdge::Bottom),
        Edge::Left => (LsEdge::Left, LsEdge::Right, LsEdge::Top, LsEdge::Bottom),
        Edge::Top => (LsEdge::Top, LsEdge::Bottom, LsEdge::Left, LsEdge::Right),
        Edge::Bottom => (LsEdge::Bottom, LsEdge::Top, LsEdge::Left, LsEdge::Right),
    }
}

/// The monitor the bar targets (by connector name, else the first), for geometry.
fn target_monitor(cfg: &Config) -> Option<gtk::gdk::Monitor> {
    let monitors = gtk::gdk::Display::default()?.monitors();
    let n = monitors.n_items();
    if cfg.bar.monitor != "auto" {
        for i in 0..n {
            if let Some(m) = monitors.item(i).and_downcast::<gtk::gdk::Monitor>() {
                if m.connector().as_deref() == Some(&cfg.bar.monitor) {
                    return Some(m);
                }
            }
        }
    }
    // Fall back to the first monitor for geometry.
    (0..n).find_map(|i| monitors.item(i).and_downcast::<gtk::gdk::Monitor>())
}

impl BarHandle {
    /// Regenerate the stylesheet from the theme + config (colors, fonts,
    /// opacity). CSS-only: does NOT rebuild panels, so graph history survives.
    /// Use this for appearance tweaks that don't change layout or structure.
    pub fn restyle(&self) {
        let cfg = self.cfg.borrow();
        let theme = self.theme.borrow();
        // Config font-size overrides (if any) win over the theme's defaults.
        let mut eff = theme.clone();
        if let Some(v) = cfg.font.small {
            eff.font_small = v;
        }
        if let Some(v) = cfg.font.normal {
            eff.font_normal = v;
        }
        if let Some(v) = cfg.font.large {
            eff.font_large = v;
        }
        self.theme_css.load_from_data(&eff.css(cfg.bar.opacity, cfg.bar.corner_radius, cfg.bar.corner_mode, cfg.bar.edge));
        // Clip children to the rounded bar corners (must be done programmatically
        // — GTK4 CSS has no "overflow" property).
        self.root.set_overflow(if cfg.bar.corner_radius > 0 {
            gtk::Overflow::Hidden
        } else {
            gtk::Overflow::Visible
        });
    }

    /// Apply window geometry: monitor, stacking layer, edge/anchors, thickness,
    /// length, orientation. No panel rebuild — the graphs adapt to the new
    /// width, so this doesn't reset graph history either.
    pub fn relayout(&self) {
        let cfg = self.cfg.borrow();
        let monitor = target_monitor(&cfg);
        // Pin to the configured output if requested; "auto" = compositor's choice
        // (but we still use the first output's geometry for full length).
        // Clear any previous pin on the auto path so it actually unpins live.
        if cfg.bar.monitor != "auto"
            && let Some(m) = &monitor
        {
            self.window.set_monitor(Some(m));
        } else {
            self.window.set_monitor(None);
        }

        self.window.set_layer(ls_layer(cfg.bar.layer));

        let edge = cfg.bar.edge;
        let horizontal = edge.is_horizontal();
        let (main, opp, start, end) = edges(edge);

        // Long-axis length: a fixed pixel count, or the monitor's full extent.
        // We size it explicitly (rather than relying on both-edge anchoring to
        // stretch) because the window is non-resizable — see build().
        let full_len = monitor.as_ref().map(|m| {
            let g = m.geometry();
            if horizontal { g.width() } else { g.height() }
        });
        let (length_px, anchor_start, anchor_end) = match cfg.bar.length {
            Length::Full => (full_len.filter(|&v| v > 0).unwrap_or(-1), true, true),
            Length::Px(n) => {
                let (s, e) = match cfg.bar.align {
                    Align::Start => (true, false),
                    Align::End => (false, true),
                    Align::Center => (false, false),
                };
                (n, s, e)
            }
        };

        self.window.set_anchor(main, true);
        self.window.set_anchor(opp, false);
        self.window.set_anchor(start, anchor_start);
        self.window.set_anchor(end, anchor_end);

        if horizontal {
            self.window.set_height_request(cfg.bar.thickness);
            self.window.set_width_request(length_px);
            self.root.set_height_request(cfg.bar.thickness);
            self.root.set_width_request(length_px);
            self.root.set_orientation(Orientation::Horizontal);
        } else {
            self.window.set_width_request(cfg.bar.thickness);
            self.window.set_height_request(length_px);
            self.root.set_width_request(cfg.bar.thickness);
            self.root.set_height_request(length_px);
            self.root.set_orientation(Orientation::Vertical);
        }
        // Only reserve compositor space on Top/Overlay layers so Bottom and
        // Background bars don't prevent windows from maximizing over them.
        if matches!(cfg.bar.layer, Layer::Top | Layer::Overlay) {
            self.window.auto_exclusive_zone_enable();
        }
    }

    /// Live-apply only the mail config (command / dir / interval). No panel
    /// rebuild — the click handler reads the command live and the counter thread
    /// is re-pointed at the new dir/interval, so graph history survives.
    pub fn apply_mail(&self) {
        panels::refresh_mail_config(self.cfg.borrow().mail.clone());
    }

    /// Full re-render from the current config: styling, geometry, panels and
    /// scheduler. Safe to call any number of times.
    pub fn apply(&self) {
        {
            let cfg = self.cfg.borrow();
            let theme = self.theme.borrow();
            panels::set_gamma(cfg.bar.graph_gamma);
            panels::set_meter_thickness(cfg.bar.meter_thickness);
            panels::set_palette(theme.palette());
            panels::set_tray(cfg.tray.columns, cfg.tray.icon_size);
            panels::set_horizontal(cfg.bar.edge.is_horizontal());
            panels::set_temp_config(cfg.temp.clone());
            panels::set_weather_config(cfg.weather.clone());
            panels::set_mail_config(cfg.mail.clone());
            panels::set_header_buttons(cfg.header.clone());
        }
        self.restyle();
        self.relayout();

        let cfg = self.cfg.borrow();
        // Rebuild panels: drop the old widgets, build fresh from config.
        while let Some(child) = self.root.first_child() {
            self.root.remove(&child);
        }
        // Smooth scrolling is power-aware: `smooth` on AC, `smooth_battery` on
        // battery. Baked into the panels' frame clock at build time, so an
        // AC<->battery transition rebuilds (handled in the scheduler below).
        let on_battery = power::on_battery();
        let (smooth_ac, smooth_bat) = (cfg.bar.smooth, cfg.bar.smooth_battery);
        let smooth = if on_battery { smooth_bat } else { smooth_ac };
        panels::reset_smooth_registry(); // old graphs are about to be dropped
        panels::reset_panel_hosts(); // and old host callbacks must stop firing
        let mut panels: Vec<Panel> = Vec::new();
        let build_order: Box<dyn Iterator<Item = &crate::config::PanelConfig>> = if cfg.bar.reverse {
            Box::new(cfg.panel.iter().rev())
        } else {
            Box::new(cfg.panel.iter())
        };
        for pcfg in build_order {
            if let Some(panel) = panels::build(pcfg, smooth, &cfg.actions) {
                self.root.append(&panel.root);
                panels.push(panel);
            }
        }

        // Prime once so the bar isn't blank before the first tick.
        for p in &panels {
            (p.update)();
        }
        // Any polling worker now lacking a panel (e.g. one was just removed) has
        // an empty callback list → drop it so its thread stops sampling.
        panels::stop_idle_hosts();

        // Single central scheduler: a 250 ms base tick updates each panel when
        // its (possibly fractional) interval has elapsed, stretched by the
        // battery multiplier on battery. A generation guard retires the timer
        // installed by a previous apply().
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        let gen_cell = self.generation.clone();
        let panels = Rc::new(panels);
        let mult = cfg.power.battery_interval_multiplier.max(1.0);
        let last = Rc::new(RefCell::new(vec![Instant::now(); panels.len()]));
        drop(cfg);
        // On AC<->battery flips, toggle smooth scrolling on the existing graphs
        // in place (no rebuild → graph history is preserved).
        let was_battery = Rc::new(Cell::new(on_battery));
        let handle = self.clone();
        glib::timeout_add_local(Duration::from_millis(250), move || {
            handle.relayout();
            if gen_cell.get() != generation {
                return glib::ControlFlow::Break;
            }
            let on_battery = power::on_battery();
            if on_battery != was_battery.get() {
                was_battery.set(on_battery);
                panels::set_all_smooth(if on_battery { smooth_bat } else { smooth_ac });
            }
            let m = if on_battery { mult } else { 1.0 };
            let now = Instant::now();
            let mut last = last.borrow_mut();
            for (i, p) in panels.iter().enumerate() {
                if now.duration_since(last[i]).as_secs_f64() >= p.interval * m {
                    (p.update)();
                    last[i] = now;
                }
            }
            glib::ControlFlow::Continue
        });
    }
}

pub fn build(app: &Application, cfg: Config) -> BarHandle {
    let window = ApplicationWindow::builder().application(app).build();
    window.init_layer_shell();
    window.set_namespace(Some("xerotop"));
    window.add_css_class("xerotop"); // scopes the transparent-window rule to us
    // Size to exactly our request every time. A resizable window remembers the
    // largest size it ever had, so reducing thickness wouldn't shrink it back.
    window.set_resizable(false);

    let theme_css = gtk::CssProvider::new();
    if let Some(display) = gtk::gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &theme_css,
            gtk::STYLE_PROVIDER_PRIORITY_USER,
        );
    }

    // Root box lays panels along the bar's main axis (orientation set in apply).
    let root = GtkBox::new(Orientation::Vertical, 2);
    root.add_css_class("bar");
    window.set_child(Some(&root));

    let theme = crate::theme::resolve(&cfg.theme);
    let handle = BarHandle {
        window: window.clone(),
        cfg: Rc::new(RefCell::new(cfg)),
        theme: Rc::new(RefCell::new(theme)),
        root: root.clone(),
        theme_css,
        generation: Rc::new(Cell::new(0)),
    };
    handle.apply();
    // React to monitor hotplug: re-pin the bar when outputs change.
    if let Some(display) = gtk::gdk::Display::default() {
        let h = handle.clone();
        display.monitors().connect_items_changed(move |_, _, removed, added| {
            if removed > 0 || added > 0 {
                h.relayout();
                h.window.present();
            }
        });
    }
    install_context_menu(&handle, &root);
    window.present();
    handle
}

/// gkrellm-style: right-click any dead space on the bar to get a small menu
/// (Preferences / Quit). Right-clicks consumed by interactive panels (e.g. tray
/// items, which have their own button-3 menus) don't reach this.
fn install_context_menu(handle: &BarHandle, root: &GtkBox) {
    let gesture = gtk::GestureClick::new();
    gesture.set_button(3); // right button
    let h = handle.clone();
    let root_w = root.clone();
    gesture.connect_pressed(move |_, _, x, y| {
        // Only over dead space: if the click landed inside an interactive panel
        // (a button, a meter with its own scroll/right-click, or the tray/taskbar
        // areas where icons can be misclicked), let that panel own the click.
        if let Some(picked) = root_w.pick(x, y, gtk::PickFlags::DEFAULT) {
            let mut w = Some(picked);
            while let Some(cur) = w {
                if cur.downcast_ref::<gtk::Button>().is_some()
                    || cur.has_css_class("meter")
                    || cur.has_css_class("tray")
                    || cur.has_css_class("taskbar")
                {
                    return;
                }
                w = cur.parent();
            }
        }

        let popover = gtk::Popover::new();
        popover.set_has_arrow(false);
        popover.set_parent(&root_w);
        popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));

        let menu = GtkBox::new(Orientation::Vertical, 0);
        menu.add_css_class("menu");
        let prefs = gtk::Button::with_label("Preferences");
        prefs.add_css_class("menu-item");
        let quit = gtk::Button::with_label("Quit xerotop");
        quit.add_css_class("menu-item");
        menu.append(&prefs);
        menu.append(&quit);
        popover.set_child(Some(&menu));

        let h2 = h.clone();
        let pop = popover.clone();
        prefs.connect_clicked(move |_| {
            pop.popdown();
            crate::prefs::open(&h2);
        });
        let h3 = h.clone();
        let pop = popover.clone();
        quit.connect_clicked(move |_| {
            pop.popdown();
            if let Some(app) = h3.window.application() {
                app.quit();
            }
        });
        popover.connect_closed(|p| p.unparent());
        popover.popup();
    });
    root.add_controller(gesture);
}
