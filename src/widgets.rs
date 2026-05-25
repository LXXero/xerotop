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

/// Trace a rounded-rectangle path (radius clamped so it can't exceed a pill).
fn rounded_rect(cr: &gtk::cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    use std::f64::consts::{FRAC_PI_2, PI};
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -FRAC_PI_2, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, FRAC_PI_2);
    cr.arc(x + r, y + h - r, r, FRAC_PI_2, PI);
    cr.arc(x + r, y + r, r, PI, PI + FRAC_PI_2);
    cr.close_path();
}

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
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        w: i32,
        h: i32,
        fixed_max: Option<f64>,
        gamma: f64,
        specs: &[(Rgba, bool)],
        interval_s: f64,
        smooth: bool,
        // Scale to the recent min..max instead of 0..max — amplifies small swings
        // on values that never go near zero (temps). Ignores fixed_max.
        autobase: bool,
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
            // Vertical scale: autobase maps recent min..max across the height;
            // otherwise 0..(fixed_max or recent peak).
            let (lo, hi) = if autobase {
                let mut lo = f64::INFINITY;
                let mut hi = f64::NEG_INFINITY;
                for v in ss.iter().flat_map(|se| se.buf.iter().copied()) {
                    lo = lo.min(v);
                    hi = hi.max(v);
                }
                (lo, hi)
            } else {
                let hi = match fixed_max {
                    Some(m) => m,
                    None => {
                        // Autoscale to the recent peak, with 15% headroom so a
                        // sustained peak (e.g. GPU pinned high) doesn't clip flat
                        // against the very top of the chart.
                        let peak = ss
                            .iter()
                            .flat_map(|se| se.buf.iter().copied())
                            .fold(1e-9, f64::max);
                        peak * 1.15
                    }
                };
                (0.0, hi)
            };
            let span = hi - lo;
            let yof = |v: f64| {
                if span < 1e-6 {
                    h / 2.0 // flat (constant) → midline rather than collapsed
                } else {
                    h - ((v - lo) / span).clamp(0.0, 1.0).powf(gamma) * h
                }
            };
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
            // Rounded "thermometer" track + fill (pill-capped ends, ewwii style).
            rounded_rect(cr, 0.0, 0.0, w, h, h / 2.0);
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.10);
            let _ = cr.fill();
            let frac = (v.get() / max).clamp(0.0, 1.0);
            let fw = w * frac;
            if fw > 0.5 {
                rounded_rect(cr, 0.0, 0.0, fw, h, h / 2.0);
                cr.set_source_rgba(r, g, b, a);
                let _ = cr.fill();
            }
        });

        Bar { area, value }
    }

    pub fn set(&self, v: f64) {
        self.value.set(v);
        self.area.queue_draw();
    }
}
