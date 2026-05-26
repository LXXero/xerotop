// xerotop — a gkrellm-style, battery-conscious Wayland system monitor.
// Single process: native /proc & /sys metrics, GTK4 + layer-shell, TOML config,
// one central battery-aware scheduler. No subprocesses, no fork storm.

mod bar;
mod config;
mod mail;
mod metrics;
mod panels;
mod power;
mod prefs;
mod taskbar;
mod theme;
mod tray;
mod weather;
mod widgets;

use gtk::Application;
use gtk::glib;
use gtk::prelude::*;

const APP_ID: &str = "cc.xeron.xerotop";

fn main() -> glib::ExitCode {
    // Open the prefs window on launch with `xerotop --prefs`. Read from env so
    // GApplication's own option parser doesn't choke on the flag.
    let open_prefs = std::env::args().any(|a| a == "--prefs");

    let app = Application::builder().application_id(APP_ID).build();
    app.connect_activate(move |app| {
        let bar = bar::build(app, config::load());
        if open_prefs {
            prefs::open(&bar);
        }
    });
    // Pass no args through to GApplication (we handled them above).
    app.run_with_args::<&str>(&[])
}
