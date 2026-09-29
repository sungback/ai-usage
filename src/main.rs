#![cfg_attr(windows, windows_subsystem = "windows")]

mod accounts;
mod app_settings;
// Windows-only: the native Windows tray/widget menu is its sole non-test consumer
// (macOS builds its own NSMenu via `tray.set_menu()`). Compiled under `test` too so
// the model/validation logic keeps being exercised on macOS CI.
#[cfg(any(windows, test))]
mod context_menu;
#[cfg(windows)]
mod desktop_compositor;
mod localization;
mod models;
mod native_interop;
mod platform;
mod poller;
mod providers;
mod theme;
mod theme_engine;
#[cfg(windows)]
mod tray_icon;
mod updater;
#[cfg(windows)]
mod window;
mod winsqlite;

fn main() {
    std::panic::set_hook(Box::new(|info| {
        let msg = format!("PANIC: {info}");
        let _ = std::fs::write(std::env::temp_dir().join("ai-usage-panic.log"), &msg);
        eprintln!("{msg}");
    }));

    let args: Vec<String> = std::env::args().collect();
    if let Some(exit_code) = updater::handle_cli_mode(&args) {
        std::process::exit(exit_code);
    }

    platform::run();
}
