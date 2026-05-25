//! Assembles a layer-shell bar from config (orientation-agnostic) and runs the
//! single central scheduler. The whole bar is re-rendered in place from the
//! in-memory `Config` whenever `BarHandle::apply` is called, so the prefs GUI
//! can change anything live without restarting.

use crate::config::{Align, BarLength as Length, Config, Edge};
use crate::panels::{self, Panel};
use crate::power;
use crate::theme::Theme;
use gtk::prelude::*;
use gtk::{Application, ApplicationWindow, Box as GtkBox, Orientation, glib};
use gtk4_layer_shell::{Edge as LsEdge, Layer, LayerShell};
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

/// The monitor the bar targets (configured index, else the first), for geometry.
fn target_monitor(cfg: &Config) -> Option<gtk::gdk::Monitor> {
    let monitors = gtk::gdk::Display::default()?.monitors();
    let idx = if cfg.bar.monitor >= 0 {
        cfg.bar.monitor as u32
    } else {
        0
    };
    monitors
        .item(idx)
        .and_downcast::<gtk::gdk::Monitor>()
        .or_else(|| monitors.item(0).and_downcast::<gtk::gdk::Monitor>())
}

impl BarHandle {
    /// Re-render the entire bar from the current config: styling, geometry,
    /// orientation, panels and scheduler. Safe to call any number of times.
    pub fn apply(&self) {
        let cfg = self.cfg.borrow();
        let theme = self.theme.borrow();
        panels::set_gamma(cfg.bar.graph_gamma);
        panels::set_palette(theme.palette());
        panels::set_tray(cfg.tray.columns, cfg.tray.icon_size);

        // Generate the whole stylesheet from the theme (colors + font); the bar
        // background alpha comes from config opacity.
        self.theme_css.load_from_data(&theme.css(cfg.bar.opacity));
        drop(theme);

        let monitor = target_monitor(&cfg);
        // Pin to the configured output if requested; monitor < 0 = compositor's
        // choice (but we still use the first output's geometry for full length).
        // Clear any previous pin on the auto path so it actually unpins live.
        if cfg.bar.monitor >= 0
            && let Some(m) = &monitor
        {
            self.window.set_monitor(Some(m));
        } else {
            self.window.set_monitor(None);
        }

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
            self.root.set_orientation(Orientation::Horizontal);
        } else {
            self.window.set_width_request(cfg.bar.thickness);
            self.window.set_height_request(length_px);
            self.root.set_orientation(Orientation::Vertical);
        }
        self.window.auto_exclusive_zone_enable();

        // Rebuild panels: drop the old widgets, build fresh from config.
        while let Some(child) = self.root.first_child() {
            self.root.remove(&child);
        }
        let mut panels: Vec<Panel> = Vec::new();
        for pcfg in cfg.panel.iter() {
            if let Some(panel) = panels::build(pcfg, cfg.bar.smooth, &cfg.actions) {
                self.root.append(&panel.root);
                panels.push(panel);
            }
        }

        // Prime once so the bar isn't blank before the first tick.
        for p in &panels {
            (p.update)();
        }

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
        glib::timeout_add_local(Duration::from_millis(250), move || {
            if gen_cell.get() != generation {
                return glib::ControlFlow::Break;
            }
            let m = if power::on_battery() { mult } else { 1.0 };
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
    window.set_layer(Layer::Top);
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
