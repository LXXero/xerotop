//! Panels are self-contained meters. Each builder returns a `Panel`: its root
//! widget, a base update interval (seconds), and an `update` closure the central
//! scheduler calls when the panel is due. No per-panel timers.

use crate::config::{Actions, PanelConfig};
use crate::metrics::{
    Cpu, Disk, Net, Top, add_brightness, add_volume, battery, brightness, disk_usage, fan_rpm, gpu,
    mem_detail, temp_cpu, temp_gpu, temp_ssd, toggle_mute, volume,
};
use crate::widgets::{Bar, Graph, Rgba};
use gtk::prelude::*;
use gtk::{Box as GtkBox, Label, Orientation};
use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
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

/// Gamma for autoscaled graphs (cpu/gpu/net/disk), set once from config.
static AUTOSCALE_GAMMA: std::sync::OnceLock<f64> = std::sync::OnceLock::new();

/// Set the autoscaled-graph gamma. Call before building panels.
pub fn set_gamma(g: f64) {
    let _ = AUTOSCALE_GAMMA.set(g);
}

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
        "temp" => Some(temp_panel(iv, cfg.graph, smooth)),
        "top" => Some(top_panel(iv)),
        "gpu" => Some(gpu_panel(iv, cfg.graph, smooth)),
        "disk" => Some(disk_panel(iv, cfg.graph, smooth)),
        "net" => Some(net_panel(iv, cfg.graph, smooth)),
        "win" | "taskbar" => Some(taskbar_panel()),
        "tray" => Some(tray_panel()),
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
        let gamma = if fixed.is_some() {
            1.0
        } else {
            *AUTOSCALE_GAMMA.get().unwrap_or(&1.4)
        };
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
    let bar = Bar::new(0, BAR_H, 100.0, rgba);
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

/// Resolve a window's app_id to an icon: try the icon theme, then match a
/// .desktop file's `Icon=` (like ewwii's app_icon script). Cached per app_id.
fn resolve_icon(app_id: &str) -> Option<gtk::Image> {
    if app_id.is_empty() {
        return None;
    }
    thread_local! {
        static CACHE: RefCell<HashMap<String, Option<String>>> = RefCell::new(HashMap::new());
    }
    let name = CACHE.with(|c| {
        if let Some(v) = c.borrow().get(app_id) {
            return v.clone();
        }
        let resolved = compute_icon_name(app_id);
        c.borrow_mut().insert(app_id.to_string(), resolved.clone());
        resolved
    })?;
    if name.starts_with('/') {
        Some(gtk::Image::from_file(name))
    } else {
        Some(gtk::Image::from_icon_name(&name))
    }
}

fn compute_icon_name(app_id: &str) -> Option<String> {
    let lower = app_id.to_lowercase();
    let tail = app_id.rsplit('.').next().unwrap_or(app_id).to_lowercase();
    if let Some(display) = gtk::gdk::Display::default() {
        let theme = gtk::IconTheme::for_display(&display);
        for cand in [app_id, &lower, &tail] {
            if theme.has_icon(cand) {
                return Some(cand.to_string());
            }
        }
    }
    desktop_icon(&lower, &tail)
}

fn desktop_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(home) = std::env::var("HOME") {
        dirs.push(PathBuf::from(home).join(".local/share/applications"));
    }
    let xdg =
        std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
    for d in xdg.split(':') {
        dirs.push(PathBuf::from(d).join("applications"));
    }
    dirs
}

/// Find a .desktop file matching `want`/`tail` and return its Icon=. Exact
/// matches (filename stem / StartupWMClass) first, then fuzzy, to avoid
/// grabbing the wrong handler (see ewwii ONBOARDING gotcha #15).
fn desktop_icon(want: &str, tail: &str) -> Option<String> {
    let dirs = desktop_dirs();
    for fuzzy in [false, true] {
        for dir in &dirs {
            let Ok(rd) = fs::read_dir(dir) else {
                continue;
            };
            for entry in rd.flatten() {
                let p = entry.path();
                if p.extension().and_then(|x| x.to_str()) != Some("desktop") {
                    continue;
                }
                let stem = p
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_lowercase();
                let Ok(content) = fs::read_to_string(&p) else {
                    continue;
                };
                let mut icon = None;
                let mut wmclass = None;
                for line in content.lines() {
                    if let Some(v) = line.strip_prefix("Icon=") {
                        if icon.is_none() {
                            icon = Some(v.trim().to_string());
                        }
                    } else if let Some(v) = line.strip_prefix("StartupWMClass=") {
                        wmclass = Some(v.trim().to_lowercase());
                    }
                }
                let hit = if fuzzy {
                    stem.contains(want) || stem.contains(tail)
                } else {
                    stem == want
                        || stem == tail
                        || wmclass.as_deref() == Some(want)
                        || wmclass.as_deref() == Some(tail)
                };
                if hit && let Some(i) = icon {
                    return Some(i);
                }
            }
        }
    }
    None
}

/// Build a tray icon image from a StatusNotifier item (themed name, else pixmap).
fn tray_image(it: &crate::tray::TrayItem) -> gtk::Image {
    let img = gtk::Image::new();
    img.set_pixel_size(18);
    if let (Some(name), Some(display)) = (&it.icon_name, gtk::gdk::Display::default()) {
        let theme = gtk::IconTheme::for_display(&display);
        if let Some(path) = &it.icon_theme_path {
            theme.add_search_path(path);
        }
        if theme.has_icon(name) {
            img.set_icon_name(Some(name));
            return img;
        }
    }
    if let Some((w, h, argb)) = &it.pixmap
        && let Some(tex) = pixmap_texture(*w, *h, argb)
    {
        img.set_paintable(Some(&tex));
        return img;
    }
    img.set_icon_name(Some("application-x-executable"));
    img
}

/// SNI pixmaps are ARGB32 (network byte order: bytes A,R,G,B).
fn pixmap_texture(w: i32, h: i32, argb: &[u8]) -> Option<gtk::gdk::Texture> {
    if w <= 0 || h <= 0 || argb.len() < (w * h * 4) as usize {
        return None;
    }
    let bytes = gtk::glib::Bytes::from(argb);
    let tex = gtk::gdk::MemoryTexture::new(
        w,
        h,
        gtk::gdk::MemoryFormat::A8r8g8b8,
        &bytes,
        (w * 4) as usize,
    );
    Some(tex.upcast())
}

/// Build a custom cascading menu Popover from the DBus menu tree. Plain widgets
/// with direct click handlers (no GAction muxer), left-aligned, with submenus
/// that open on hover (and click). `dismiss` closes the whole chain (clicking a
/// leaf or the root auto-hiding cascades down).
#[allow(clippy::too_many_arguments)]
fn make_menu_popover(
    entries: &[crate::tray::MenuEntry],
    atx: &Rc<async_channel::Sender<crate::tray::TrayAction>>,
    addr: &str,
    mp: &str,
    dismiss: &Rc<dyn Fn()>,
    autohide: bool,
    position: gtk::PositionType,
) -> gtk::Popover {
    let pop = gtk::Popover::new();
    pop.set_autohide(autohide);
    pop.set_has_arrow(false);
    pop.set_position(position);
    pop.add_css_class("ctx-menu");
    let vbox = GtkBox::new(Orientation::Vertical, 0);
    vbox.add_css_class("menu");

    // child submenu open at this level (so hovering a sibling closes it)
    let cur_child: Rc<RefCell<Option<gtk::Popover>>> = Rc::new(RefCell::new(None));

    for e in entries {
        if e.separator {
            let rule = GtkBox::new(Orientation::Horizontal, 0);
            rule.add_css_class("rule");
            rule.set_size_request(-1, 1);
            vbox.append(&rule);
            continue;
        }
        let row = gtk::Button::new();
        row.add_css_class("menu-item");
        row.set_sensitive(e.enabled);
        let rb = GtkBox::new(Orientation::Horizontal, 8);
        let lbl = Label::new(Some(&e.label));
        lbl.set_xalign(0.0);
        lbl.set_hexpand(true);
        rb.append(&lbl);
        if !e.children.is_empty() {
            let arrow = Label::new(Some("\u{203a}")); // ›
            rb.append(&arrow);
        }
        row.set_child(Some(&rb));

        if e.children.is_empty() {
            let atx = atx.clone();
            let addr = addr.to_string();
            let mp = mp.to_string();
            let id = e.id;
            let dismiss2 = dismiss.clone();
            row.connect_clicked(move |_| {
                let _ = atx.try_send(crate::tray::TrayAction::MenuClick(
                    addr.clone(),
                    mp.clone(),
                    id,
                ));
                dismiss2();
            });
            // hovering a leaf closes any open sibling submenu
            let cur = cur_child.clone();
            let motion = gtk::EventControllerMotion::new();
            motion.connect_enter(move |_, _, _| {
                if let Some(old) = cur.borrow_mut().take() {
                    old.popdown();
                }
            });
            row.add_controller(motion);
        } else {
            let children = e.children.clone();
            let atx_c = atx.clone();
            let addr_c = addr.to_string();
            let mp_c = mp.to_string();
            let dismiss_c = dismiss.clone();
            let cur = cur_child.clone();
            let row_weak = row.downgrade();
            let sub_id = e.id;
            let open_child = move || {
                if cur.borrow().is_some() {
                    return; // already open
                }
                let Some(rw) = row_weak.upgrade() else {
                    return;
                };
                // tell the app this submenu is opening so it honors its items
                let _ = atx_c.try_send(crate::tray::TrayAction::AboutToShow(
                    addr_c.clone(),
                    mp_c.clone(),
                    sub_id,
                ));
                let child = make_menu_popover(
                    &children,
                    &atx_c,
                    &addr_c,
                    &mp_c,
                    &dismiss_c,
                    false,
                    gtk::PositionType::Right,
                );
                // Parent to the submenu row (inside this popover) so GTK treats
                // it as a nested sub-popover — the root won't autohide and eat
                // clicks made inside the child.
                child.set_parent(&rw);
                child.popup();
                cur.replace(Some(child));
            };
            let on_enter = open_child.clone();
            let motion = gtk::EventControllerMotion::new();
            motion.connect_enter(move |_, _, _| on_enter());
            row.add_controller(motion);
            row.connect_clicked(move |_| open_child());
        }
        vbox.append(&row);
    }

    pop.set_child(Some(&vbox));
    let cur2 = cur_child.clone();
    pop.connect_closed(move |p| {
        if let Some(c) = cur2.borrow_mut().take() {
            c.popdown(); // cascade-close descendants
        }
        p.unparent();
    });
    pop
}

/// System tray: StatusNotifier items as clickable icons (left-click activates).
fn tray_panel() -> Panel {
    let root = panel_box();
    root.add_css_class("tray");
    let flow = gtk::FlowBox::new();
    flow.set_selection_mode(gtk::SelectionMode::None);
    flow.set_min_children_per_line(1);
    flow.set_max_children_per_line(8);
    flow.set_homogeneous(false);
    root.append(&flow);

    let (rx, atx) = crate::tray::spawn();
    let atx = Rc::new(atx);
    let flow2 = flow.clone();
    let root2 = root.clone(); // stable parent for popovers (survives rebuilds)
    gtk::glib::spawn_future_local(async move {
        while let Ok(items) = rx.recv().await {
            while let Some(child) = flow2.first_child() {
                flow2.remove(&child);
            }
            for it in items {
                let btn = gtk::Button::new();
                btn.add_css_class("tray-item");
                btn.set_has_frame(false);
                btn.set_tooltip_text(Some(&it.title));
                btn.set_child(Some(&tray_image(&it)));

                // left-click: activate
                {
                    let atx = atx.clone();
                    let id = it.id.clone();
                    btn.connect_clicked(move |_| {
                        let _ = atx.try_send(crate::tray::TrayAction::Activate(id.clone()));
                    });
                }

                // right-click: custom cascading menu (direct handlers + hover)
                if it.menu_path.is_some() && !it.menu.is_empty() {
                    let atx = atx.clone();
                    let addr = it.id.clone();
                    let menu_path = it.menu_path.clone().unwrap();
                    let entries = it.menu.clone();
                    let btn_weak = btn.downgrade();
                    let parent = root2.clone();
                    let gesture = gtk::GestureClick::new();
                    gesture.set_button(3);
                    gesture.connect_pressed(move |_, _, _, _| {
                        let Some(anchor) = btn_weak.upgrade() else {
                            return;
                        };
                        // ask the app to refresh its menu (DBusMenu AboutToShow)
                        let _ = atx.try_send(crate::tray::TrayAction::AboutToShow(
                            addr.clone(),
                            menu_path.clone(),
                            0,
                        ));
                        let stable: gtk::Widget = parent.clone().upcast();
                        // dismiss() closes the root, whose closed handler cascades.
                        let root_holder: Rc<RefCell<Option<gtk::Popover>>> =
                            Rc::new(RefCell::new(None));
                        let dismiss: Rc<dyn Fn()> = {
                            let h = root_holder.clone();
                            Rc::new(move || {
                                if let Some(p) = h.borrow().as_ref() {
                                    p.popdown();
                                }
                            })
                        };
                        let pop = make_menu_popover(
                            &entries,
                            &atx,
                            &addr,
                            &menu_path,
                            &dismiss,
                            true,
                            gtk::PositionType::Left,
                        );
                        pop.set_parent(&stable);
                        if let Some(b) = anchor.compute_bounds(&stable) {
                            pop.set_pointing_to(Some(&gtk::gdk::Rectangle::new(
                                b.x() as i32,
                                b.y() as i32,
                                b.width() as i32,
                                b.height() as i32,
                            )));
                        }
                        root_holder.replace(Some(pop.clone()));
                        pop.popup();
                    });
                    btn.add_controller(gesture);
                }

                flow2.insert(&btn, -1);
            }
        }
    });

    Panel {
        root: root.upcast(),
        interval: 3600.0,
        update: Box::new(|| {}),
    }
}

/// Taskbar: open windows via wlr-foreign-toplevel, rebuilt on each snapshot
/// streamed from the wayland listener thread.
fn taskbar_panel() -> Panel {
    let root = panel_box();
    root.add_css_class("taskbar");
    let head = Label::new(Some("WIN"));
    head.add_css_class("label");
    head.set_xalign(0.0);
    root.append(&head);
    let list = GtkBox::new(Orientation::Vertical, 1);
    root.append(&list);

    let (rx, atx) = crate::taskbar::spawn();
    let atx = Rc::new(atx);
    let list2 = list.clone();
    gtk::glib::spawn_future_local(async move {
        while let Ok(tops) = rx.recv().await {
            while let Some(child) = list2.first_child() {
                list2.remove(&child);
            }
            for t in tops {
                let row = GtkBox::new(Orientation::Horizontal, 4);
                if let Some(img) = resolve_icon(&t.app_id) {
                    img.set_pixel_size(16);
                    row.append(&img);
                }
                let text = if t.title.is_empty() {
                    t.app_id.clone()
                } else {
                    t.title.clone()
                };
                let lbl = Label::new(Some(&text));
                lbl.set_xalign(0.0);
                lbl.set_hexpand(true);
                lbl.set_ellipsize(gtk::pango::EllipsizeMode::End);
                lbl.set_max_width_chars(1); // ellipsize within the bar width
                row.append(&lbl);

                let btn = gtk::Button::new();
                btn.add_css_class("task");
                if t.activated {
                    btn.add_css_class("task-active");
                }
                if t.minimized {
                    btn.add_css_class("task-min");
                }
                btn.set_child(Some(&row));
                let id = t.id;
                {
                    let atx = atx.clone();
                    btn.connect_clicked(move |_| {
                        let _ = atx.send(crate::taskbar::Action::Activate(id));
                    });
                }
                {
                    let atx = atx.clone();
                    let right = gtk::GestureClick::new();
                    right.set_button(3); // right-click toggles minimize
                    right.connect_pressed(move |_, _, _, _| {
                        let _ = atx.send(crate::taskbar::Action::ToggleMinimize(id));
                    });
                    btn.add_controller(right);
                }
                list2.append(&btn);
            }
        }
    });

    Panel {
        root: root.upcast(),
        interval: 3600.0,
        update: Box::new(|| {}),
    }
}

type TempSrc = fn() -> Option<f64>;

/// TEMP: multiple sensors (cpu/gpu/ssd) as compact mini line-charts + a fan row.
fn temp_panel(interval: f64, graph: bool, smooth: bool) -> Panel {
    let root = panel_box();
    let head = Label::new(Some("TEMP"));
    head.add_css_class("label");
    head.set_xalign(0.0);
    root.append(&head);

    let sensors: [(&str, TempSrc, Rgba); 3] = [
        ("cpu", temp_cpu, RED),
        ("gpu", temp_gpu, VIOLET),
        ("ssd", temp_ssd, CYAN),
    ];
    let mut rows: Vec<(Option<Graph>, Label, TempSrc)> = Vec::new();
    for (name, src, color) in sensors {
        let row = GtkBox::new(Orientation::Horizontal, 4);
        let lbl = Label::new(Some(name));
        lbl.add_css_class("sub");
        lbl.set_xalign(0.0);
        lbl.set_width_chars(3);
        let g = graph.then(|| {
            let g = Graph::new(0, 12, Some(100.0), 1.0, &[(color, true)], interval, smooth);
            g.area.set_hexpand(true);
            g.area.set_valign(gtk::Align::Center);
            g
        });
        let val = Label::new(Some("--"));
        val.add_css_class("value");
        val.set_xalign(1.0);
        val.set_width_chars(4);
        row.append(&lbl);
        if let Some(g) = &g {
            row.append(&g.area);
        } else {
            let sp = Label::new(None);
            sp.set_hexpand(true);
            row.append(&sp);
        }
        row.append(&val);
        root.append(&row);
        rows.push((g, val, src));
    }

    let fanrow = GtkBox::new(Orientation::Horizontal, 4);
    let fanlbl = Label::new(Some("fan"));
    fanlbl.add_css_class("sub");
    fanlbl.set_xalign(0.0);
    fanlbl.set_hexpand(true);
    let fanval = Label::new(Some("--"));
    fanval.add_css_class("value");
    fanval.set_xalign(1.0);
    fanrow.append(&fanlbl);
    fanrow.append(&fanval);
    root.append(&fanrow);

    let update = Box::new(move || {
        for (g, val, src) in &rows {
            match src() {
                Some(t) => {
                    val.set_text(&format!("{t:.0}\u{00b0}"));
                    if let Some(g) = g {
                        g.push(&[t]);
                    }
                }
                None => val.set_text("--"),
            }
        }
        match fan_rpm() {
            Some(r) => fanval.set_text(&format!("{r:.0}")),
            None => fanval.set_text("--"),
        }
    });
    Panel {
        root: root.upcast(),
        interval,
        update,
    }
}

/// TOP: the busiest processes by instantaneous CPU %.
fn top_panel(interval: f64) -> Panel {
    const N: usize = 5;
    let root = panel_box();
    let head = Label::new(Some("TOP"));
    head.add_css_class("label");
    head.set_xalign(0.0);
    root.append(&head);

    let mut rows: Vec<(Label, Label)> = Vec::new();
    for _ in 0..N {
        let row = GtkBox::new(Orientation::Horizontal, 4);
        let name = Label::new(Some(""));
        name.add_css_class("sub");
        name.set_xalign(0.0);
        name.set_hexpand(true);
        name.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let val = Label::new(Some(""));
        val.add_css_class("value");
        val.set_xalign(1.0);
        row.append(&name);
        row.append(&val);
        root.append(&row);
        rows.push((name, val));
    }

    let top = Rc::new(RefCell::new(Top::new()));
    let update = Box::new(move || {
        let list = top.borrow_mut().sample(N);
        for (i, (name, val)) in rows.iter().enumerate() {
            if let Some((c, p)) = list.get(i) {
                name.set_text(c);
                val.set_text(&format!("{p:.1}"));
            } else {
                name.set_text("");
                val.set_text("");
            }
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
    power.add_css_class("power");
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
    lock.add_css_class("lock");
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
