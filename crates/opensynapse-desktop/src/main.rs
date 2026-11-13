//! OpenSynapse desktop GUI. See README.md.
//!
//! Uses the background agent over IPC when it runs, else talks to the device
//! in-process on a worker thread. Lean by design: no network, no telemetry, no
//! continuous repaint.

// No console window behind the GUI in release builds on Windows.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod agent_ctl;
mod app;
mod app_profiles_view;
mod backend;
mod ops;
mod remote;
mod user_settings;
mod worker;

/// Window icon size; the OS scales it for title bar and taskbar.
const WINDOW_ICON: u32 = 64;

fn main() -> eframe::Result {
    let icon = eframe::egui::IconData {
        rgba: opensynapse_icon::render(WINDOW_ICON),
        width: WINDOW_ICON,
        height: WINDOW_ICON,
    };
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("OpenSynapse")
            .with_icon(icon)
            .with_inner_size([960.0, 760.0])
            .with_min_inner_size([520.0, 360.0]),
        ..Default::default()
    };
    eframe::run_native(
        "OpenSynapse",
        options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc)))),
    )
}
