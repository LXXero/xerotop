//! Assembles a layer-shell bar from config (orientation-agnostic) and runs the
//! single central scheduler: one 1 Hz timer that updates each panel when due,
//! stretching every interval by the battery multiplier when on battery.

use crate::config::{Config, Edge};
use crate::panels::{self, Panel};
use crate::power;
use gtk::prelude::*;
use gtk::{Application, ApplicationWindow, Box as GtkBox, Orientation, glib};
use gtk4_layer_shell::{Edge as LsEdge, Layer, LayerShell};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

pub fn build(app: &Application, cfg: &Config) {
    panels::set_gamma(cfg.bar.graph_gamma);

    // Opacity override (higher priority than the base stylesheet's .bar rule).
    if let Some(display) = gtk::gdk::Display::default() {
        let provider = gtk::CssProvider::new();
        provider.load_from_data(&format!(
            ".bar {{ background-color: rgba(16,16,20,{:.3}); }}",
            cfg.bar.opacity.clamp(0.0, 1.0)
        ));
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_USER,
        );
    }

    let window = ApplicationWindow::builder().application(app).build();

    window.init_layer_shell();
    window.set_layer(Layer::Top);
    window.set_namespace(Some("xerotop"));

    // Anchor the chosen edge plus the two perpendicular edges so the bar spans
    // the full screen side; leave the opposite edge unanchored.
    let edge = cfg.bar.edge;
    let horizontal = edge.is_horizontal();
    let anchors = match edge {
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
    };
    for (e, on) in anchors {
        window.set_anchor(e, on);
    }
    if horizontal {
        window.set_height_request(cfg.bar.thickness);
    } else {
        window.set_width_request(cfg.bar.thickness);
    }
    window.auto_exclusive_zone_enable();

    // Root box lays panels along the bar's main axis.
    let root = GtkBox::new(
        if horizontal {
            Orientation::Horizontal
        } else {
            Orientation::Vertical
        },
        4,
    );
    root.add_css_class("bar");

    let mut panels: Vec<Panel> = Vec::new();
    for (i, pcfg) in cfg.panel.iter().enumerate() {
        if let Some(panel) = panels::build(pcfg, cfg.bar.smooth, &cfg.actions) {
            if i > 0 {
                let rule = GtkBox::new(Orientation::Horizontal, 0);
                rule.add_css_class("rule");
                root.append(&rule);
            }
            root.append(&panel.root);
            panels.push(panel);
        }
    }
    window.set_child(Some(&root));

    // Prime once so the bar isn't blank before the first tick.
    for p in &panels {
        (p.update)();
    }

    // Single central scheduler: a 250 ms base tick updates each panel when its
    // (possibly fractional) interval has elapsed, stretched by the battery
    // multiplier on battery.
    let panels = Rc::new(panels);
    let mult = cfg.power.battery_interval_multiplier.max(1.0);
    let last = Rc::new(RefCell::new(vec![Instant::now(); panels.len()]));
    glib::timeout_add_local(Duration::from_millis(250), move || {
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

    window.present();
}
