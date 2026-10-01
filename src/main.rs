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
    if args.iter().any(|arg| arg == "--json") {
        std::process::exit(run_json_once());
    }
    if let Some(exit_code) = updater::handle_cli_mode(&args) {
        std::process::exit(exit_code);
    }

    platform::run();
}

/// `--json`: 1회 폴링 후 사용량을 JSON 한 줄로 출력하고 종료한다.
/// 스크립트·Raycast 연동용으로 GUI를 띄우지 않는다.
fn run_json_once() -> i32 {
    #[cfg(windows)]
    unsafe {
        use windows::Win32::System::Console::AttachConsole;
        const ATTACH_PARENT_PROCESS: u32 = 0xFFFF_FFFF;
        let _ = AttachConsole(ATTACH_PARENT_PROCESS);
    }

    let settings = app_settings::load_settings();
    let mut data = match poller::poll(settings.enabled_providers(), &settings.accounts, None, false) {
        Ok(data) => data,
        Err(failure) => {
            eprintln!(
                "poll failed for {}: {:?}",
                failure.provider.descriptor().display_name,
                failure.error
            );
            return 1;
        }
    };
    data.select_accounts(&settings.accounts);
    match serde_json::to_string(&data) {
        Ok(json) => {
            println!("{json}");
            0
        }
        Err(error) => {
            eprintln!("failed to encode usage as JSON: {error}");
            1
        }
    }
}
