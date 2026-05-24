//! Panels are self-contained meters. Each builder returns a `Panel`: its root
//! widget, a base update interval (seconds), and an `update` closure the central
//! scheduler calls when the panel is due. No per-panel timers.

use crate::config::{Actions, PanelConfig};
use crate::metrics::{
    Cpu, Disk, Net, add_brightness, add_volume, battery, brightness, cpu_temp, disk_usage, gpu,
    mem_detail, toggle_mute, volume,
};
use crate::widgets::{Bar, Graph, Rgba};
use gtk::prelude::*;
use gtk::{Box as GtkBox, Label, Orientation};
use std::cell::RefCell;
use std::rc::Rc;

pub struct Panel {
    pub root: gtk::Widget,
    pub interval: f64,
    pub update: Box<dyn Fn()>,
}

const GRAPH_W: i32 = 134;
const GRAPH_H: i32 = 24;
const MINI_H: i32 = 14;
const BAR_H: i32 = 10;

const GREEN: Rgba = (0.40, 1.0, 0.40, 0.9);
const CYAN: Rgba = (0.40, 0.8, 1.0, 0.9);
const AMBER: Rgba = (1.0, 0.75, 0.30, 0.9);
const RED: Rgba = (1.0, 0.45, 0.40, 0.9);
const VIOLET: Rgba = (0.78, 0.55, 1.0, 0.9);
const PALE: Rgba = (0.70, 0.92, 1.0, 0.85); // MEM cache overlay line

/// Build a panel from its config, or None for an unknown type.
pub fn build(cfg: &PanelConfig, smooth: bool, actions: &Actions) -> Option<Panel> {
    let iv = cfg.interval.max(0.1);
    let clock_fmts = || {
        (
            cfg.time_format.clone().unwrap_or_else(|| "%I:%M %p".into()),
            cfg.date_format.clone().unwrap_or_else(|| "%a %d %b".into()),
        )
    };
    match cfg.kind.as_str() {
        "header" => {
            let (tf, df) = clock_fmts();
            Some(header_panel(iv, tf, df, actions))
        }
        "clock" => {
            let (tf, df) = clock_fmts();
            Some(clock_panel(iv, tf, df))
        }
        "cpu" => Some(metric_panel("CPU", iv, cfg.graph, GREEN, smooth, None, {
            let cpu = Rc::new(RefCell::new(Cpu::new()));
            move || {
                let p = cpu.borrow_mut().sample();
                (format!("{p:.0}%"), p)
            }
        })),
        "mem" => Some(mem_panel(iv, cfg.graph, smooth)),
        "temp" => Some(metric_panel(
            "TEMP",
            iv,
            cfg.graph,
            RED,
            smooth,
            Some(100.0),
            || {
                let t = cpu_temp();
                (format!("{t:.0}\u{00b0}"), t)
            },
        )),
        "gpu" => Some(gpu_panel(iv, cfg.graph, smooth)),
        "disk" => Some(disk_panel(iv, cfg.graph, smooth)),
        "net" => Some(net_panel(iv, cfg.graph, smooth)),
        "bat" | "battery" => Some(bar_panel(
            iv,
            RED,
            || match battery() {
                Some((p, status)) => {
                    let icon = if status == "Charging" {
                        "\u{f0e7}" // charging bolt
                    } else if p >= 90.0 {
                        "\u{f240}" // full
                    } else if p >= 70.0 {
                        "\u{f241}" // 3/4
                    } else if p >= 40.0 {
                        "\u{f242}" // half
                    } else if p >= 15.0 {
                        "\u{f243}" // 1/4
                    } else {
                        "\u{f244}" // empty
                    };
                    (icon.to_string(), format!("{p:.0}%"), p)
                }
                None => ("\u{f1e6}".to_string(), "AC".to_string(), 100.0), // plug
            },
            None,
            None,
        )),
        "vol" | "volume" => Some(bar_panel(
            iv,
            GREEN,
            || match volume() {
                Some((p, muted)) => {
                    let glyph = if muted || p <= 0.0 {
                        "\u{f026}" // muted
                    } else if p < 50.0 {
                        "\u{f027}" // low
                    } else {
                        "\u{f028}" // high
                    };
                    let icon = if muted {
                        format!("<span foreground='#ff6666'>{glyph}</span>")
                    } else {
                        glyph.to_string()
                    };
                    (icon, format!("{p:.0}%"), p)
                }
                None => ("\u{f028}".to_string(), "--".to_string(), 0.0),
            },
            Some(Box::new(|d| add_volume(d * 5.0))),
            Some(Box::new(toggle_mute)),
        )),
        "bri" | "brightness" => Some(bar_panel(
            iv,
            AMBER,
            || match brightness() {
                Some(p) => ("\u{f185}".to_string(), format!("{p:.0}%"), p), // sun
                None => ("\u{f185}".to_string(), "--".to_string(), 0.0),
            },
            Some(Box::new(|d| add_brightness(d * 5.0))),
            None,
        )),
        other => {
            eprintln!("xerotop: unknown panel type '{other}', skipping");
            None
        }
    }
}

fn panel_box() -> GtkBox {
    let b = GtkBox::new(Orientation::Vertical, 2);
    b.add_css_class("panel");
    b
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

fn sub() -> Label {
    let l = Label::new(Some(""));
    l.add_css_class("sub");
    l.set_xalign(0.0);
    l
}

fn graph_widget(
    root: &GtkBox,
    h: i32,
    specs: &[(Rgba, bool)],
    fixed: Option<f64>,
    iv: f64,
    smooth: bool,
    graph: bool,
) -> Option<Graph> {
    graph.then(|| {
        // Autoscaled graphs (cpu/gpu/net/disk) use gamma > 1 to deepen valleys
        // and sharpen peaks (ewwii-like spikiness); fixed-scale meters (mem/temp)
        // stay linear so the fill reflects the true level.
        let gamma = if fixed.is_some() { 1.0 } else { 1.6 };
        let g = Graph::new(GRAPH_W, h, fixed, gamma, specs, iv, smooth);
        root.append(&g.area);
        g
    })
}

/// 0..100 single-series filled metric (cpu, temp).
fn metric_panel<F>(
    name: &str,
    interval: f64,
    graph: bool,
    rgba: Rgba,
    smooth: bool,
    fixed: Option<f64>,
    sampler: F,
) -> Panel
where
    F: Fn() -> (String, f64) + 'static,
{
    let root = panel_box();
    let (row, val) = header(name);
    root.append(&row);
    let g = graph_widget(
        &root,
        GRAPH_H,
        &[(rgba, true)],
        fixed,
        interval,
        smooth,
        graph,
    );
    let update = Box::new(move || {
        let (text, pct) = sampler();
        val.set_text(&text);
        if let Some(g) = &g {
            g.push(&[pct]);
        }
    });
    Panel {
        root: root.upcast(),
        interval,
        update,
    }
}

/// MEM: filled "used" plus an overlay line at used+cache (so cache/buffers shows
/// as the band between fill and line).
fn mem_panel(interval: f64, graph: bool, smooth: bool) -> Panel {
    let root = panel_box();
    let (row, val) = header("MEM");
    root.append(&row);
    let g = graph_widget(
        &root,
        GRAPH_H,
        &[(CYAN, true), (PALE, false)],
        Some(100.0),
        interval,
        smooth,
        graph,
    );
    let update = Box::new(move || {
        let (used, cache) = mem_detail();
        val.set_text(&format!("{used:.0}%"));
        if let Some(g) = &g {
            g.push(&[used, used + cache]);
        }
    });
    Panel {
        root: root.upcast(),
        interval,
        update,
    }
}

/// Slim single-row meter: icon · thin inline bar · value. The icon and value
/// come from the sampler `(icon, value, pct)`. The value label is fixed-width
/// (room for "100%") and the bar hexpands, so bars stay the same length and the
/// % column never shifts. Optional scroll/click control (volume, brightness).
fn bar_panel<F>(
    interval: f64,
    rgba: Rgba,
    sampler: F,
    on_scroll: Option<Box<dyn Fn(f64)>>,
    on_click: Option<Box<dyn Fn()>>,
) -> Panel
where
    F: Fn() -> (String, String, f64) + 'static,
{
    let row = GtkBox::new(Orientation::Horizontal, 4);
    row.add_css_class("panel");
    row.add_css_class("meter");
    let icon = Label::new(None);
    icon.add_css_class("meter-icon");
    icon.set_width_chars(2);
    icon.set_xalign(0.5);
    let bar = Bar::new(-1, BAR_H, 100.0, rgba);
    bar.area.set_hexpand(true);
    bar.area.set_valign(gtk::Align::Center);
    let val = Label::new(Some("--"));
    val.add_css_class("value");
    val.set_width_chars(4); // fixed column: "100%" fits, bars stay equal length
    val.set_xalign(1.0);
    row.append(&icon);
    row.append(&bar.area);
    row.append(&val);

    let refresh: Rc<dyn Fn()> = Rc::new({
        let icon = icon.clone();
        let val = val.clone();
        let bar = bar.clone();
        move || {
            let (ic, text, pct) = sampler();
            icon.set_markup(&ic); // markup so samplers can color (e.g. red mute)
            val.set_text(&text);
            bar.set(pct);
        }
    });

    if let Some(scroll) = on_scroll {
        let ec = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        let refresh = refresh.clone();
        ec.connect_scroll(move |_, _, dy| {
            scroll(if dy < 0.0 { 1.0 } else { -1.0 });
            refresh();
            gtk::glib::Propagation::Stop
        });
        row.add_controller(ec);
    }
    if let Some(click) = on_click {
        let gc = gtk::GestureClick::new();
        let refresh = refresh.clone();
        gc.connect_released(move |_, _, _, _| {
            click();
            refresh();
        });
        row.add_controller(gc);
    }

    let upd = refresh.clone();
    let update = Box::new(move || upd());
    Panel {
        root: row.upcast(),
        interval,
        update,
    }
}

fn net_panel(interval: f64, graph: bool, smooth: bool) -> Panel {
    let root = panel_box();
    let (row, val) = header("NET");
    root.append(&row);
    let dn = graph_widget(
        &root,
        MINI_H,
        &[(CYAN, true)],
        None,
        interval,
        smooth,
        graph,
    );
    let up = graph_widget(
        &root,
        MINI_H,
        &[(AMBER, true)],
        None,
        interval,
        smooth,
        graph,
    );
    let net = Rc::new(RefCell::new(Net::new()));
    let update = Box::new(move || {
        let (down, upl) = net.borrow_mut().sample();
        val.set_text(&format!("\u{2193}{down:.0} \u{2191}{upl:.0}"));
        if let Some(g) = &dn {
            g.push(&[down]);
        }
        if let Some(g) = &up {
            g.push(&[upl]);
        }
    });
    Panel {
        root: root.upcast(),
        interval,
        update,
    }
}

fn gpu_panel(interval: f64, graph: bool, smooth: bool) -> Panel {
    let root = panel_box();
    let (row, val) = header("GPU");
    root.append(&row);
    let g = graph_widget(
        &root,
        GRAPH_H,
        &[(VIOLET, true)],
        None,
        interval,
        smooth,
        graph,
    );
    let vram = sub();
    root.append(&vram);
    let update = Box::new(move || match gpu() {
        Some((busy, used, total)) => {
            val.set_text(&format!("{busy:.0}%"));
            vram.set_text(&format!("{used:.1}G/{total:.1}G"));
            if let Some(g) = &g {
                g.push(&[busy]);
            }
        }
        None => val.set_text("--"),
    });
    Panel {
        root: root.upcast(),
        interval,
        update,
    }
}

fn disk_panel(interval: f64, graph: bool, smooth: bool) -> Panel {
    let root = panel_box();
    let (row, val) = header("DISK");
    root.append(&row);
    let usage = sub();
    root.append(&usage);
    let rd = graph_widget(
        &root,
        MINI_H,
        &[(CYAN, true)],
        None,
        interval,
        smooth,
        graph,
    );
    let wr = graph_widget(
        &root,
        MINI_H,
        &[(AMBER, true)],
        None,
        interval,
        smooth,
        graph,
    );
    let disk = Rc::new(RefCell::new(Disk::new()));
    let update = Box::new(move || {
        if let Some((pct, used, total)) = disk_usage() {
            val.set_text(&format!("{pct:.0}%"));
            usage.set_text(&format!("{used:.0}G/{total:.0}G"));
        }
        let (r, w) = disk.borrow_mut().sample();
        if let Some(g) = &rd {
            g.push(&[r]);
        }
        if let Some(g) = &wr {
            g.push(&[w]);
        }
    });
    Panel {
        root: root.upcast(),
        interval,
        update,
    }
}

fn spawn(cmd: &str) {
    let _ = std::process::Command::new("sh").arg("-c").arg(cmd).spawn();
}

/// Header: power menu (left) · clock (center) · lock (right), with date below.
fn header_panel(interval: f64, time_fmt: String, date_fmt: String, actions: &Actions) -> Panel {
    let root = panel_box();
    root.add_css_class("clock");

    let top = gtk::CenterBox::new();

    // power menu button → popover with logout/reboot/shutdown
    let power = gtk::Button::new();
    power.set_label("\u{f011}"); // power-off glyph
    power.add_css_class("hbtn");
    let pop = gtk::Popover::new();
    let menu = GtkBox::new(Orientation::Vertical, 2);
    menu.add_css_class("menu");
    for (label, cmd) in [
        ("Logout", actions.logout.clone()),
        ("Reboot", actions.reboot.clone()),
        ("Shutdown", actions.shutdown.clone()),
    ] {
        let item = gtk::Button::with_label(label);
        item.add_css_class("menu-item");
        let pop = pop.clone();
        item.connect_clicked(move |_| {
            spawn(&cmd);
            pop.popdown();
        });
        menu.append(&item);
    }
    pop.set_child(Some(&menu));
    pop.set_parent(&power);
    pop.set_position(gtk::PositionType::Bottom);
    let pop_open = pop.clone();
    power.connect_clicked(move |_| pop_open.popup());

    // lock button
    let lock = gtk::Button::new();
    lock.set_label("\u{f023}"); // lock glyph
    lock.add_css_class("hbtn");
    let lock_cmd = actions.lock.clone();
    lock.connect_clicked(move |_| spawn(&lock_cmd));

    let time = Label::new(Some("--:--"));
    time.add_css_class("clock-time");

    top.set_start_widget(Some(&power));
    top.set_center_widget(Some(&time));
    top.set_end_widget(Some(&lock));

    let date = Label::new(Some(""));
    date.add_css_class("clock-date");
    date.set_halign(gtk::Align::Center);

    root.append(&top);
    root.append(&date);

    let update = Box::new(move || {
        if let Ok(now) = gtk::glib::DateTime::now_local() {
            if let Ok(t) = now.format(&time_fmt) {
                time.set_text(t.trim_start());
            }
            if let Ok(d) = now.format(&date_fmt) {
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

fn clock_panel(interval: f64, time_fmt: String, date_fmt: String) -> Panel {
    let root = panel_box();
    root.add_css_class("clock");
    let time = Label::new(Some("--:--"));
    time.add_css_class("clock-time");
    let date = Label::new(Some(""));
    date.add_css_class("clock-date");
    root.append(&time);
    root.append(&date);
    let update = Box::new(move || {
        if let Ok(now) = gtk::glib::DateTime::now_local() {
            if let Ok(t) = now.format(&time_fmt) {
                time.set_text(t.trim_start());
            }
            if let Ok(d) = now.format(&date_fmt) {
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
