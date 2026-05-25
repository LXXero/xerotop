//! Assembles a layer-shell bar from config (orientation-agnostic) and runs the
//! single central scheduler. The whole bar is re-rendered in place from the
//! in-memory `Config` whenever `BarHandle::apply` is called, so the prefs GUI
//! can change anything live without restarting.

use crate::config::{Config, Edge};
use crate::panels::{self, Panel};
use crate::power;
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
    root: GtkBox,
    theme_css: gtk::CssProvider,
    /// Bumped on every `apply()`; the live scheduler breaks when it sees a newer
    /// generation, so stale timers from a previous layout stop on their next tick.
    generation: Rc<Cell<u64>>,
}

/// Which layer-shell edges to anchor for a given bar edge: the chosen edge plus
/// the two perpendicular ones (so the bar spans the full side), opposite off.
fn anchors(edge: Edge) -> [(LsEdge, bool); 4] {
    match edge {
        Edge::Right => [
            (LsEdge::Right, true),
            (LsEdge::Top, true),
            (LsEdge::Bottom, true),
            (LsEdge::Left, false),
        ],
        Edge::Left => [
            (LsEdge::Left, true),
            (LsEdge::Top, true),
            (LsEdge::Bottom, true),
            (LsEdge::Right, false),
        ],
        Edge::Top => [
            (LsEdge::Top, true),
            (LsEdge::Left, true),
            (LsEdge::Right, true),
            (LsEdge::Bottom, false),
        ],
        Edge::Bottom => [
            (LsEdge::Bottom, true),
            (LsEdge::Left, true),
            (LsEdge::Right, true),
            (LsEdge::Top, false),
        ],
    }
}

impl BarHandle {
    /// Re-render the entire bar from the current config: styling, geometry,
    /// orientation, panels and scheduler. Safe to call any number of times.
    pub fn apply(&self) {
        let cfg = self.cfg.borrow();
        panels::set_gamma(cfg.bar.graph_gamma);

        // Live theme/opacity override (above the base stylesheet's .bar rule).
        self.theme_css.load_from_data(&format!(
            ".bar {{ background-color: rgba(16,16,20,{:.3}); }}",
            cfg.bar.opacity.clamp(0.0, 1.0)
        ));

        // Pin to the configured output if it exists; monitor < 0 (or out of
        // range) means "let the compositor decide".
        if cfg.bar.monitor >= 0
            && let Some(display) = gtk::gdk::Display::default()
            && let Some(monitor) = display
                .monitors()
                .item(cfg.bar.monitor as u32)
                .and_downcast::<gtk::gdk::Monitor>()
        {
            self.window.set_monitor(Some(&monitor));
        }

        // Edge → anchors, orientation and thickness.
        let edge = cfg.bar.edge;
        let horizontal = edge.is_horizontal();
        for (e, on) in anchors(edge) {
            self.window.set_anchor(e, on);
        }
        if horizontal {
            self.window.set_height_request(cfg.bar.thickness);
            self.window.set_width_request(-1);
            self.root.set_orientation(Orientation::Horizontal);
        } else {
            self.window.set_width_request(cfg.bar.thickness);
            self.window.set_height_request(-1);
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
                // Separator rule between panels (not before the first one).
                if !panels.is_empty() {
                    let rule = GtkBox::new(Orientation::Horizontal, 0);
                    rule.add_css_class("rule");
                    self.root.append(&rule);
                }
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

    let theme_css = gtk::CssProvider::new();
    if let Some(display) = gtk::gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &theme_css,
            gtk::STYLE_PROVIDER_PRIORITY_USER,
        );
    }

    // Root box lays panels along the bar's main axis (orientation set in apply).
    let root = GtkBox::new(Orientation::Vertical, 4);
    root.add_css_class("bar");
    window.set_child(Some(&root));

    let handle = BarHandle {
        window: window.clone(),
        cfg: Rc::new(RefCell::new(cfg)),
        root,
        theme_css,
        generation: Rc::new(Cell::new(0)),
    };
    handle.apply();
    window.present();
    handle
}
