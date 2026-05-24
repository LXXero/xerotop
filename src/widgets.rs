//! Reusable meters. `Graph` is a multi-series filled history chart with
//! non-linear `gamma` (≈0.5 lifts low values, gkrellm/ewwii feel) and optional
//! smooth horizontal scrolling (continuous "second-hand" motion vs 1 Hz ticks).
//! `Bar` is a single-level fill.
//!
//! Series: the first/each `fill` series draws a translucent area + edge line;
//! non-fill series draw a line only (e.g. MEM's used-vs-cache overlay).

use gtk::DrawingArea;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;
use std::time::Instant;

pub type Rgba = (f64, f64, f64, f64);

const WINDOW_SECS: f64 = 60.0; // graph spans ~60s regardless of sample interval

struct Series {
    buf: VecDeque<f64>,
    rgba: Rgba,
    fill: bool,
}

#[derive(Clone)]
pub struct Graph {
    pub area: DrawingArea,
    series: Rc<RefCell<Vec<Series>>>,
    last: Rc<Cell<Instant>>,
}

impl Graph {
    /// `specs` = one `(color, fill?)` per series. `fixed_max` None = autoscale.
    /// `smooth` adds a per-frame scroll between samples.
    pub fn new(
        w: i32,
        h: i32,
        fixed_max: Option<f64>,
        gamma: f64,
        specs: &[(Rgba, bool)],
        interval_s: f64,
        smooth: bool,
    ) -> Self {
        let area = DrawingArea::new();
        area.add_css_class("graph");
        area.set_content_width(w);
        area.set_content_height(h);

        // Sample count tracks a ~60s window so the graph spans the same wall-clock
        // time at any interval (and fills in ~60s, not 128s at a 2s interval).
        let len = ((WINDOW_SECS / interval_s.max(0.05)).round() as usize).clamp(8, 512);
        let series: Vec<Series> = specs
            .iter()
            .map(|(rgba, fill)| Series {
                buf: VecDeque::from(vec![0.0; len]),
                rgba: *rgba,
                fill: *fill,
            })
            .collect();
        let series = Rc::new(RefCell::new(series));
        let last = Rc::new(Cell::new(Instant::now()));
        let frac = Rc::new(Cell::new(0.0_f64));

        let s = series.clone();
        let fr = frac.clone();
        area.set_draw_func(move |_, cr, w, h| {
            let (w, h) = (w as f64, h as f64);
            let ss = s.borrow();
            let Some(n) = ss.first().map(|se| se.buf.len()) else {
                return;
            };
            if n < 3 {
                return;
            }
            let max = fixed_max
                .unwrap_or_else(|| {
                    ss.iter()
                        .flat_map(|se| se.buf.iter().copied())
                        .fold(1e-9, f64::max)
                })
                .max(1e-9);
            let yof = |v: f64| h - (v / max).clamp(0.0, 1.0).powf(gamma) * h;
            let step = w / (n as f64 - 2.0);
            let f = fr.get();
            let xof = |i: usize| (i as f64 - f) * step;

            for se in ss.iter() {
                let (r, g, b, a) = se.rgba;
                if se.fill {
                    // filled area (closed down to the baseline)
                    cr.move_to(xof(0), h);
                    for i in 0..n {
                        cr.line_to(xof(i), yof(se.buf[i]));
                    }
                    cr.line_to(xof(n - 1), h);
                    cr.close_path();
                    cr.set_source_rgba(r, g, b, a * 0.35);
                    let _ = cr.fill();
                }
                // top curve only — never the closing verticals/baseline, so no
                // stray vertical line scrolls down the right edge.
                cr.move_to(xof(0), yof(se.buf[0]));
                for i in 1..n {
                    cr.line_to(xof(i), yof(se.buf[i]));
                }
                cr.set_source_rgba(r, g, b, a);
                cr.set_line_width(1.0);
                let _ = cr.stroke();
            }
        });

        if smooth {
            let fr2 = frac.clone();
            let last2 = last.clone();
            let iv = interval_s.max(0.1);
            area.add_tick_callback(move |area, _| {
                fr2.set((last2.get().elapsed().as_secs_f64() / iv).clamp(0.0, 1.0));
                area.queue_draw();
                gtk::glib::ControlFlow::Continue
            });
        }

        Graph { area, series, last }
    }

    /// Push one new value per series.
    pub fn push(&self, vals: &[f64]) {
        {
            let mut ss = self.series.borrow_mut();
            for (i, se) in ss.iter_mut().enumerate() {
                let v = vals.get(i).copied().unwrap_or(0.0).max(0.0);
                se.buf.pop_front();
                se.buf.push_back(v);
            }
        }
        self.last.set(Instant::now());
        self.area.queue_draw();
    }
}

#[derive(Clone)]
pub struct Bar {
    pub area: DrawingArea,
    value: Rc<Cell<f64>>,
}

impl Bar {
    pub fn new(w: i32, h: i32, max: f64, rgba: Rgba) -> Self {
        let area = DrawingArea::new();
        area.add_css_class("bar-meter");
        area.set_content_width(w);
        area.set_content_height(h);
        let value = Rc::new(Cell::new(0.0));

        let v = value.clone();
        let (r, g, b, a) = rgba;
        area.set_draw_func(move |_, cr, w, h| {
            let (w, h) = (w as f64, h as f64);
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.10);
            cr.rectangle(0.0, 0.0, w, h);
            let _ = cr.fill();
            let frac = (v.get() / max).clamp(0.0, 1.0);
            cr.set_source_rgba(r, g, b, a);
            cr.rectangle(0.0, 0.0, w * frac, h);
            let _ = cr.fill();
        });

        Bar { area, value }
    }

    pub fn set(&self, v: f64) {
        self.value.set(v);
        self.area.queue_draw();
    }
}
