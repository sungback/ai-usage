#![cfg_attr(windows, windows_subsystem = "windows")]

mod accounts;
mod app_settings;
// Windows 전용: 네이티브 Windows 트레이/위젯 메뉴가 테스트 외의 유일한 소비자입니다
// (macOS는 `tray.set_menu()`를 통해 자체 NSMenu를 구성합니다). 모델/유효성 검사 로직이
// macOS CI에서도 계속 실행될 수 있도록 `test` 환경에서도 컴파일됩니다.
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
