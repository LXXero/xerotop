// xerotop — a gkrellm-style, battery-conscious Wayland system monitor.
// Single process: native /proc & /sys metrics, GTK4 + layer-shell, TOML config,
// one central battery-aware scheduler. No subprocesses, no fork storm.

mod bar;
mod config;
mod metrics;
mod panels;
mod power;
mod prefs;
mod taskbar;
mod theme;
mod tray;
mod widgets;

use gtk::Application;
use gtk::glib;
use gtk::prelude::*;

const APP_ID: &str = "cc.xeron.xerotop";

fn main() -> glib::ExitCode {
    let app = Application::builder().application_id(APP_ID).build();
    app.connect_activate(|app| {
        let _bar = bar::build(app, config::load());
    });
    app.run()
}
