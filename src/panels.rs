//! Panels are self-contained meters. Each builder returns a `Panel`: its root
//! widget, a base update interval (seconds), and an `update` closure the central
//! scheduler calls when the panel is due. No per-panel timers.

use crate::config::PanelConfig;
use crate::metrics::{Cpu, Net, battery, cpu_temp, mem_percent};
use crate::widgets::Graph;
use gtk::prelude::*;
use gtk::{Box as GtkBox, Label, Orientation};
use std::cell::RefCell;
use std::rc::Rc;

pub struct Panel {
    pub root: gtk::Widget,
    pub interval: u32,
    pub update: Box<dyn Fn()>,
}

const GRAPH_W: i32 = 134;
const GRAPH_H: i32 = 28;
const GREEN: (f64, f64, f64, f64) = (0.40, 1.0, 0.40, 0.9);
const CYAN: (f64, f64, f64, f64) = (0.40, 0.8, 1.0, 0.9);
const AMBER: (f64, f64, f64, f64) = (1.0, 0.75, 0.30, 0.9);
const RED: (f64, f64, f64, f64) = (1.0, 0.45, 0.40, 0.9);

/// Build a panel from its config, or None for an unknown type.
pub fn build(cfg: &PanelConfig) -> Option<Panel> {
    let interval = cfg.interval.max(1);
    match cfg.kind.as_str() {
        "clock" => Some(clock_panel(interval)),
        "cpu" => Some(metric_panel("CPU", interval, cfg.graph, GREEN, {
            let cpu = Rc::new(RefCell::new(Cpu::new()));
            move || {
                let p = cpu.borrow_mut().sample();
                (format!("{p:.0}%"), p)
            }
        })),
        "mem" => Some(metric_panel("MEM", interval, cfg.graph, CYAN, || {
            let p = mem_percent();
            (format!("{p:.0}%"), p)
        })),
        "temp" => Some(metric_panel("TEMP", interval, cfg.graph, RED, || {
            let t = cpu_temp();
            (format!("{t:.0}\u{00b0}"), t)
        })),
        "net" => Some(net_panel(interval, cfg.graph)),
        "bat" | "battery" => Some(battery_panel(interval, cfg.graph)),
        other => {
            eprintln!("xerotop: unknown panel type '{other}', skipping");
            None
        }
    }
}

fn header(name: &str) -> (GtkBox, Label) {
    let row = GtkBox::new(Orientation::Horizontal, 2);
    let lbl = Label::new(Some(name));
    lbl.add_css_class("label");
    lbl.set_xalign(0.0);
    lbl.set_hexpand(true);
    let val = Label::new(Some("--"));
    val.add_css_class("value");
    val.set_xalign(1.0);
    row.append(&lbl);
    row.append(&val);
    (row, val)
}

/// Generic 0..100 metric panel (cpu, mem): a value label + optional graph,
/// driven by a sampler returning (display_string, percent).
fn metric_panel<F>(
    name: &str,
    interval: u32,
    graph: bool,
    rgba: (f64, f64, f64, f64),
    sampler: F,
) -> Panel
where
    F: Fn() -> (String, f64) + 'static,
{
    let root = GtkBox::new(Orientation::Vertical, 2);
    root.add_css_class("panel");
    let (row, val) = header(name);
    root.append(&row);

    let graph_widget = if graph {
        let g = Graph::new(60, GRAPH_W, GRAPH_H, Some(100.0), rgba);
        root.append(&g.area);
        Some(g)
    } else {
        None
    };

    let update = Box::new(move || {
        let (text, pct) = sampler();
        val.set_text(&text);
        if let Some(g) = &graph_widget {
            g.push(pct);
        }
    });

    Panel {
        root: root.upcast(),
        interval,
        update,
    }
}

fn net_panel(interval: u32, graph: bool) -> Panel {
    let root = GtkBox::new(Orientation::Vertical, 2);
    root.add_css_class("panel");
    let (row, val) = header("NET");
    root.append(&row);

    let graph_widget = if graph {
        let g = Graph::new(60, GRAPH_W, GRAPH_H, None, AMBER); // autoscale
        root.append(&g.area);
        Some(g)
    } else {
        None
    };

    let net = Rc::new(RefCell::new(Net::new()));
    let update = Box::new(move || {
        let (down, up) = net.borrow_mut().sample();
        val.set_text(&format!("\u{2193}{down:.0} \u{2191}{up:.0}"));
        if let Some(g) = &graph_widget {
            g.push(down);
        }
    });

    Panel {
        root: root.upcast(),
        interval,
        update,
    }
}

fn battery_panel(interval: u32, graph: bool) -> Panel {
    let root = GtkBox::new(Orientation::Vertical, 2);
    root.add_css_class("panel");
    let (row, val) = header("BAT");
    root.append(&row);

    let graph_widget = if graph {
        let g = Graph::new(60, GRAPH_W, GRAPH_H, Some(100.0), GREEN);
        root.append(&g.area);
        Some(g)
    } else {
        None
    };

    let update = Box::new(move || match battery() {
        Some((pct, status)) => {
            let sym = match status.as_str() {
                "Charging" => " \u{2191}",
                "Discharging" => " \u{2193}",
                _ => "",
            };
            val.set_text(&format!("{pct:.0}%{sym}"));
            if let Some(g) = &graph_widget {
                g.push(pct);
            }
        }
        None => {
            val.set_text("AC");
            if let Some(g) = &graph_widget {
                g.push(0.0);
            }
        }
    });

    Panel {
        root: root.upcast(),
        interval,
        update,
    }
}

fn clock_panel(interval: u32) -> Panel {
    let root = GtkBox::new(Orientation::Vertical, 0);
    root.add_css_class("panel");
    root.add_css_class("clock");

    let time = Label::new(Some("--:--"));
    time.add_css_class("clock-time");
    let date = Label::new(Some(""));
    date.add_css_class("clock-date");
    root.append(&time);
    root.append(&date);

    let update = Box::new(move || {
        if let Ok(now) = gtk::glib::DateTime::now_local() {
            if let Ok(t) = now.format("%H:%M") {
                time.set_text(&t);
            }
            if let Ok(d) = now.format("%a %d %b") {
                date.set_text(&d);
            }
        }
    });

    Panel {
        root: root.upcast(),
        interval,
        update,
    }
}
