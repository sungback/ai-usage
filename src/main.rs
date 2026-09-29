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
mod diagnose;
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
        let _ = std::fs::write(std::env::temp_dir().join("claude-panic.log"), &msg);
        eprintln!("{msg}");
    }));

    let args: Vec<String> = std::env::args().collect();
    let diagnose_enabled = args
        .iter()
        .any(|arg| arg == "--diagnose" || arg == "--diagnose-append");
    if diagnose_enabled {
        let init_result = if args.iter().any(|arg| arg == "--diagnose-append") {
            diagnose::init_append()
        } else {
            diagnose::init()
        };
        match init_result {
            Ok(path) => diagnose::log(format!("startup args={args:?} log_path={}", path.display())),
            Err(error) => {
                // Logging may not be available yet, but keep startup behavior unchanged.
                let _ = error;
            }
        }
    }

    if let Some(exit_code) = updater::handle_cli_mode(&args) {
        if diagnose_enabled {
            diagnose::log(format!("cli mode exited with code {exit_code}"));
        }
        std::process::exit(exit_code);
    }

    if diagnose_enabled {
        diagnose::log("entering platform::run");
    }
    platform::run();
}
