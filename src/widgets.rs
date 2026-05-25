//! Reusable meters. `Graph` is a multi-series filled history chart with
//! non-linear `gamma` (>1 sharpens peaks, gkrellm/ewwii feel) and optional
//! smooth horizontal scrolling (continuous "second-hand" motion vs 1 Hz ticks).
//! `Bar` is a single-level fill.
//!
//! Series: `fill` series draw a translucent area + edge line; non-fill series
//! draw a line only.

use gtk::DrawingArea;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;
use std::time::Instant;

pub type Rgba = (f64, f64, f64, f64);

#[derive(Clone, Copy)]
#[allow(dead_code)] // Fixed/peak modes are reserved for the upcoming graph config surface.
pub enum GraphScale {
    /// Fixed 0..max scale for meters that should preserve an absolute ceiling.
    Fixed(f64),
    /// Fixed 0 baseline with a recent-window peak.
    DynamicPeak,
    /// Recent min..max range with a little top headroom.
    DynamicRange,
}

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
const RANGE_HEADROOM: f64 = 0.18;

struct Series {
    buf: VecDeque<f64>,
    rgba: Rgba,
    fill: bool,
}

#[derive(Clone)]
pub struct Graph {
    pub area: DrawingArea,
    series: Rc<RefCell<Vec<Series>>>,
    cap: usize,
    times: Rc<RefCell<VecDeque<Instant>>>,
    max_age_s: f64,
}

impl Graph {
    /// `specs` = one `(color, fill?)` per series. `scale` controls whether the
    /// graph uses fixed 0..max, dynamic 0..peak, or dynamic min..max.
    /// `smooth` adds a per-frame scroll between samples.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        w: i32,
        h: i32,
        scale: GraphScale,
        gamma: f64,
        specs: &[(Rgba, bool)],
        interval_s: f64,
        smooth: bool,
        // Minimum vertical range (in value units). With min..max scaling a
        // dead-steady series (e.g. SSD temp) has zero range and collapses onto
        // the baseline; this pads the range so it renders as a centered line and
        // small wiggles stay visible. 0 = no floor.
        min_span: f64,
    ) -> Self {
        let area = DrawingArea::new();
        area.add_css_class("graph");
        area.set_content_width(w);
        area.set_content_height(h);

        // Keep a little more than the visible window so the line can enter from
        // off-canvas smoothly instead of rescaling during warmup.
        let iv = interval_s.max(0.05);
        let len = ((WINDOW_SECS / iv).ceil() as usize + 2).clamp(8, 512);
        let series: Vec<Series> = specs
            .iter()
            .map(|(rgba, fill)| Series {
                buf: VecDeque::with_capacity(len),
                rgba: *rgba,
                fill: *fill,
            })
            .collect();
        let series = Rc::new(RefCell::new(series));
        let times = Rc::new(RefCell::new(VecDeque::with_capacity(len)));
        let max_age_s = WINDOW_SECS + iv * 2.0;

        let s = series.clone();
        let t = times.clone();
        area.set_draw_func(move |_, cr, w, h| {
            let (w, h) = (w as f64, h as f64);
            let ss = s.borrow();
            let times = t.borrow();
            let n = ss
                .iter()
                .map(|se| se.buf.len())
                .min()
                .unwrap_or(0)
                .min(times.len());
            if n == 0 {
                return;
            }
            let (lo, hi) = match scale {
                GraphScale::Fixed(max) => (0.0, max.max(1e-9)),
                GraphScale::DynamicPeak => {
                    let peak = ss
                        .iter()
                        .flat_map(|se| se.buf.iter().take(n).copied())
                        .fold(0.0, f64::max);
                    (0.0, (peak * 1.15).max(1e-9))
                }
                GraphScale::DynamicRange => {
                    let mut lo = f64::INFINITY;
                    let mut hi = f64::NEG_INFINITY;
                    for v in ss.iter().flat_map(|se| se.buf.iter().take(n).copied()) {
                        lo = lo.min(v);
                        hi = hi.max(v);
                    }
                    let span = hi - lo;
                    if span > 1e-6 {
                        hi += span * RANGE_HEADROOM;
                    }
                    (lo, hi)
                }
            };
            // Pad to a minimum range so a near-flat series (e.g. a steady SSD
            // temp) renders as a centered line instead of collapsing onto the
            // baseline. 0 = no floor (cpu/net/disk stay baseline-when-idle).
            let (lo, hi) = if min_span > 0.0 && hi - lo < min_span {
                let mid = (lo + hi) / 2.0;
                (mid - min_span / 2.0, mid + min_span / 2.0)
            } else {
                (lo, hi)
            };
            let span = hi - lo;
            let yof = |v: f64| {
                if span < 1e-6 {
                    h // ewwii dynamic graphs with no range sit on the baseline.
                } else {
                    h - ((v - lo) / span).clamp(0.0, 1.0).powf(gamma) * h
                }
            };
            let now = Instant::now();
            let xof = |i: usize| {
                let age = now
                    .checked_duration_since(times[i])
                    .unwrap_or_default()
                    .as_secs_f64();
                w - (age / WINDOW_SECS) * w
            };
            let _ = cr.save();
            cr.rectangle(0.0, 0.0, w, h);
            cr.clip();

            for se in ss.iter() {
                let (r, g, b, a) = se.rgba;
                let first_x = xof(0);
                let first_y = yof(se.buf[0]);
                let last_y = yof(se.buf[n - 1]);
                if se.fill {
                    // filled area (closed down to the baseline)
                    cr.move_to(first_x, h);
                    cr.line_to(first_x, first_y);
                    for i in 1..n {
                        cr.line_to(xof(i), yof(se.buf[i]));
                    }
                    cr.line_to(w, last_y);
                    cr.line_to(w, h);
                    cr.close_path();
                    cr.set_source_rgba(r, g, b, a * 0.35);
                    let _ = cr.fill();
                }
                // Top curve only: never stroke closing verticals/baselines.
                cr.move_to(first_x, first_y);
                for i in 1..n {
                    cr.line_to(xof(i), yof(se.buf[i]));
                }
                cr.line_to(w, last_y);
                cr.set_source_rgba(r, g, b, a);
                cr.set_line_width(1.0);
                let _ = cr.stroke();
            }
            let _ = cr.restore();
        });

        if smooth {
            area.add_tick_callback(move |area, _| {
                area.queue_draw();
                gtk::glib::ControlFlow::Continue
            });
        }

        Graph {
            area,
            series,
            cap: len,
            times,
            max_age_s,
        }
    }

    /// Push one new value per series.
    pub fn push(&self, vals: &[f64]) {
        {
            let now = Instant::now();
            let mut ss = self.series.borrow_mut();
            let mut times = self.times.borrow_mut();
            times.push_back(now);
            for (i, se) in ss.iter_mut().enumerate() {
                let v = vals.get(i).copied().unwrap_or(0.0).max(0.0);
                se.buf.push_back(v);
            }
            while times.len() > self.cap
                || times.front().is_some_and(|t| {
                    times.len() > 2 && now.duration_since(*t).as_secs_f64() > self.max_age_s
                })
            {
                times.pop_front();
                for se in ss.iter_mut() {
                    se.buf.pop_front();
                }
            }
        }
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
                // Glossy vertical gradient (bright top → base → darker bottom) for
                // a neon-tube sheen matching the filled history graphs.
                let lite = |c: f64| (c + (1.0 - c) * 0.55).min(1.0);
                let grad = gtk::cairo::LinearGradient::new(0.0, 0.0, 0.0, h);
                grad.add_color_stop_rgba(0.0, lite(r), lite(g), lite(b), a);
                grad.add_color_stop_rgba(0.5, r, g, b, a);
                grad.add_color_stop_rgba(1.0, r * 0.55, g * 0.55, b * 0.55, a);
                let _ = cr.set_source(&grad);
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

/// A row of small vertical bars — one per CPU core (conky-style), 0..100 each.
#[derive(Clone)]
pub struct Cores {
    pub area: DrawingArea,
    vals: Rc<RefCell<Vec<f64>>>,
}

impl Cores {
    pub fn new(h: i32, rgba: Rgba) -> Self {
        let area = DrawingArea::new();
        area.add_css_class("bar-meter");
        area.set_content_height(h);
        area.set_hexpand(true);
        let vals: Rc<RefCell<Vec<f64>>> = Rc::new(RefCell::new(Vec::new()));

        let v = vals.clone();
        let (r, g, b, a) = rgba;
        area.set_draw_func(move |_, cr, w, h| {
            let (w, h) = (w as f64, h as f64);
            let vv = v.borrow();
            let n = vv.len();
            if n == 0 {
                return;
            }
            let gap = if n > 24 { 1.0 } else { 2.0 };
            let bw = ((w - gap * (n as f64 - 1.0)) / n as f64).max(1.0);
            let lite = |c: f64| (c + (1.0 - c) * 0.5).min(1.0);
            for (i, &val) in vv.iter().enumerate() {
                let x = i as f64 * (bw + gap);
                // track
                cr.set_source_rgba(1.0, 1.0, 1.0, 0.08);
                cr.rectangle(x, 0.0, bw, h);
                let _ = cr.fill();
                // fill, bottom-up, glossy
                let fh = h * (val / 100.0).clamp(0.0, 1.0);
                if fh > 0.5 {
                    let grad = gtk::cairo::LinearGradient::new(0.0, h - fh, 0.0, h);
                    grad.add_color_stop_rgba(0.0, lite(r), lite(g), lite(b), a);
                    grad.add_color_stop_rgba(1.0, r * 0.6, g * 0.6, b * 0.6, a);
                    cr.rectangle(x, h - fh, bw, fh);
                    let _ = cr.set_source(&grad);
                    let _ = cr.fill();
                }
            }
        });
        Cores { area, vals }
    }

    pub fn set(&self, v: Vec<f64>) {
        *self.vals.borrow_mut() = v;
        self.area.queue_draw();
    }
}
