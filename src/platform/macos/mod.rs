//! macOS 플랫폼 구현 진입점.
//!
//! 모듈 구조:
//! - `startup`  : LaunchAgent 로그인 자동 실행 관리
//! - `tray`     : 트레이 메뉴 빌드 / 메뉴바 타이틀 계산
//! - (mod.rs)   : 이벤트 루프, 폴링 스레드, run() 진입점

pub mod startup;
pub mod tray;

pub use startup::{is_startup_enabled, set_startup_enabled};
pub use tray::{
    app_version_label, build_context_menu, compute_tooltip, render_compact_menu_badge,
};

use std::sync::mpsc;
use std::time::{Duration, Instant};

// ring_badge 재-export: 외부에서 macos::render_ring_badge_image 등으로 접근 가능
#[allow(unused_imports)]
pub use crate::platform::ring_badge::*;

use tray_icon::{menu::MenuEvent, TrayIcon, TrayIconBuilder};
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy},
    window::WindowId,
};

// ── 이벤트 타입 ──────────────────────────────────────────────────────────

#[derive(Debug)]
enum UserEvent {
    UsageUpdated(Box<crate::models::AppUsageData>),
    UpdateChecked(Option<crate::updater::ReleaseDescriptor>),
}

// ── 앱 상태 구조체 ────────────────────────────────────────────────────────

struct MacOsMonitorApp {
    tray_icon: Option<TrayIcon>,
    current_usage: Option<crate::models::AppUsageData>,
    available_update: Option<crate::updater::ReleaseDescriptor>,
    refresh_sender: mpsc::Sender<()>,
    last_settings_mtime: Option<std::time::SystemTime>,
    last_refresh_trigger_mtime: Option<std::time::SystemTime>,
    event_proxy: EventLoopProxy<UserEvent>,
}

impl MacOsMonitorApp {
    fn update_tray(&mut self) {
        let Some(tray) = &mut self.tray_icon else {
            return;
        };
        let settings = crate::app_settings::load_settings();
        let lang = crate::localization::resolve_language(
            settings
                .language
                .as_deref()
                .and_then(crate::localization::LanguageId::from_code),
        );

        let menu = build_context_menu(&self.current_usage, &self.available_update, &settings, lang);
        let _ = tray.set_menu(Some(Box::new(menu)));

        let badge_icon = render_compact_menu_badge(&self.current_usage, &settings);
        let _ = tray.set_icon_as_template(false);
        let _ = tray.set_icon(Some(badge_icon));
        tray.set_title::<&str>(None);

        let tooltip = compute_tooltip(&self.current_usage, settings.usage_countdown, lang);
        let _ = tray.set_tooltip(Some(tooltip));
    }

    /// 설정 파일 변경 감지
    fn check_settings_change(&mut self) {
        let settings_path = crate::app_settings::settings_path();
        if let Ok(meta) = std::fs::metadata(&settings_path) {
            if let Ok(mtime) = meta.modified() {
                if let Some(prev) = self.last_settings_mtime {
                    if prev != mtime {
                        self.last_settings_mtime = Some(mtime);
                        crate::diagnose::log("settings.json modified externally, updating menu bar");
                        self.update_tray();
                        let _ = self.refresh_sender.send(());
                    }
                } else {
                    self.last_settings_mtime = Some(mtime);
                }
            }
        }
    }

    /// refresh.trigger 파일 감지
    fn check_refresh_trigger(&mut self) {
        let trigger_path = crate::app_settings::app_data_directory().join("refresh.trigger");
        if let Ok(meta) = std::fs::metadata(&trigger_path) {
            if let Ok(mtime) = meta.modified() {
                if let Some(prev) = self.last_refresh_trigger_mtime {
                    if prev != mtime {
                        self.last_refresh_trigger_mtime = Some(mtime);
                        crate::diagnose::log("refresh.trigger touched, immediately polling usage");
                        let _ = self.refresh_sender.send(());
                    }
                } else {
                    self.last_refresh_trigger_mtime = Some(mtime);
                }
            }
        }
    }

    /// update_check.trigger 파일 감지
    fn check_update_trigger(&mut self) {
        let trigger = crate::app_settings::app_data_directory().join("update_check.trigger");
        if std::fs::metadata(&trigger).is_ok() {
            let _ = std::fs::remove_file(&trigger);
            crate::diagnose::log("update_check.trigger touched, checking for updates");
            let proxy = self.event_proxy.clone();
            std::thread::spawn(move || {
                let release = match crate::updater::check_for_updates() {
                    Ok(crate::updater::UpdateCheckResult::Available(r)) => Some(r),
                    _ => None,
                };
                let _ = proxy.send_event(UserEvent::UpdateChecked(release));
            });
        }
    }

    /// update_apply.trigger 파일 감지
    fn check_apply_trigger(&mut self, event_loop: &ActiveEventLoop) {
        let trigger = crate::app_settings::app_data_directory().join("update_apply.trigger");
        if std::fs::metadata(&trigger).is_ok() {
            let _ = std::fs::remove_file(&trigger);
            if let Some(release) = &self.available_update {
                crate::diagnose::log(format!(
                    "update_apply.trigger touched, applying update {}",
                    release.latest_version
                ));
                if crate::updater::begin_self_update(release).is_ok() {
                    event_loop.exit();
                }
            }
        }
    }

    /// 메뉴 이벤트 처리
    fn handle_menu_events(&mut self, event_loop: &ActiveEventLoop) {
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            match event.id.as_ref() {
                "refresh" => {
                    let _ = self.refresh_sender.send(());
                }
                "startup" => {
                    set_startup_enabled(!is_startup_enabled());
                    self.update_tray();
                }
                "update" => {
                    if let Some(release) = &self.available_update {
                        if crate::updater::begin_self_update(release).is_ok() {
                            event_loop.exit();
                        }
                    }
                }
                "countdown" => {
                    let mut s = crate::app_settings::load_settings();
                    s.usage_countdown = !s.usage_countdown;
                    let _ = crate::app_settings::save_settings(&s);
                    self.update_tray();
                }
                "inner_ring" => {
                    let mut s = crate::app_settings::load_settings();
                    s.show_inner_ring = !s.show_inner_ring;
                    let _ = crate::app_settings::save_settings(&s);
                    self.update_tray();
                }
                "quit" => {
                    event_loop.exit();
                }
                other if other.starts_with("model:") => {
                    let provider_id = match &other[6..] {
                        "claude" => Some(crate::providers::ProviderId::Claude),
                        "codex" => Some(crate::providers::ProviderId::Codex),
                        "antigravity" => Some(crate::providers::ProviderId::Antigravity),
                        "opencode" => Some(crate::providers::ProviderId::OpenCode),
                        "cursor" => Some(crate::providers::ProviderId::Cursor),
                        _ => None,
                    };
                    if let Some(provider) = provider_id {
                        let mut s = crate::app_settings::load_settings();
                        let current = s.provider_enabled(provider);
                        s.set_provider_enabled(provider, !current);
                        let _ = crate::app_settings::save_settings(&s);
                        let _ = self.refresh_sender.send(());
                        self.update_tray();
                    }
                }
                other if other.starts_with("order:") => {
                    let action = &other[6..];
                    let mut s = crate::app_settings::load_settings();
                    if action == "reset" {
                        s.reset_provider_order();
                    } else if let Some(key) = action.strip_prefix("up:") {
                        if let Some(id) = crate::providers::ProviderId::from_key(key) {
                            s.move_provider_up(id);
                        }
                    } else if let Some(key) = action.strip_prefix("down:") {
                        if let Some(id) = crate::providers::ProviderId::from_key(key) {
                            s.move_provider_down(id);
                        }
                    }
                    let _ = crate::app_settings::save_settings(&s);
                    self.update_tray();
                }
                _ => {}
            }
        }
    }
}

impl ApplicationHandler<UserEvent> for MacOsMonitorApp {
    fn resumed(&mut self, _event_loop: &ActiveEventLoop) {}

    fn window_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        _event: WindowEvent,
    ) {}

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::UsageUpdated(data) => {
                self.current_usage = Some(*data);
                self.update_tray();
            }
            UserEvent::UpdateChecked(release) => {
                self.available_update = release;
                self.update_tray();
            }
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        event_loop.set_control_flow(ControlFlow::wait_duration(Duration::from_millis(250)));
        self.check_settings_change();
        self.check_refresh_trigger();
        self.check_update_trigger();
        self.check_apply_trigger(event_loop);
        self.handle_menu_events(event_loop);
    }
}

// ── 진입점 ────────────────────────────────────────────────────────────────

pub fn run() {
    crate::diagnose::log("macOS Menu Bar runner starting");

    let (refresh_tx, refresh_rx) = mpsc::channel::<()>();

    let event_loop = match EventLoop::<UserEvent>::with_user_event().build() {
        Ok(el) => el,
        Err(err) => {
            crate::diagnose::log(format!("Failed to create event loop: {err}"));
            return;
        }
    };

    let proxy = event_loop.create_proxy();

    // 초기 상태 로드
    let settings = crate::app_settings::load_settings();
    let lang = crate::localization::resolve_language(
        settings
            .language
            .as_deref()
            .and_then(crate::localization::LanguageId::from_code),
    );
    let cached_usage = crate::app_settings::load_usage_cache().map(|c| c.data);
    let initial_menu = build_context_menu(&cached_usage, &None, &settings, lang);
    let initial_tooltip = compute_tooltip(&cached_usage, settings.usage_countdown, lang);
    let initial_badge = render_compact_menu_badge(&cached_usage, &settings);

    // 트레이 아이콘 생성
    let builder = TrayIconBuilder::new()
        .with_menu(Box::new(initial_menu))
        .with_tooltip(initial_tooltip)
        .with_icon(initial_badge)
        .with_icon_as_template(false);

    let tray = match builder.build() {
        Ok(t) => Some(t),
        Err(err) => {
            crate::diagnose::log(format!("Failed to create tray icon: {err}"));
            None
        }
    };

    // 폴링 스레드
    let poll_proxy = proxy.clone();
    std::thread::spawn(move || {
        let mut last_poll = Instant::now() - Duration::from_secs(3600);
        let mut consecutive_failures: u32 = 0;
        loop {
            let settings = crate::app_settings::load_settings();
            let poll_interval = Duration::from_millis(settings.poll_interval_ms as u64);
            let should_poll = last_poll.elapsed() >= poll_interval
                || refresh_rx.recv_timeout(Duration::from_millis(500)).is_ok();

            if should_poll {
                while refresh_rx.try_recv().is_ok() {}
                match crate::poller::poll(
                    settings.enabled_providers(),
                    &settings.accounts,
                    None,
                    false,
                ) {
                    Ok(mut data) => {
                        consecutive_failures = 0;
                        data.select_accounts(&settings.accounts);
                        let _ = crate::app_settings::save_usage_cache(&data, true);
                        let _ = poll_proxy.send_event(UserEvent::UsageUpdated(Box::new(data)));
                        last_poll = Instant::now();
                    }
                    Err(err) => {
                        crate::diagnose::log(format!("macOS poller::poll failed: {err:?}"));
                        consecutive_failures = consecutive_failures.saturating_add(1);
                        let backoff = Duration::from_millis(u64::from(
                            crate::poller::poll_retry_backoff_ms(
                                consecutive_failures,
                                settings.poll_interval_ms,
                            ),
                        ));
                        last_poll = Instant::now() - poll_interval.saturating_sub(backoff);
                    }
                }
            }
        }
    });

    // 업데이트 확인 스레드 (시작 3초 후, 이후 24시간마다)
    let update_proxy = proxy.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(3));
        loop {
            if let Ok(crate::updater::UpdateCheckResult::Available(release)) =
                crate::updater::check_for_updates()
            {
                crate::diagnose::log(format!(
                    "macOS update found: {}",
                    release.latest_version
                ));
                let _ = update_proxy.send_event(UserEvent::UpdateChecked(Some(release)));
            }
            std::thread::sleep(Duration::from_secs(
                crate::updater::AUTO_UPDATE_CHECK_INTERVAL_SECS,
            ));
        }
    });

    let initial_mtime = std::fs::metadata(crate::app_settings::settings_path())
        .ok()
        .and_then(|m| m.modified().ok());

    let mut app = MacOsMonitorApp {
        tray_icon: tray,
        current_usage: cached_usage,
        available_update: None,
        refresh_sender: refresh_tx,
        last_settings_mtime: initial_mtime,
        last_refresh_trigger_mtime: None,
        event_proxy: proxy,
    };

    let _ = event_loop.run_app(&mut app);
}

// ── 테스트 ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{AppUsageData, UsageData, UsageSection};
    use crate::providers::ProviderId;
    use image::Rgba;

    fn make_usage(session_pct: f64, weekly_pct: f64) -> UsageData {
        UsageData {
            session: UsageSection { available: true, percentage: session_pct, resets_at: None },
            weekly: UsageSection { available: true, percentage: weekly_pct, resets_at: None },
            weekly_label: None,
            monthly: None,
            credits: None,
            stale: false,
        }
    }

    #[test]
    fn test_parse_hex_color() {
        assert_eq!(parse_hex_color("#FF0000"), Some(Rgba([255, 0, 0, 255])));
        assert_eq!(parse_hex_color("#4A90D9"), Some(Rgba([74, 144, 217, 255])));
        assert_eq!(parse_hex_color("invalid"), None);
        assert_eq!(parse_hex_color("#FFF"), None);
    }

    #[test]
    fn test_render_single_ring_pair() {
        let img = render_single_ring_pair(
            44,
            0.67,
            0.75,
            Rgba([74, 144, 217, 255]),
            Rgba([74, 222, 128, 255]),
            None,
            Rgba([245, 245, 245, 255]),
            None,
            true,
        );
        assert_eq!(img.width(), 44);
        assert_eq!(img.height(), 44);
        assert!(img.pixels().any(|p| p[3] > 0));
    }

    #[test]
    fn test_compact_menu_badge_renders_one_ring_per_provider() {
        let mut data = AppUsageData::default();
        data.insert(ProviderId::Codex, make_usage(30.0, 50.0));
        data.insert(ProviderId::Claude, make_usage(10.0, 47.0));
        data.insert(ProviderId::Antigravity, make_usage(0.0, 25.0));

        let mut settings = crate::app_settings::SettingsFile::default();
        settings.set_provider_enabled(ProviderId::Codex, true);
        settings.set_provider_enabled(ProviderId::Claude, true);
        settings.set_provider_enabled(ProviderId::Antigravity, true);
        settings.usage_countdown = true;

        let _icon = render_compact_menu_badge(&Some(data.clone()), &settings);
        let img = render_ring_badge_image_at_size(&data, &settings, 44, 4)
            .expect("three enabled providers should render");
        assert_eq!(img.width(), 140);
        assert_eq!(img.height(), 44);
        assert!(img.pixels().any(|p| p[3] > 0));
    }

    #[test]
    fn test_app_version_label_tracks_package_version() {
        let label = app_version_label();
        assert!(label.starts_with("AI Usage Monitor v"));
        assert!(label.contains(env!("CARGO_PKG_VERSION")));
    }

    #[test]
    fn test_ring_antialias() {
        assert_eq!(ring_antialias(15.0, 14.0, 18.0), 1.0);
        assert_eq!(ring_antialias(10.0, 14.0, 18.0), 0.0);
        let edge = ring_antialias(13.7, 14.0, 18.0);
        assert!(edge > 0.0 && edge < 1.0);
    }
}
