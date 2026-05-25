// xerotop — a gkrellm-style, battery-conscious Wayland system monitor.
// Single process: native /proc & /sys metrics, GTK4 + layer-shell, TOML config,
// one central battery-aware scheduler. No subprocesses, no fork storm.

mod bar;
mod config;
mod metrics;
mod panels;
mod power;
mod taskbar;
mod tray;
mod widgets;

use gtk::prelude::*;
use gtk::{Application, glib};

const APP_ID: &str = "cc.xeron.xerotop";

fn main() -> glib::ExitCode {
    let app = Application::builder().application_id(APP_ID).build();
    app.connect_startup(|_| load_css());
    app.connect_activate(|app| {
        let _bar = bar::build(app, config::load());
    });
    app.run()
}

fn load_css() {
    let provider = gtk::CssProvider::new();
    provider.load_from_data(include_str!("style.css"));
    if let Some(display) = gtk::gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}
