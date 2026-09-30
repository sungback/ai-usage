//! Windows 네이티브 창·트레이 — 처음 보시는 분을 위한 안내.
//!
//! - 작업표시줄 위젯과 트레이 메뉴를 만들고, 메시지 루프에서 클릭·타이머를 받습니다.
//! - `window/` 폴더의 작은 파일들이 실제 일(위치·마우스·메시지)을 나눠 맡습니다.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use windows::core::PCWSTR;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::{GetModuleFileNameW, GetModuleHandleW};
use windows::Win32::System::Registry::*;
use windows::Win32::System::Threading::{CreateMutexW, WaitForSingleObject};
use windows::Win32::UI::Accessibility::HWINEVENTHOOK;
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetDoubleClickTime, ReleaseCapture, SetCapture, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT,
};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::app_settings::{
    self, load_settings, save_settings, LegacyPlacement, PlacementOverride, SettingsFile,
    POLL_15_MIN, POLL_15_MIN_SECONDS, POLL_1_HOUR, POLL_1_HOUR_SECONDS, POLL_1_MIN,
    POLL_1_MIN_SECONDS, POLL_5_MIN, POLL_5_MIN_SECONDS,
};
use crate::context_menu::{self, ContextMenuAction, ContextMenuItem, ContextMenuItemKind};
use crate::localization::{self, LanguageId, Strings};
use crate::models::{AppUsageData, UsageData};
use crate::native_interop::{
    self, TIMER_CLOCK, TIMER_COUNTDOWN, TIMER_MOUSE_CLICK, TIMER_POLL, TIMER_RESET_POLL,
    TIMER_TRAY_HOVER, TIMER_TRAY_REPOSITION, TIMER_UPDATE_CHECK, TIMER_WINDOW_STATE,
    WM_APP_QUIT, WM_APP_REFRESH_NOW, WM_APP_SETTINGS_UPDATED, WM_APP_TASKBAR_COLLISION,
    WM_APP_TRAY, WM_APP_USAGE_UPDATED,
};
use crate::poller;
use crate::providers::{ProviderId, ProviderSet};
use crate::theme;
use crate::theme_engine::{
    self, Canvas, DataContext, HorizontalAnchor, MouseActionEffect, MouseActionOverrideKey,
    MouseEventKind, ReferenceRegion, SurfaceNest, ThemeDocument, ThemeRuntime, VerticalAnchor,
};
use crate::tray_icon;
use crate::updater::{self, ReleaseDescriptor, UpdateCheckResult};

/// UI 스레드가 값을 발행한 후 감시자(watchdog)가 사용하는 복사 가능한 HWND 값입니다.
#[derive(Clone, Copy, PartialEq, Eq)]
struct SendHwnd(isize);

// SAFETY: 이 래퍼는 절대 윈도우의 소유권을 이전하지 않습니다. 크로스 스레드 사용자는
// 이 값을 IsWindow나 PostMessageW 같이 다른 스레드에서 생성된 핸들을 
// 명시적으로 허용하는 Win32 API로 다시 전달할 때만 사용합니다.
unsafe impl Send for SendHwnd {}

impl SendHwnd {
    fn from_hwnd(hwnd: HWND) -> Self {
        Self(hwnd.0 as isize)
    }
    fn to_hwnd(self) -> HWND {
        HWND(self.0 as *mut _)
    }
}

/// UI 컨트롤러가 수명을 소유하는 복사 가능한 이벤트 후크(event-hook) 값입니다.
#[derive(Clone, Copy)]
struct SendWinEventHook(isize);

// SAFETY: 이 후크는 저장되거나 UnhookWinEvent로 전달되기만 합니다. 콜백 작업은
// Win32를 통해 마샬링되며, Rust 데이터는 절대 이 값을 통해 역참조되지 않습니다.
unsafe impl Send for SendWinEventHook {}

impl SendWinEventHook {
    fn from_hook(hook: HWINEVENTHOOK) -> Self {
        Self(hook.0 as isize)
    }

    fn to_hook(self) -> HWINEVENTHOOK {
        HWINEVENTHOOK(self.0 as *mut _)
    }
}

/// 공유되는 애플리케이션 상태입니다.
struct AppState {
    hwnd: SendHwnd,
    taskbar_hwnd: Option<SendHwnd>,
    tray_notify_hwnd: Option<SendHwnd>,
    win_event_hook: Option<SendWinEventHook>,
    is_dark: bool,
    embedded: bool,
    language_override: Option<LanguageId>,
    language: LanguageId,

    providers: ProviderSet,
    accounts: crate::accounts::AccountSettings,

    data: Option<AppUsageData>,

    poll_interval_ms: u32,
    retry_count: u32,
    force_notify_auth_error: bool,
    auth_error_paused_polling: bool,
    auth_watch_mode: poller::CredentialWatchMode,
    auth_watch_snapshot: poller::CredentialWatchSnapshot,
    last_poll_ok: bool,
    update_status: UpdateStatus,
    last_update_check_unix: Option<u64>,

    taskbar_index: usize,
    tray_offset: i32,
    dragging: bool,
    pending_drag: bool,
    drag_start_cursor: POINT,
    drag_start_origin: POINT,
    drag_start_client_x: i32,
    auto_ejected: bool,
    auto_ejected_origin: Option<POINT>,
    is_switching_window_style: bool,
    is_snapped: bool,
    placement_override: Option<PlacementOverride>,
    floating_card_opacity: Option<u8>,
    window_state_timer_active: bool,

    custom_theme_enabled: bool,
    usage_countdown: bool,
    taskbar_ring_badge: bool,
    active_theme_path: Option<PathBuf>,
    active_theme: Option<ThemeDocument>,
    theme_clock_interval: Option<Duration>,
    tray_theme_uses_current_time: bool,
    mirror_hwnds: Vec<SendHwnd>,
    desktop_hwnds: Vec<Option<SendHwnd>>,
    mouse_action_overrides: HashMap<MouseActionOverrideKey, theme_engine::Expression>,
    hovered_mouse_layer: Option<(usize, String)>,
    pending_mouse_click: Option<PendingMouseClick>,
    suppress_next_left_up: bool,
}

#[derive(Clone, Debug)]
struct PendingMouseClick {
    surface_index: usize,
    object_id: String,
}

#[derive(Clone, Debug)]
enum UpdateStatus {
    Idle,
    Checking,
    Applying,
    UpToDate,
    Available(ReleaseDescriptor),
}

fn perform_update_action(hwnd: HWND) {
    let release = {
        let state = lock_state();
        let Some(state) = state.as_ref() else {
            return;
        };
        if matches!(
            state.update_status,
            UpdateStatus::Checking | UpdateStatus::Applying
        ) {
            return;
        }
        match &state.update_status {
            UpdateStatus::Available(release) => Some(release.clone()),
            _ => None,
        }
    };
    match release {
        Some(release) => begin_update_apply(hwnd, release),
        None => begin_update_check(hwnd, true),
    }
}

// 업데이트 주기(frequency)를 위한 메뉴 항목 ID들입니다.
const IDM_FREQ_1MIN: u16 = 10;
const IDM_FREQ_5MIN: u16 = 11;
const IDM_FREQ_15MIN: u16 = 12;
const IDM_FREQ_1HOUR: u16 = 13;
const IDM_START_WITH_WINDOWS: u16 = 20;
const IDM_VERSION_ACTION: u16 = 31;
const IDM_TOGGLE_USAGE_DIRECTION: u16 = 32;
const IDM_TOGGLE_TASKBAR_RING_BADGE: u16 = 33;

const WM_DPICHANGED_MSG: u32 = 0x02E0;
const WM_APP_UPDATE_CHECK_COMPLETE: u32 = WM_APP + 2;
const TRAY_ICON_UPDATE_REPOSITION_SUPPRESS_MS: u64 = 750;
const WINDOW_STATE_INTERVAL_MS: u32 = 250;

fn open_web_url(hwnd: HWND, url: &str) {
    if !theme_engine::supported_url(url) {
        return;
    }
    unsafe {
        let operation = native_interop::wide_str("open");
        let url = native_interop::wide_str(url.trim());
        let _ = ShellExecuteW(
            Some(hwnd),
            PCWSTR::from_raw(operation.as_ptr()),
            PCWSTR::from_raw(url.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        );
    }
}

/// 감시자(watchdog) 스레드가 explorer.exe의 재시작(작업 표시줄을 재생성하고
/// 우리의 트레이 아이콘 등록을 지워버림)을 확인하기 위해 폴링하는 주기입니다.
const TASKBAR_WATCH_INTERVAL_SECS: u64 = 2;

static SUPPRESS_TRAY_REPOSITION_UNTIL: Mutex<Option<Instant>> = Mutex::new(None);

/// 현재 시스템의 DPI입니다. (96 = 100% 스케일, 144 = 150%, 192 = 200% 등)
static CURRENT_DPI: AtomicU32 = AtomicU32::new(96);
static POLL_IN_FLIGHT: AtomicBool = AtomicBool::new(false);
static POLL_PENDING: AtomicBool = AtomicBool::new(false);

/// 우리 윈도우가 속한 모니터의 DPI를 다시 쿼리하고 캐시된 값을 업데이트합니다.
/// GetDpiForWindow를 사용하여 실시간 DPI를 반환받습니다 (프로세스 시작 시 캐시되어
/// 절대 변하지 않는 GetDpiForSystem과는 다릅니다).
fn refresh_dpi() {
    let hwnd = {
        let state = lock_state();
        state.as_ref().map(|s| s.hwnd.to_hwnd())
    };
    if let Some(hwnd) = hwnd {
        let dpi = unsafe { GetDpiForWindow(hwnd) };
        if dpi > 0 {
            CURRENT_DPI.store(dpi, Ordering::Relaxed);
        }
    }
}

fn display_scale(display_index: usize) -> f64 {
    let displays = native_interop::find_monitors();
    let Some(display) = displays
        .get(display_index)
        .copied()
        .or_else(|| displays.first().copied())
    else {
        return 1.0;
    };
    monitor_scale(display)
}

fn monitor_scale(display: native_interop::DisplayMonitor) -> f64 {
    let mut dpi_x = 96;
    let mut dpi_y = 96;
    if unsafe { GetDpiForMonitor(display.handle, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) }
        .is_ok()
        && dpi_x > 0
    {
        (dpi_x as f64 / 96.0).clamp(0.25, 8.0)
    } else {
        let system_dpi = unsafe { GetDpiForSystem() };
        if system_dpi > 0 {
            (system_dpi as f64 / 96.0).clamp(0.25, 8.0)
        } else {
            1.0
        }
    }
}

fn migrated_theme_placement(legacy: LegacyPlacement) -> (usize, i32) {
    let displays = native_interop::find_monitors();
    let taskbars = native_interop::find_taskbars();
    let display_index = taskbars
        .get(legacy.taskbar_index)
        .or_else(|| taskbars.first())
        .map(|taskbar| unsafe { MonitorFromWindow(taskbar.hwnd, MONITOR_DEFAULTTOPRIMARY) })
        .and_then(|monitor| {
            displays
                .iter()
                .position(|display| display.handle == monitor)
        })
        .unwrap_or_else(|| legacy.taskbar_index.min(displays.len().saturating_sub(1)));
    let offset_x = legacy_offset_to_theme_offset(legacy.tray_offset, display_scale(display_index));
    (display_index, offset_x)
}

fn legacy_offset_to_theme_offset(tray_offset: i32, scale: f64) -> i32 {
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    -((tray_offset.max(0) as f64 / scale).round() as i32)
}

fn theme_surface_scale(theme: &ThemeDocument, surface_index: usize) -> f64 {
    let display_index = theme
        .surfaces
        .get(surface_index)
        .map(|surface| surface.placement.reference.display)
        .unwrap_or(theme.placement.reference.display);
    display_scale(display_index)
}

fn logical_host_dimension(physical: i32, scale: f64) -> u32 {
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    (physical.max(1) as f64 / scale)
        .round()
        .clamp(1.0, u32::MAX as f64) as u32
}

fn scaled_theme_dimension(logical: u32, scale: f64) -> i32 {
    (logical as f64 * scale).round().clamp(1.0, 8192.0) as i32
}

/// 두 번의 재시작(relaunch) 간격이 이 값보다 짧으면 재시작 폭주(storm)로 간주합니다
/// (예: explorer.exe가 크래시 루프에 빠진 경우). 폭주가 감지되면 루프를 멈추고 백오프(대기)합니다.
const RELAUNCH_THROTTLE_SECS: u64 = 10;
const RELAUNCH_BACKOFF_SECS: u64 = 30;
/// 재시작된 자식 프로세스에 설정되는 환경 변수 플래그입니다. 즉시 종료되지 않고
/// 이전 인스턴스의 단일 인스턴스(single-instance) 뮤텍스를 기다리도록 합니다.
const ENV_RELAUNCH: &str = "AI_USAGE_RELAUNCH";
/// 이 프로세스를 생성한 재시작 이벤트의 Unix 타임스탬프(초 단위)입니다.
/// 자식 프로세스가 재시작 폭주를 감지할 수 있도록 전달됩니다.
const ENV_LAST_RELAUNCH_UNIX: &str = "AI_USAGE_LAST_RELAUNCH_UNIX";

/// explorer.exe가 재시작된 후 위젯을 완전히 새로운 프로세스로 재시작합니다.
///
/// 셸이 재시작되면 임베드된 자식 윈도우가 완전히 파괴되며 (단순히 고아가 되는 것이 아니라 `IsWindow`가 false를 반환함),
/// UI 스레드는 다시 생성할 윈도우 없이 `GetMessage`에 멈춰있게 됩니다.
/// 따라서 완전히 새로운 프로세스를 생성하여 새로 만들어진 작업 표시줄에 다시 임베드하고,
/// 현재 프로세스는 종료하는 것이 가장 확실한 복구 방법입니다.
/// 자식 프로세스는 `ENV_RELAUNCH` 플래그를 통해 현재 인스턴스의 단일 인스턴스 뮤텍스가 해제될 때까지 대기한 후 제어권을 넘겨받습니다 (`run` 함수의 가드 부분 참고).
fn relaunch_self() {
    // 현재 우리를 생성한 재시작 이벤트 직후에 다시 재시작하려고 한다면 백오프(대기)합니다:
    // 이는 단순한 일회성 재시작이 아니라 셸이 크래시 루프에 빠졌음을 의미합니다.
    let now = now_unix_secs();
    let last = std::env::var(ENV_LAST_RELAUNCH_UNIX)
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0);
    if last != 0 && now.saturating_sub(last) < RELAUNCH_THROTTLE_SECS {
        std::thread::sleep(Duration::from_secs(RELAUNCH_BACKOFF_SECS));
    }

    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(_) => {
            return;
        }
    };

    let args: Vec<String> = std::env::args().skip(1).collect();
    match std::process::Command::new(exe)
        .args(&args)
        .env(ENV_RELAUNCH, "1")
        .env(ENV_LAST_RELAUNCH_UNIX, now.to_string())
        .spawn()
    {
        Ok(_) => {
            std::process::exit(0);
        }
        Err(_) => {
        }
    }
}

/// explorer.exe의 재시작을 감지하고 복구합니다.
///
/// 파일 탐색기(Explorer)는 작업 표시줄과 바탕 화면 표면(surface) 호스트를 모두 소유합니다.
/// 탐색기가 재시작되면 모든 자식 위젯 윈도우가 파괴되며, 기본 창이 여기에 호스팅되어 있었다면 UI 메시지 루프도 함께 손실됩니다.
/// 이를 대비해 전용 스레드가 모든 네이티브 표면 핸들을 검사하며, 셸이 돌아온 후 앱을 재시작합니다.
fn spawn_taskbar_watchdog() {
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(TASKBAR_WATCH_INTERVAL_SECS));
        let invalid = {
            let state = lock_state();
            let Some(state) = state.as_ref() else {
                continue;
            };
            let shell_hosted = theme_with_placement(state, false)
                .as_ref()
                .is_some_and(|theme| {
                    theme.surfaces.iter().any(|surface| {
                        matches!(
                            surface
                                .placement
                                .nest
                                .resolve(surface.placement.reference.region),
                            SurfaceNest::Taskbar | SurfaceNest::Desktop
                        )
                    })
                });
            if !shell_hosted {
                continue;
            }
            std::iter::once(state.hwnd)
                .chain(state.mirror_hwnds.iter().copied())
                .chain(state.desktop_hwnds.iter().flatten().copied())
                .any(|window| unsafe {
                    let hwnd = window.to_hwnd();
                    if !IsWindow(Some(hwnd)).as_bool() {
                        return true;
                    }
                    // 셸 윈도우(예: Shell_TrayWnd나 Progman) 내부에 호스팅될 때, 
                    // 탐색기가 재시작되어도 Windows가 항상 크로스 프로세스 자식 윈도우를 파괴하는 것은 아닙니다.
                    // 만약 이 윈도우의 부모가 파괴되었다면 이 윈도우도 무효(invalid)로 표시합니다.
                    match GetParent(hwnd).ok() {
                        Some(p) if !p.is_invalid() => !IsWindow(Some(p)).as_bool(),
                        _ => false,
                    }
                })
        };
        if invalid && !native_interop::find_taskbars().is_empty() {
            relaunch_self();
        }

        static LAST_WATCHDOG_TRAY_RECT: Mutex<Option<RECT>> = Mutex::new(None);
        let (reposition_target, current_tray_rect) = {
            let state = lock_state();
            if let Some(s) = state.as_ref() {
                let tray_hwnd = s.tray_notify_hwnd.map(|h| h.to_hwnd());
                let target_hwnd = s.hwnd.to_hwnd();
                (
                    target_hwnd,
                    tray_hwnd.and_then(native_interop::get_window_rect_safe),
                )
            } else {
                (HWND::default(), None)
            }
        };
        if !reposition_target.is_invalid() && current_tray_rect.is_some() {
            let mut last_rect = LAST_WATCHDOG_TRAY_RECT
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if rect_changed(*last_rect, current_tray_rect) {
                *last_rect = current_tray_rect;
                unsafe {
                    let _ = PostMessageW(
                        Some(reposition_target),
                        WM_TIMER,
                        WPARAM(TIMER_TRAY_REPOSITION),
                        LPARAM(0),
                    );
                }
            }
        }

        let collision_action = {
            let state = lock_state();
            if let Some(s) = state.as_ref() {
                if s.dragging {
                    None
                } else if !s.auto_ejected
                    && s.embedded
                    && s.placement_override
                        .as_ref()
                        .is_none_or(|p| p.nest != "floating")
                {
                    let widget_hwnd = s.hwnd.to_hwnd();
                    let taskbar_hwnd = s.taskbar_hwnd.map(|h| h.to_hwnd());
                    if let (Some(tb), Some(widget_rect)) = (
                        taskbar_hwnd,
                        native_interop::get_window_rect_safe(widget_hwnd),
                    ) {
                        native_interop::get_taskbar_rect(tb).and_then(|taskbar_rect| {
                            let slot = positioning::taskbar_free_dock_slot(tb, taskbar_rect);
                            positioning::overlaps_taskbar_apps(taskbar_rect, slot, widget_rect)
                                .then_some((widget_hwnd, 1usize))
                        })
                    } else {
                        None
                    }
                } else if s.auto_ejected {
                    let widget_hwnd = s.hwnd.to_hwnd();
                    let taskbar_hwnd = s.taskbar_hwnd.map(|h| h.to_hwnd());
                    if let (Some(tb), Some(taskbar_rect)) = (
                        taskbar_hwnd,
                        taskbar_hwnd.and_then(native_interop::get_window_rect_safe),
                    ) {
                        let slot = positioning::taskbar_free_dock_slot(tb, taskbar_rect);
                        restored_dock_rect(s, tb, taskbar_rect).and_then(|rect| {
                            positioning::dock_rect_fits(taskbar_rect, slot, rect, 20)
                                .then_some((widget_hwnd, 0usize))
                        })
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            }
        };

        if let Some((target_hwnd, action)) = collision_action {
            unsafe {
                let _ = PostMessageW(
                    Some(target_hwnd),
                    native_interop::WM_APP_TASKBAR_COLLISION,
                    WPARAM(action),
                    LPARAM(0),
                );
            }
        }
    });
}

static STATE: Mutex<Option<AppState>> = Mutex::new(None);

/// 오염된(poisoned) 뮤텍스를 복구하며 STATE를 안전하게 잠급니다.
fn lock_state() -> MutexGuard<'static, Option<AppState>> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

fn theme_runtime_from_state(state: &AppState) -> ThemeRuntime {
    let (poll_ok, has_error) = poll_display_state(
        state.last_poll_ok,
        state.retry_count,
        state.auth_error_paused_polling,
        state.data.as_ref(),
    );
    let nest = if state.auto_ejected {
        SurfaceNest::Floating
    } else if let Some(ref ov) = state.placement_override {
        if ov.nest == "floating" {
            SurfaceNest::Floating
        } else {
            SurfaceNest::Taskbar
        }
    } else {
        SurfaceNest::Taskbar
    };
    let opacity = state.floating_card_opacity.unwrap_or(85);
    ThemeRuntime::from_providers(state.providers)
        .with_poll_state(poll_ok, has_error)
        .with_language(state.language)
        .with_countdown(state.usage_countdown)
        .with_nest(nest)
        .with_floating_card_opacity(opacity)
}

/// 일시적인 에러(outage) 발생 시, 재시도하는 동안 이전의 실제 데이터가 계속 표시되도록 유지합니다.
/// 인증 실패나 캐시된 데이터가 없는 상태에서의 실패는 여전히 명시적인 에러 상태를 표시해야 합니다.
fn poll_display_state(
    last_poll_ok: bool,
    retry_count: u32,
    auth_error_paused_polling: bool,
    data: Option<&AppUsageData>,
) -> (bool, bool) {
    if let Some(data) = data.filter(|data| !data.accounts.is_empty()) {
        let has_error = data.accounts.iter().any(|account| account.error.is_some());
        return (
            !data.is_empty(),
            data.is_empty() && (has_error || retry_count > 0),
        );
    }
    let has_usable_stale_data = !auth_error_paused_polling
        && data.is_some_and(|data| data.iter().any(|(_, usage)| usage.stale));
    (
        last_poll_ok || has_usable_stale_data,
        retry_count > 0 && !has_usable_stale_data,
    )
}

fn effective_theme_from_state(state: &AppState) -> Option<ThemeDocument> {
    theme_with_placement(state, state.auto_ejected)
}

fn theme_with_placement(state: &AppState, auto_ejected: bool) -> Option<ThemeDocument> {
    let mut theme = state.active_theme.as_ref().map(|theme| {
        theme_engine::apply_mouse_action_overrides(theme, &state.mouse_action_overrides)
    })?;
    let floating = if auto_ejected {
        state.auto_ejected_origin.map(|point| (None, point))
    } else {
        state
            .placement_override
            .as_ref()
            .filter(|p| p.nest == "floating")
            .map(|p| {
                (
                    Some(p.monitor_index),
                    POINT {
                        x: p.screen_x,
                        y: p.screen_y,
                    },
                )
            })
    };
    if let Some((saved_monitor, point)) = floating {
        let displays = native_interop::find_monitors();
        let selected = saved_monitor
            .and_then(|index| displays.get(index).copied().map(|d| (index, d)))
            .unwrap_or_else(|| positioning::monitor_for_point(&displays, point));
        apply_floating_position(&mut theme, state, selected.0, selected.1, point);
    } else if let Some(p) = state
        .placement_override
        .as_ref()
        .filter(|p| p.nest == "taskbar")
    {
        let displays = native_interop::find_monitors();
        let index = if p.monitor_index < displays.len() {
            p.monitor_index
        } else {
            0
        };
        let horizontal = taskbar_is_horizontal(index);
        let placement =
            positioning::dock_placement(index, p.tray_offset, display_scale(index), horizontal);
        positioning::override_primary_placement(&mut theme, placement);
    }
    Some(theme)
}

fn apply_floating_position(
    theme: &mut ThemeDocument,
    state: &AppState,
    index: usize,
    display: native_interop::DisplayMonitor,
    point: POINT,
) {
    let mut placement = positioning::floating_placement(index);
    positioning::override_primary_placement(theme, placement.clone());
    let scale = monitor_scale(display);
    let runtime = theme_runtime_for_surface(theme, 0, theme_runtime_from_state(state));
    let frame = positioning::widget_frame(theme, state.data.as_ref(), runtime, scale);
    let offset = positioning::clamped_floating_offset(point, display.rect, &frame, scale);
    placement.offset_x = offset.x;
    placement.offset_y = offset.y;
    positioning::override_primary_placement(theme, placement);
}

/// 테마에 작성된 배치(placement)나 저장된 드래그 오프셋을 포함하여,
/// 실제로 복원될 대상 사각형 영역(rectangle)을 계산합니다. 감시자(watchdog)도 이 대상을 검사해야 합니다.
fn restored_dock_rect(state: &AppState, taskbar: HWND, taskbar_rect: RECT) -> Option<RECT> {
    let theme = theme_with_placement(state, false)?;
    let surface = theme.surfaces.first()?;
    if surface
        .placement
        .nest
        .resolve(surface.placement.reference.region)
        != SurfaceNest::Taskbar
    {
        return None;
    }
    let displays = native_interop::find_monitors();
    let display = displays
        .get(surface.placement.reference.display)
        .or_else(|| displays.first())?;
    if unsafe { MonitorFromWindow(taskbar, MONITOR_DEFAULTTOPRIMARY) } != display.handle {
        return None;
    }
    let runtime = theme_runtime_for_surface(&theme, 0, theme_runtime_from_state(state));
    let scale = monitor_scale(*display);
    let frame = positioning::widget_frame(&theme, state.data.as_ref(), runtime, scale);
    let offsets = theme_engine::resolve_surface_placement(&theme, 0, state.data.as_ref(), runtime);
    let mut placement = surface.placement.clone();
    placement.offset_x = offsets.offset_x;
    placement.offset_y = offsets.offset_y;
    let tray = native_interop::find_child_window(taskbar, "TrayNotifyWnd")
        .and_then(native_interop::get_window_rect_safe);
    Some(positioning::surface_screen_rect(
        &placement,
        frame.width,
        frame.height,
        scale,
        display.rect,
        Some(taskbar_rect),
        tray,
    ))
}

fn theme_has_floating_surface(theme: &ThemeDocument) -> bool {
    theme.surfaces.iter().any(|surface| {
        surface
            .placement
            .nest
            .resolve(surface.placement.reference.region)
            == SurfaceNest::Floating
    })
}

fn window_state_timer_required(state: &AppState) -> bool {
    state.custom_theme_enabled
        && effective_theme_from_state(state)
            .as_ref()
            .is_some_and(theme_has_floating_surface)
}

fn sync_window_state_timer(hwnd: HWND) {
    let required = lock_state()
        .as_ref()
        .is_some_and(window_state_timer_required);
    set_window_state_timer(hwnd, required);
}

fn set_window_state_timer(hwnd: HWND, required: bool) {
    {
        let mut state = lock_state();
        let Some(state) = state.as_mut() else {
            return;
        };
        if required == state.window_state_timer_active {
            return;
        }
        state.window_state_timer_active = required;
    }
    unsafe {
        if required {
            SetTimer(
                Some(hwnd),
                TIMER_WINDOW_STATE,
                WINDOW_STATE_INTERVAL_MS,
                None,
            );
        } else {
            let _ = KillTimer(Some(hwnd), TIMER_WINDOW_STATE);
        }
    }
}

fn save_state_settings() {
    let state = lock_state();
    if let Some(s) = state.as_ref() {
        let mut persisted = load_settings();
        persisted.tray_offset = s.tray_offset;
        persisted.taskbar_index = s.taskbar_index;
        persisted.legacy_placement_pending = false;
        persisted.widget_visible = true;
        persisted.legacy_visibility_pending = false;
        persisted.poll_interval_ms = s.poll_interval_ms;
        persisted.language = s
            .language_override
            .map(|language| language.code().to_string());
        persisted.last_update_check_unix = s.last_update_check_unix;
        persisted.set_enabled_providers(s.providers);
        persisted.custom_theme_enabled = s.custom_theme_enabled;
        persisted.active_theme_path = s
            .active_theme_path
            .as_ref()
            .map(|path| path.to_string_lossy().to_string());
        persisted.placement_override = s.placement_override.clone();
        persisted.floating_card_opacity = s.floating_card_opacity;
        // 모니터는 자신의 치수(dimensions)를 소유하므로, 모니터 액션이 설정을
        // 유지(persist)할 때 새로 로드된 값들을 변경하지 않고 그대로 둡니다.
        let _ = save_settings(&persisted);
    }
}

fn save_settings_or_log(settings: &SettingsFile, _context: &str) {
    let _ = save_settings(settings);
}

fn tray_usage_summary_lines(
    data: &AppUsageData,
    providers: ProviderSet,
    language: LanguageId,
    countdown: bool,
) -> Vec<String> {
    let strings = language.strings();
    let shown = |percentage: f64| UsageData::shown(percentage, countdown);
    providers
        .iter()
        .filter_map(|provider| {
            let usage = data.get(provider)?;
            let descriptor = provider.descriptor();
            let weekly_label = usage
                .weekly_label
                .as_deref()
                .unwrap_or(strings.weekly_window);
            Some(format!(
                "{} {}: {:.0}% | {}: {:.0}%",
                match data.selected_account_name(provider) {
                    Some(name) => format!("{} ({name})", language.text(descriptor.display_name)),
                    None => language.text(descriptor.display_name).to_string(),
                },
                strings.session_window,
                shown(usage.session.percentage),
                weekly_label,
                shown(usage.weekly.percentage),
            ))
        })
        .collect()
}

fn tray_usage_summary_from_state() -> Option<String> {
    let state = lock_state();
    let state = state.as_ref()?;
    if !state.last_poll_ok {
        return None;
    }
    let lines = tray_usage_summary_lines(
        state.data.as_ref()?,
        state.providers,
        state.language,
        state.usage_countdown,
    );
    (!lines.is_empty()).then(|| lines.join("\n"))
}

fn tray_icon_tooltip_from_state() -> String {
    tray_usage_summary_from_state().unwrap_or_else(|| {
        lock_state()
            .as_ref()
            .map(|state| state.language.strings().window_title.to_string())
            .unwrap_or_else(|| "AI Usage Monitor".to_string())
    })
}

fn tray_usage_summary_for_provider(
    provider: ProviderId,
    data: Option<&AppUsageData>,
    language: LanguageId,
    countdown: bool,
) -> Option<String> {
    let data = data?;
    let usage = data.get(provider)?;
    let strings = language.strings();
    let shown = |percentage: f64| UsageData::shown(percentage, countdown);
    let descriptor = provider.descriptor();
    let weekly_label = usage
        .weekly_label
        .as_deref()
        .unwrap_or(strings.weekly_window);
    Some(format!(
        "{} {}: {:.0}% | {}: {:.0}%",
        match data.selected_account_name(provider) {
            Some(name) => format!("{} ({name})", language.text(descriptor.display_name)),
            None => language.text(descriptor.display_name).to_string(),
        },
        strings.session_window,
        shown(usage.session.percentage),
        weekly_label,
        shown(usage.weekly.percentage),
    ))
}

fn sync_tray_icon(hwnd: HWND) {
    let settings = load_settings();
    let themed = {
        let state = lock_state();
        state.as_ref().and_then(|state| {
            effective_theme_from_state(state)
                .map(|theme| (theme, state.data.clone(), theme_runtime_from_state(state)))
        })
    };

    // 커스텀 테마를 사용 중이고 내장된(builtin) 클래식 테마가 아니라면, 커스텀 테마의 트레이 표면 설정을 존중합니다.
    if let Some((theme, data, runtime)) = themed.as_ref() {
        if !theme.is_builtin_classic() {
            let has_tray_surfaces = theme.surfaces.iter().any(|surface| {
                surface
                    .placement
                    .nest
                    .resolve(surface.placement.reference.region)
                    == SurfaceNest::TrayIcon
            });
            if has_tray_surfaces {
                let usage_tooltip = tray_usage_summary_from_state();
                let icons = theme
                    .surfaces
                    .iter()
                    .enumerate()
                    .filter(|(surface_index, surface)| {
                        let surface_runtime =
                            theme_runtime_for_surface(theme, *surface_index, *runtime);
                        surface
                            .placement
                            .nest
                            .resolve(surface.placement.reference.region)
                            == SurfaceNest::TrayIcon
                            && theme_engine::surface_should_render(
                                theme,
                                *surface_index,
                                data.as_ref(),
                                surface_runtime,
                            )
                    })
                    .filter_map(|(surface_index, surface)| {
                        let surface_runtime = theme_runtime_for_surface(theme, surface_index, *runtime);
                        let (logical_width, logical_height) = theme_engine::resolve_surface_size(
                            theme,
                            surface_index,
                            data.as_ref(),
                            surface_runtime,
                        );
                        let max_dimension = logical_width.max(logical_height) as f64;
                        let scale =
                            theme_surface_scale(theme, surface_index).min(if max_dimension > 0.0 {
                                512.0 / max_dimension
                            } else {
                                1.0
                            });
                        if scale < 0.25 {
                            return None;
                        }
                        let rendered = theme_engine::render_theme_surface_with_runtime_at_scale(
                            theme,
                            surface_index,
                            data.as_ref(),
                            surface_runtime,
                            scale,
                        );
                        Some(tray_icon::ThemedTrayIcon {
                            surface_index,
                            tooltip: usage_tooltip
                                .clone()
                                .unwrap_or_else(|| surface.name.clone()),
                            width: rendered.width,
                            height: rendered.height,
                            pixels: rendered.pixels,
                        })
                    })
                    .collect::<Vec<_>>();
                tray_icon::sync_themed(hwnd, &icons);
                return;
            }
        }
    }

    // 기본(Default) / 클래식(Classic) 동작: 활성화된 공급자들을 위해 모던한 동심원 링 형태의 배지를 렌더링합니다.
    let (data, language) = {
        let state = lock_state();
        let data = state.as_ref().and_then(|s| s.data.clone());
        let language = state
            .as_ref()
            .map(|s| s.language)
            .unwrap_or_else(localization::detect_system_language);
        (data, language)
    };

    let tray_size = unsafe {
        let dpi = CURRENT_DPI.load(Ordering::Relaxed);
        let sm = if dpi > 0 {
            GetSystemMetricsForDpi(SM_CXSMICON, dpi)
        } else {
            GetSystemMetrics(SM_CXSMICON)
        };
        (sm as u32).max(32)
    };

    let ordered_providers = settings.ordered_providers();
    let ring_icons: Vec<tray_icon::ThemedTrayIcon> = ordered_providers
        .into_iter()
        .filter(|&provider_id| settings.provider_enabled(provider_id))
        .enumerate()
        .map(|(idx, provider_id)| {
            let p_usage = data.as_ref().and_then(|d| d.get(provider_id));
            let ring_img = crate::platform::ring_badge::render_single_provider_ring(
                provider_id,
                p_usage,
                &settings,
                tray_size,
            );
            let bgra = crate::platform::ring_badge::rgba_to_bgra_premultiplied(&ring_img);
            let tooltip = tray_usage_summary_for_provider(
                provider_id,
                data.as_ref(),
                language,
                settings.usage_countdown,
            ).unwrap_or_else(|| provider_id.descriptor().display_name.to_string());

            tray_icon::ThemedTrayIcon {
                surface_index: idx + 1,
                tooltip,
                width: ring_img.width(),
                height: ring_img.height(),
                pixels: bgra,
            }
        })
        .collect();

    if !ring_icons.is_empty() {
        tray_icon::sync_themed(hwnd, &ring_icons);
        return;
    }

    tray_icon::sync(hwnd, &tray_icon_tooltip_from_state());
}

fn theme_tray_uses_current_time(theme: &ThemeDocument) -> bool {
    theme
        .surfaces
        .iter()
        .enumerate()
        .filter(|(_, surface)| {
            surface
                .placement
                .nest
                .resolve(surface.placement.reference.region)
                == SurfaceNest::TrayIcon
        })
        .any(|(surface_index, _)| {
            theme
                .surface_current_time_refresh_interval(surface_index)
                .is_some()
        })
}

fn taskbar_created_message() -> u32 {
    static MESSAGE: OnceLock<u32> = OnceLock::new();
    *MESSAGE.get_or_init(|| unsafe {
        let name = native_interop::wide_str("TaskbarCreated");
        RegisterWindowMessageW(PCWSTR::from_raw(name.as_ptr()))
    })
}

fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn update_check_interval() -> Duration {
    Duration::from_secs(updater::AUTO_UPDATE_CHECK_INTERVAL_SECS)
}

fn auto_update_check_due(last_update_check_unix: Option<u64>) -> bool {
    let Some(last_update_check_unix) = last_update_check_unix else {
        return true;
    };

    now_unix_secs().saturating_sub(last_update_check_unix) >= update_check_interval().as_secs()
}

fn schedule_auto_update_check(hwnd: HWND) {
    let delay_ms = {
        let state = lock_state();
        let Some(s) = state.as_ref() else {
            return;
        };

        if auto_update_check_due(s.last_update_check_unix) {
            None
        } else {
            let elapsed = now_unix_secs().saturating_sub(s.last_update_check_unix.unwrap_or(0));
            let remaining_secs = update_check_interval().as_secs().saturating_sub(elapsed);
            Some((remaining_secs.saturating_mul(1000)).min(u32::MAX as u64) as u32)
        }
    };

    unsafe {
        let _ = KillTimer(Some(hwnd), TIMER_UPDATE_CHECK);
        if let Some(delay_ms) = delay_ms {
            SetTimer(Some(hwnd), TIMER_UPDATE_CHECK, delay_ms.max(1), None);
        }
    }
}

fn set_window_title(hwnd: HWND, strings: Strings) {
    unsafe {
        let title = native_interop::wide_str(strings.window_title);
        let _ = SetWindowTextW(hwnd, PCWSTR::from_raw(title.as_ptr()));
    }
}

fn show_info_message(hwnd: HWND, title: &str, message: &str) {
    unsafe {
        let title_wide = native_interop::wide_str(title);
        let message_wide = native_interop::wide_str(message);
        let _ = MessageBoxW(
            Some(hwnd),
            PCWSTR::from_raw(message_wide.as_ptr()),
            PCWSTR::from_raw(title_wide.as_ptr()),
            MB_OK | MB_ICONINFORMATION,
        );
    }
}

fn show_error_message(hwnd: HWND, title: &str, message: &str) {
    unsafe {
        let title_wide = native_interop::wide_str(title);
        let message_wide = native_interop::wide_str(message);
        let _ = MessageBoxW(
            Some(hwnd),
            PCWSTR::from_raw(message_wide.as_ptr()),
            PCWSTR::from_raw(title_wide.as_ptr()),
            MB_OK | MB_ICONERROR,
        );
    }
}

fn show_update_prompt(hwnd: HWND, strings: Strings, release: &ReleaseDescriptor) -> bool {
    let message = strings
        .update_prompt_now
        .replace("{version}", &release.latest_version);

    unsafe {
        let title_wide = native_interop::wide_str(strings.update_available);
        let message_wide = native_interop::wide_str(&message);
        MessageBoxW(
            Some(hwnd),
            PCWSTR::from_raw(message_wide.as_ptr()),
            PCWSTR::from_raw(title_wide.as_ptr()),
            MB_YESNO | MB_ICONQUESTION,
        ) == IDYES
    }
}

fn apply_language_to_state(state: &mut AppState, language_override: Option<LanguageId>) {
    state.language_override = language_override;
    state.language = localization::resolve_language(language_override);
    set_window_title(state.hwnd.to_hwnd(), state.language.strings());
}

fn update_language_change() -> bool {
    let mut state = lock_state();
    let Some(app_state) = state.as_mut() else {
        return false;
    };

    if app_state.language_override.is_some() {
        return false;
    }

    let new_language = localization::detect_system_language();
    if new_language == app_state.language {
        return false;
    }

    apply_language_to_state(app_state, None);
    true
}

fn begin_update_check(hwnd: HWND, interactive: bool) {
    let send_hwnd = SendHwnd::from_hwnd(hwnd);
    let strings = {
        let mut state = lock_state();
        let Some(app_state) = state.as_mut() else {
            return;
        };

        if matches!(
            app_state.update_status,
            UpdateStatus::Checking | UpdateStatus::Applying
        ) {
            if interactive {
                show_info_message(
                    hwnd,
                    app_state.language.strings().updates,
                    app_state.language.strings().update_in_progress,
                );
            }
            return;
        }

        app_state.update_status = UpdateStatus::Checking;
        app_state.language.strings()
    };

    std::thread::spawn(move || {
        let hwnd = send_hwnd.to_hwnd();
        let checked_at = now_unix_secs();
        match updater::check_for_updates() {
            Ok(UpdateCheckResult::UpToDate) => {
                {
                    let mut state = lock_state();
                    if let Some(s) = state.as_mut() {
                        s.update_status = UpdateStatus::UpToDate;
                        s.last_update_check_unix = Some(checked_at);
                    }
                }
                save_state_settings();
                if interactive {
                    show_info_message(hwnd, strings.updates, strings.up_to_date);
                }
                unsafe {
                    let _ = PostMessageW(
                        Some(hwnd),
                        WM_APP_UPDATE_CHECK_COMPLETE,
                        WPARAM(0),
                        LPARAM(0),
                    );
                }
            }
            Ok(UpdateCheckResult::Available(release)) => {
                {
                    let mut state = lock_state();
                    if let Some(s) = state.as_mut() {
                        s.update_status = UpdateStatus::Available(release.clone());
                        s.last_update_check_unix = Some(checked_at);
                    }
                }
                save_state_settings();
                if interactive && show_update_prompt(hwnd, strings, &release) {
                    begin_update_apply(hwnd, release);
                }
                unsafe {
                    let _ = PostMessageW(
                        Some(hwnd),
                        WM_APP_UPDATE_CHECK_COMPLETE,
                        WPARAM(0),
                        LPARAM(0),
                    );
                }
            }
            Err(error) => {
                {
                    let mut state = lock_state();
                    if let Some(s) = state.as_mut() {
                        s.update_status = UpdateStatus::Idle;
                        s.last_update_check_unix = Some(checked_at);
                    }
                }
                save_state_settings();
                if interactive {
                    let message = format!("{}.\n\n{}", strings.update_failed, error);
                    show_error_message(hwnd, strings.updates, &message);
                }
                unsafe {
                    let _ = PostMessageW(
                        Some(hwnd),
                        WM_APP_UPDATE_CHECK_COMPLETE,
                        WPARAM(0),
                        LPARAM(0),
                    );
                }
            }
        }
    });
}

fn begin_update_apply(hwnd: HWND, release: ReleaseDescriptor) {
    let send_hwnd = SendHwnd::from_hwnd(hwnd);
    let strings = {
        let mut state = lock_state();
        let Some(app_state) = state.as_mut() else {
            return;
        };

        if matches!(
            app_state.update_status,
            UpdateStatus::Checking | UpdateStatus::Applying
        ) {
            show_info_message(
                hwnd,
                app_state.language.strings().updates,
                app_state.language.strings().update_in_progress,
            );
            return;
        }

        app_state.update_status = UpdateStatus::Applying;
        app_state.language.strings()
    };

    std::thread::spawn(move || {
        let hwnd = send_hwnd.to_hwnd();
        match updater::begin_self_update(&release) {
            Ok(()) => unsafe {
                let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
            },
            Err(error) => {
                {
                    let mut state = lock_state();
                    if let Some(s) = state.as_mut() {
                        s.update_status = UpdateStatus::Available(release);
                    }
                }
                let message = format!("{}.\n\n{}", strings.update_failed, error);
                show_error_message(hwnd, strings.updates, &message);
                unsafe {
                    let _ = PostMessageW(
                        Some(hwnd),
                        WM_APP_UPDATE_CHECK_COMPLETE,
                        WPARAM(0),
                        LPARAM(0),
                    );
                }
            }
        }
    });
}

const STARTUP_REGISTRY_PATH: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const STARTUP_REGISTRY_KEY: &str = "AIUsage";

/// 시작프로그램 레지스트리 값이 현재 실행 파일(executable)을 가리킬 때만 true를 반환합니다.
pub(crate) fn is_startup_enabled() -> bool {
    unsafe {
        let path = native_interop::wide_str(STARTUP_REGISTRY_PATH);
        let key_name = native_interop::wide_str(STARTUP_REGISTRY_KEY);

        let mut hkey = HKEY::default();
        let result = RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR::from_raw(path.as_ptr()),
            None,
            KEY_READ,
            &mut hkey,
        );
        if result.is_err() {
            return false;
        }

        // 값의 크기를 쿼리(요청)합니다.
        let mut data_size: u32 = 0;
        let result = RegQueryValueExW(
            hkey,
            PCWSTR::from_raw(key_name.as_ptr()),
            None,
            None,
            None,
            Some(&mut data_size),
        );
        if result.is_err() || data_size == 0 {
            let _ = RegCloseKey(hkey);
            return false;
        }

        // 값을 읽어옵니다.
        let mut buf = vec![0u8; data_size as usize];
        let result = RegQueryValueExW(
            hkey,
            PCWSTR::from_raw(key_name.as_ptr()),
            None,
            None,
            Some(buf.as_mut_ptr()),
            Some(&mut data_size),
        );
        let _ = RegCloseKey(hkey);
        if result.is_err() {
            return false;
        }

        // 레지스트리 값(UTF-16)을 문자열로 변환합니다.
        let wide_slice =
            std::slice::from_raw_parts(buf.as_ptr() as *const u16, data_size as usize / 2);
        let reg_value = String::from_utf16_lossy(wide_slice)
            .trim_end_matches('\0')
            .to_string();

        // 현재 실행 파일(executable)의 경로를 가져옵니다.
        let mut exe_buf = [0u16; 260];
        let len = GetModuleFileNameW(None, &mut exe_buf) as usize;
        if len == 0 {
            return false;
        }
        let current_exe = String::from_utf16_lossy(&exe_buf[..len]);

        // 대소문자를 구분하지 않고 비교합니다 (Windows 경로는 대소문자를 구분하지 않음).
        reg_value.eq_ignore_ascii_case(&current_exe)
    }
}

pub(crate) fn set_startup_enabled(enable: bool) {
    unsafe {
        let path = native_interop::wide_str(STARTUP_REGISTRY_PATH);

        let mut hkey = HKEY::default();
        let result = RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR::from_raw(path.as_ptr()),
            None,
            KEY_SET_VALUE,
            &mut hkey,
        );
        if result.is_err() {
            return;
        }

        let key_name = native_interop::wide_str(STARTUP_REGISTRY_KEY);

        if enable {
            let mut exe_buf = [0u16; 260];
            let len = GetModuleFileNameW(None, &mut exe_buf) as usize;
            if len > 0 {
                // 널 종단 문자(null terminator)를 포함하여 와이드 문자열을 씁니다.
                let byte_len = ((len + 1) * 2) as u32;
                let _ = RegSetValueExW(
                    hkey,
                    PCWSTR::from_raw(key_name.as_ptr()),
                    None,
                    REG_SZ,
                    Some(std::slice::from_raw_parts(
                        exe_buf.as_ptr() as *const u8,
                        byte_len as usize,
                    )),
                );
            }
        } else {
            let _ = RegDeleteValueW(hkey, PCWSTR::from_raw(key_name.as_ptr()));
        }

        let _ = RegCloseKey(hkey);
    }
}

fn total_widget_width_for_state(state: &AppState) -> i32 {
    widget_frame_for_state(state, None).width
}

fn widget_frame_for_state(state: &AppState, nest: Option<SurfaceNest>) -> positioning::WidgetFrame {
    if state.taskbar_ring_badge && matches!(nest, None | Some(SurfaceNest::Taskbar)) {
        let count = state.providers.iter().count().max(1) as i32;
        let ring_size = 40;
        let gap = 4;
        let width = ring_size * count + gap * (count - 1);
        return positioning::WidgetFrame {
            width,
            height: 46,
            content_width: width,
            inset: 0,
        };
    }
    effective_theme_from_state(state).as_ref().map_or(
        positioning::WidgetFrame {
            width: 1,
            height: 1,
            content_width: 1,
            inset: 0,
        },
        |theme| {
            let runtime = theme_runtime_for_surface(theme, 0, theme_runtime_from_state(state));
            let runtime = nest.map_or(runtime, |nest| runtime.with_nest(nest));
            let scale = theme_surface_scale(theme, 0);
            positioning::widget_frame(theme, state.data.as_ref(), runtime, scale)
        },
    )
}

fn apply_custom_theme(
    hwnd: HWND,
    _enabled: bool,
    path: Option<PathBuf>,
    document: Option<ThemeDocument>,
) -> Result<(), String> {
    let loaded = match (document, path.as_deref()) {
        (Some(document), _) => Some(document),
        (None, Some(path)) => Some(theme_engine::load_theme(path)?),
        (None, None) => lock_state()
            .as_ref()
            .and_then(|state| state.active_theme.clone()),
    };
    let loaded = loaded.unwrap_or_else(ThemeDocument::starter);
    let theme_clock_interval = loaded.current_time_refresh_interval();
    let tray_theme_uses_current_time = theme_tray_uses_current_time(&loaded);
    let old_hook = {
        let mut state = lock_state();
        let Some(state) = state.as_mut() else {
            return Err("Application is not ready".into());
        };
        state.custom_theme_enabled = true;
        state.active_theme = Some(loaded);
        state.theme_clock_interval = theme_clock_interval;
        state.tray_theme_uses_current_time = tray_theme_uses_current_time;
        state.mouse_action_overrides.clear();
        state.hovered_mouse_layer = None;
        state.pending_mouse_click = None;
        state.suppress_next_left_up = false;
        if path.is_some() {
            state.active_theme_path = path;
        }
        state.embedded = false;
        state.win_event_hook.take()
    };
    if let Some(hook) = old_hook {
        native_interop::unhook_win_event(hook.to_hook());
    }
    unsafe {
        native_interop::make_popup(hwnd, false);
        ensure_layered_window(hwnd);
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_NOTOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        );
    }
    sync_custom_mirrors();
    sync_window_state_timer(hwnd);
    schedule_countdown_timer();
    schedule_clock_timer();
    Ok(())
}

fn sync_custom_mirrors() {
    let (desired_total, desktop_surfaces) = {
        let state = lock_state();
        state
            .as_ref()
            .map(|state| {
                let surfaces = state
                    .active_theme
                    .as_ref()
                    .map(|theme| {
                        theme
                            .surfaces
                            .iter()
                            .map(|surface| {
                                surface
                                    .placement
                                    .nest
                                    .resolve(surface.placement.reference.region)
                                    == SurfaceNest::Desktop
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                (surfaces.len().max(1), surfaces)
            })
            .unwrap_or_else(|| (1, Vec::new()))
    };
    let desired_mirrors = desired_total.saturating_sub(1);
    loop {
        let remove = {
            let mut state = lock_state();
            state.as_mut().and_then(|state| {
                if state.mirror_hwnds.len() > desired_mirrors {
                    state.mirror_hwnds.pop()
                } else {
                    None
                }
            })
        };
        match remove {
            Some(hwnd) => unsafe {
                let _ = DestroyWindow(hwnd.to_hwnd());
            },
            None => break,
        }
    }
    while lock_state()
        .as_ref()
        .map(|state| state.mirror_hwnds.len())
        .unwrap_or(0)
        < desired_mirrors
    {
        let mirror = unsafe { create_mirror_window() };
        if mirror.is_invalid() {
            break;
        }
        if let Some(state) = lock_state().as_mut() {
            state.mirror_hwnds.push(SendHwnd::from_hwnd(mirror));
        }
    }

    let stale_desktop_windows = {
        let mut state = lock_state();
        let Some(state) = state.as_mut() else {
            return;
        };
        state.desktop_hwnds.resize_with(desired_total, || None);
        let removed = state.desktop_hwnds.split_off(desired_total);
        let mut stale = removed.into_iter().flatten().collect::<Vec<_>>();
        for (surface_index, window) in state.desktop_hwnds.iter_mut().enumerate() {
            let wanted = desktop_surfaces.get(surface_index) == Some(&true);
            let valid =
                window.is_some_and(|window| unsafe { IsWindow(Some(window.to_hwnd())).as_bool() });
            if !wanted || !valid {
                if let Some(window) = window.take() {
                    stale.push(window);
                }
            }
        }
        stale
    };
    for window in stale_desktop_windows {
        unsafe {
            let _ = DestroyWindow(window.to_hwnd());
        }
    }
    for (surface_index, wanted) in desktop_surfaces.into_iter().enumerate() {
        if !wanted {
            continue;
        }
        let missing = lock_state()
            .as_ref()
            .and_then(|state| state.desktop_hwnds.get(surface_index))
            .is_none_or(Option::is_none);
        if !missing {
            continue;
        }
        let window = unsafe { create_desktop_surface_window() };
        if window.is_invalid() {
            continue;
        }
        unsafe {
            let _ = ShowWindow(window, SW_HIDE);
        }
        if let Some(slot) = lock_state()
            .as_mut()
            .and_then(|state| state.desktop_hwnds.get_mut(surface_index))
        {
            *slot = Some(SendHwnd::from_hwnd(window));
        } else {
            unsafe {
                let _ = DestroyWindow(window);
            }
        }
    }
}

unsafe fn create_desktop_surface_window() -> HWND {
    let Some(desktop) = native_interop::find_desktop_host() else {
        return HWND::default();
    };
    let instance = GetModuleHandleW(PCWSTR::null()).unwrap();
    let class = native_interop::wide_str("AIUsageDesktopSurface");
    let title = native_interop::wide_str("");
    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style: CS_DBLCLKS,
        lpfnWndProc: Some(mirror_wnd_proc),
        hInstance: HINSTANCE(instance.0),
        hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
        hbrBackground: HBRUSH::default(),
        lpszClassName: PCWSTR::from_raw(class.as_ptr()),
        ..Default::default()
    };
    RegisterClassExW(&wc);
    let previous_hosting = SetThreadDpiHostingBehavior(DPI_HOSTING_BEHAVIOR_MIXED);
    let previous_dpi = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_UNAWARE);
    let window = CreateWindowExW(
        WS_EX_NOREDIRECTIONBITMAP | WS_EX_NOACTIVATE,
        PCWSTR::from_raw(class.as_ptr()),
        PCWSTR::from_raw(title.as_ptr()),
        WINDOW_STYLE(
            native_interop::WS_CHILD_STYLE | native_interop::WS_CLIPSIBLINGS_STYLE | WS_VISIBLE.0,
        ),
        0,
        0,
        198,
        144,
        Some(desktop.parent),
        None,
        Some(HINSTANCE(instance.0)),
        None,
    )
    .unwrap_or_default();
    let _ = SetThreadDpiAwarenessContext(previous_dpi);
    let _ = SetThreadDpiHostingBehavior(previous_hosting);
    window
}

unsafe fn create_mirror_window() -> HWND {
    let instance = GetModuleHandleW(PCWSTR::null()).unwrap();
    let class = native_interop::wide_str("AIUsageThemeMirror");
    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style: CS_DBLCLKS,
        lpfnWndProc: Some(mirror_wnd_proc),
        hInstance: HINSTANCE(instance.0),
        hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
        hbrBackground: HBRUSH::default(),
        lpszClassName: PCWSTR::from_raw(class.as_ptr()),
        ..Default::default()
    };
    RegisterClassExW(&wc);
    let title = native_interop::wide_str("Usage theme mirror");
    CreateWindowExW(
        WS_EX_TOOLWINDOW | WS_EX_LAYERED | WS_EX_NOACTIVATE,
        PCWSTR::from_raw(class.as_ptr()),
        PCWSTR::from_raw(title.as_ptr()),
        WS_POPUP,
        0,
        0,
        1,
        1,
        None,
        None,
        Some(HINSTANCE(instance.0)),
        None,
    )
    .unwrap_or_default()
}

unsafe extern "system" fn mirror_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_NCHITTEST => LRESULT(HTCLIENT as isize),
        WM_SETCURSOR if set_surface_cursor(hwnd) => LRESULT(1),
        WM_MOUSEMOVE => {
            update_mouse_hover(hwnd, lparam);
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            clear_mouse_hover(hwnd);
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let suppressed = {
                let mut state = lock_state();
                state.as_mut().is_some_and(|state| {
                    let suppressed = state.suppress_next_left_up;
                    state.suppress_next_left_up = false;
                    suppressed
                })
            };
            if !suppressed {
                if let Some((surface, object)) = mouse_target_at(hwnd, lparam) {
                    schedule_or_dispatch_click(hwnd, surface, object);
                }
            }
            LRESULT(0)
        }
        WM_LBUTTONDBLCLK => {
            if let Some((surface, object)) = mouse_target_at(hwnd, lparam) {
                dispatch_double_click(hwnd, surface, object);
            }
            LRESULT(0)
        }
        WM_RBUTTONUP => {
            if let Some((surface, object)) = mouse_target_at(hwnd, lparam) {
                let _ = dispatch_mouse_event(surface, &object, MouseEventKind::RightClick);
            }
            LRESULT(0)
        }
        WM_PAINT => {
            let mut paint = PAINTSTRUCT::default();
            let _ = BeginPaint(hwnd, &mut paint);
            let _ = EndPaint(hwnd, &paint);
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_DESTROY => {
            crate::desktop_compositor::remove(hwnd);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn total_widget_height_for_state(state: &AppState) -> i32 {
    widget_frame_for_state(state, None).height
}

fn total_widget_height() -> i32 {
    lock_state()
        .as_ref()
        .map(total_widget_height_for_state)
        .unwrap_or(1)
}

fn total_widget_width() -> i32 {
    lock_state()
        .as_ref()
        .map(total_widget_width_for_state)
        .unwrap_or(1)
}

struct RunOptions {
    allow_multiple: bool,
    no_poll: bool,
}

fn parse_run_args(args: &[String]) -> RunOptions {
    RunOptions {
        allow_multiple: args.iter().any(|argument| argument == "--allow-multiple"),
        no_poll: args.iter().any(|argument| argument == "--no-poll"),
    }
}

/// 단일 인스턴스(Single-instance) 가드: 전역 뮤텍스를 획득하며, 재시작 시 
/// 이전 인스턴스와 경쟁 상태가 발생하면 잠시 대기합니다. 조용히 종료하려면 None을 반환합니다.
fn acquire_single_instance_mutex(allow_multiple: bool) -> Option<HANDLE> {
    let is_relaunch = std::env::var(ENV_RELAUNCH).is_ok();
    let mutex_name = native_interop::wide_str(&if allow_multiple {
        format!("Global\\AIUsage-{}", std::process::id())
    } else {
        "Global\\AIUsage".to_string()
    });
    unsafe {
        let handle = CreateMutexW(None, true, PCWSTR::from_raw(mutex_name.as_ptr()));
        match handle {
            Ok(h) => {
                let err = GetLastError();
                if err == ERROR_ALREADY_EXISTS {
                    if is_relaunch {
                        let wait_result = WaitForSingleObject(h, 10_000);
                        if wait_result != WAIT_OBJECT_0 && wait_result != WAIT_ABANDONED {
                            return None;
                        }
                    } else {
                        return None;
                    }
                }
                Some(h)
            }
            Err(_) => None,
        }
    }
}

struct WindowClassResources {
    hinstance: HINSTANCE,
    class_name: Vec<u16>,
    large_icon: HICON,
    small_icon: HICON,
}

fn register_app_window_class() -> WindowClassResources {
    let class_name = native_interop::wide_str("AIUsage");
    unsafe {
        let hinstance = GetModuleHandleW(PCWSTR::null()).unwrap();
        let (large_icon, small_icon) = tray_icon::load_app_icons();
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW | CS_DBLCLKS,
            lpfnWndProc: Some(wnd_proc),
            hInstance: HINSTANCE(hinstance.0),
            hIcon: large_icon,
            hIconSm: small_icon,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hbrBackground: HBRUSH(std::ptr::null_mut()),
            lpszClassName: PCWSTR::from_raw(class_name.as_ptr()),
            ..Default::default()
        };
        let _ = RegisterClassExW(&wc);
        WindowClassResources {
            hinstance: HINSTANCE(hinstance.0),
            class_name,
            large_icon,
            small_icon,
        }
    }
}

/// 첫 번째 윈도우가 생성되기 전에 확인(resolve)된 설정, 테마 및 언어 정보입니다.
struct StartupConfig {
    settings: SettingsFile,
    active_theme_path: Option<PathBuf>,
    active_theme: Option<ThemeDocument>,
    custom_theme_enabled: bool,
    theme_clock_interval: Option<Duration>,
    tray_theme_uses_current_time: bool,
    language_override: Option<LanguageId>,
    language: LanguageId,
}

fn resolve_startup_config() -> StartupConfig {
    let mut settings = load_settings();
    let classic_theme_path = theme_engine::ensure_starter_theme().ok();
    let mut configured_theme_path = settings.active_theme_path.as_deref().map(PathBuf::from);
    let mut configured_theme = configured_theme_path
        .as_deref()
        .and_then(|path| theme_engine::load_theme(path).ok());
    let legacy_placement = settings.legacy_placement();
    let legacy_visibility = settings.legacy_widget_visibility();
    if legacy_placement.is_some() || legacy_visibility.is_some() {
        if configured_theme
            .as_ref()
            .is_some_and(|theme| !theme.is_builtin_classic())
        {
            // 사용자가 선택한 쓰기 가능한 테마는 이미 자체적인 표현 방식(presentation)을 가지고 있습니다.
            // 해당 테마를 덮어쓰지 않고 더 이상 쓰이지 않는(obsolete) 설정값들을 소비합니다.
            settings.consume_legacy_placement();
            settings.consume_legacy_widget_visibility();
            save_settings_or_log(&settings, "unable to consume legacy settings");
        } else if legacy_placement.is_some() || legacy_visibility == Some(false) {
            let placement = legacy_placement.map(migrated_theme_placement);
            let migrated =
                ThemeDocument::migrated_from_legacy(placement, legacy_visibility.unwrap_or(true));
            match theme_engine::save_theme(&migrated) {
                Ok(path) => {
                    configured_theme_path = Some(path.clone());
                    configured_theme = Some(migrated);
                    settings.active_theme_path = Some(path.to_string_lossy().into_owned());
                    settings.custom_theme_enabled = true;
                    settings.consume_legacy_placement();
                    settings.consume_legacy_widget_visibility();
                    let _ = save_settings(&settings);
                }
                Err(_) => {}
            }
        } else {
            // 명시적으로 표시되던 v1.4.9 위젯은 이미 내장 테마의
            // Render 값과 일치하므로 복사본을 생성할 필요가 없습니다.
            settings.consume_legacy_widget_visibility();
            save_settings_or_log(&settings, "unable to consume legacy visibility");
        }
    }
    let (active_theme_path, active_theme) = configured_theme
        .map(|theme| (configured_theme_path, Some(theme)))
        .unwrap_or_else(|| {
            let path = classic_theme_path;
            let theme = path
                .as_deref()
                .and_then(|path| theme_engine::load_theme(path).ok())
                .or_else(|| Some(ThemeDocument::starter()));
            (path, theme)
        });
    let custom_theme_enabled = true;
    let theme_clock_interval = active_theme
        .as_ref()
        .and_then(ThemeDocument::current_time_refresh_interval);
    let tray_theme_uses_current_time = active_theme
        .as_ref()
        .is_some_and(theme_tray_uses_current_time);
    if let Some(path) = &active_theme_path {
        let path = path.to_string_lossy().into_owned();
        if settings.active_theme_path.as_deref() != Some(path.as_str())
            || !settings.custom_theme_enabled
        {
            settings.active_theme_path = Some(path);
            settings.custom_theme_enabled = true;
            save_settings_or_log(&settings, "unable to persist active theme");
        }
    }
    let language_override = settings.language.as_deref().and_then(LanguageId::from_code);
    let language = localization::resolve_language(language_override);
    StartupConfig {
        settings,
        active_theme_path,
        active_theme,
        custom_theme_enabled,
        theme_clock_interval,
        tray_theme_uses_current_time,
        language_override,
        language,
    }
}

fn initial_window_size(
    active_theme: &Option<ThemeDocument>,
    initial_runtime: ThemeRuntime,
) -> (i32, i32) {
    active_theme
        .as_ref()
        .map(|theme| {
            let initial_runtime = theme_runtime_for_surface(theme, 0, initial_runtime);
            let (width, height) =
                theme_engine::resolve_surface_size(theme, 0, None, initial_runtime);
            let scale = theme_surface_scale(theme, 0);
            (
                scaled_theme_dimension(width, scale),
                scaled_theme_dimension(height, scale),
            )
        })
        .unwrap_or((1, 1))
}

fn create_app_window(class: &WindowClassResources, title: &[u16], width: i32, height: i32) -> HWND {
    unsafe {
        CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_LAYERED | WS_EX_NOACTIVATE,
            PCWSTR::from_raw(class.class_name.as_ptr()),
            PCWSTR::from_raw(title.as_ptr()),
            WS_POPUP,
            0,
            0,
            width,
            height,
            None,
            None,
            Some(HINSTANCE(class.hinstance.0)),
            None,
        )
        .unwrap()
    }
}

fn apply_window_icons(hwnd: HWND, large_icon: HICON, small_icon: HICON) {
    unsafe {
        if !large_icon.is_invalid() {
            let _ = SendMessageW(
                hwnd,
                WM_SETICON,
                Some(WPARAM(ICON_BIG as usize)),
                Some(LPARAM(large_icon.0 as isize)),
            );
        }
        if !small_icon.is_invalid() {
            let _ = SendMessageW(
                hwnd,
                WM_SETICON,
                Some(WPARAM(ICON_SMALL as usize)),
                Some(LPARAM(small_icon.0 as isize)),
            );
        }
    }
}

fn build_initial_state(hwnd: HWND, is_dark: bool, config: &StartupConfig) -> AppState {
    let settings = &config.settings;
    AppState {
        hwnd: SendHwnd::from_hwnd(hwnd),
        taskbar_hwnd: None,
        tray_notify_hwnd: None,
        win_event_hook: None,
        is_dark,
        embedded: false,
        language_override: config.language_override,
        language: config.language,
        providers: settings.enabled_providers(),
        accounts: settings.accounts.clone(),
        data: None,
        poll_interval_ms: settings.poll_interval_ms,
        retry_count: 0,
        force_notify_auth_error: false,
        auth_error_paused_polling: false,
        auth_watch_mode: poller::CredentialWatchMode::ActiveSource(
            settings.enabled_providers().first().unwrap_or_default(),
        ),
        auth_watch_snapshot: Vec::new(),
        last_poll_ok: false,
        update_status: UpdateStatus::Idle,
        last_update_check_unix: settings.last_update_check_unix,
        taskbar_index: settings.taskbar_index,
        tray_offset: settings.tray_offset,
        dragging: false,
        pending_drag: false,
        drag_start_cursor: POINT::default(),
        drag_start_origin: POINT::default(),
        drag_start_client_x: 0,
        auto_ejected: false,
        auto_ejected_origin: None,
        is_switching_window_style: false,
        is_snapped: false,
        placement_override: settings.placement_override.clone(),
        floating_card_opacity: settings.floating_card_opacity,
        window_state_timer_active: false,
        custom_theme_enabled: config.custom_theme_enabled,
        usage_countdown: settings.usage_countdown,
        taskbar_ring_badge: settings.taskbar_ring_badge,
        active_theme_path: config.active_theme_path.clone(),
        active_theme: config.active_theme.clone(),
        theme_clock_interval: config.theme_clock_interval,
        tray_theme_uses_current_time: config.tray_theme_uses_current_time,
        mirror_hwnds: Vec::new(),
        desktop_hwnds: Vec::new(),
        mouse_action_overrides: HashMap::new(),
        hovered_mouse_layer: None,
        pending_mouse_click: None,
        suppress_next_left_up: false,
    }
}

fn run_startup_tasks(hwnd: HWND, no_poll: bool) {
    sync_custom_mirrors();
    native_interop::make_popup(hwnd, false);

    // 영속 애플리케이션 트레이 아이콘을 등록합니다.
    if !no_poll {
        sync_tray_icon(hwnd);
    }

    // 테마 서피스가 해당 윈도우의 렌더링 여부를 결정합니다.
    position_at_taskbar();

    // 서피스 중첩(nest)에 의해 선택된 프레젠터를 사용하여 최초 렌더링을 수행합니다.
    render_layered();
    schedule_countdown_timer();
    schedule_clock_timer();

    // 설정에 지정된 주기를 사용하는 폴링 타이머입니다.
    let initial_poll_ms = {
        let state = lock_state();
        state
            .as_ref()
            .map(|s| s.poll_interval_ms)
            .unwrap_or(POLL_15_MIN)
    };
    unsafe {
        SetTimer(Some(hwnd), TIMER_POLL, initial_poll_ms, None);
    }
    sync_window_state_timer(hwnd);

    // explorer.exe 재시작을 감시하여 트레이 아이콘을 다시 임베드하고 다시 추가할 수 있도록 합니다
    // (셸이 재시작되면 트레이 등록 정보가 폐기됩니다). 이것은 윈도우 타이머가 아닌 전용 스레드에서
    // 실행됩니다: 탐색기가 작업 표시줄을 파괴하면 임베드된 자식 윈도우는 모든 메시지(WM_TIMER 포함)
    // 수신을 중단하므로, 타이머로는 다시 동작할 수 없기 때문입니다.
    spawn_taskbar_watchdog();

    // 최초 폴링
    if !no_poll {
        request_poll(hwnd);
    }

    if !no_poll {
        schedule_auto_update_check(hwnd);
    }
    let should_check_updates = {
        let state = lock_state();
        state
            .as_ref()
            .map(|s| auto_update_check_due(s.last_update_check_unix))
            .unwrap_or(false)
    };
    if should_check_updates && !no_poll {
        begin_update_check(hwnd, false);
    }

    // 최초 테마 확인
    check_theme_change();
}

fn run_message_loop() {
    unsafe {
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

pub fn run() {
    let options = parse_run_args(&std::env::args().collect::<Vec<_>>());
    unsafe {
        let _ = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        CURRENT_DPI.store(GetDpiForSystem(), Ordering::Relaxed);
    }
    let Some(_mutex) = acquire_single_instance_mutex(options.allow_multiple) else {
        return;
    };

    let class = register_app_window_class();

    let config = resolve_startup_config();
    refresh_theme_host_geometry();

    // 레이어드 팝업으로 생성 (이후 작업 표시줄의 자식으로 부모 재지정됨)
    let title = native_interop::wide_str(config.language.strings().window_title);
    let initial_runtime = ThemeRuntime::from_providers(config.settings.enabled_providers())
        .with_poll_state(false, false)
        .with_language(config.language)
        .with_countdown(config.settings.usage_countdown);
    let (initial_width, initial_height) =
        initial_window_size(&config.active_theme, initial_runtime);
    let hwnd = create_app_window(&class, &title, initial_width, initial_height);
    apply_window_icons(hwnd, class.large_icon, class.small_icon);

    let is_dark = theme::is_dark_mode();
    {
        let mut state = lock_state();
        *state = Some(build_initial_state(hwnd, is_dark, &config));
    }

    run_startup_tasks(hwnd, options.no_poll);
    run_message_loop();
}

/// 모든 테마 서피스를 렌더링한 후, 중첩(nest)에 의해 선택된 프레젠터로 전달합니다:
/// 데스크톱의 경우 DirectComposition, 그 외는 레이어드 윈도우 방식을 사용합니다.
fn render_layered() {
    refresh_dpi();
    sync_custom_mirrors();
    let (hwnd_val, active_theme, usage_data, runtime, mirror_hwnds, desktop_hwnds, taskbar_ring_badge) = {
        let state = lock_state();
        let Some(state) = state.as_ref() else {
            return;
        };
        (
            state.hwnd,
            effective_theme_from_state(state),
            state.data.clone(),
            theme_runtime_from_state(state),
            state.mirror_hwnds.clone(),
            state.desktop_hwnds.clone(),
            state.taskbar_ring_badge,
        )
    };

    // 테마 렌더링은 위젯 렌더러입니다. 시작 시 및 테마 변경 시 선택한 테마를
    // 로드할 수 없으면 메모리에 항상 Classic 테마를 설정합니다.
    let theme = active_theme.unwrap_or_else(ThemeDocument::starter);
    let hwnd = hwnd_val.to_hwnd();
    set_window_state_timer(hwnd, theme_has_floating_surface(&theme));
    let target_count = theme.surfaces.len();
    for surface_index in 0..target_count {
        let regular_hwnd = if surface_index == 0 {
            hwnd
        } else if let Some(mirror) = mirror_hwnds.get(surface_index - 1) {
            mirror.to_hwnd()
        } else {
            continue;
        };
        let surface = &theme.surfaces[surface_index];
        let surface_runtime = theme_runtime_for_surface(&theme, surface_index, runtime);
        let nest = surface
            .placement
            .nest
            .resolve(surface.placement.reference.region);
        let desktop_nested = nest == SurfaceNest::Desktop;
        let target_hwnd = if desktop_nested {
            unsafe {
                let _ = ShowWindow(regular_hwnd, SW_HIDE);
            }
            desktop_hwnds
                .get(surface_index)
                .and_then(|window| *window)
                .map(SendHwnd::to_hwnd)
                .unwrap_or(regular_hwnd)
        } else {
            regular_hwnd
        };
        if nest == SurfaceNest::TrayIcon {
            unsafe {
                let _ = ShowWindow(target_hwnd, SW_HIDE);
            }
            continue;
        }
        if !theme_engine::surface_should_render(
            &theme,
            surface_index,
            usage_data.as_ref(),
            surface_runtime,
        ) {
            unsafe {
                let _ = ShowWindow(target_hwnd, SW_HIDE);
            }
            continue;
        }

        if surface_index == 0 && nest == SurfaceNest::Taskbar && taskbar_ring_badge {
            let settings = app_settings::load_settings();
            let default_data = AppUsageData::default();
            let data_ref = usage_data.as_ref().unwrap_or(&default_data);
            let scale = theme_surface_scale(&theme, 0);
            let target_ring_size = ((40.0 * scale).round() as u32).max(16);
            let target_gap = ((4.0 * scale).round() as u32).max(1);
            let target_canvas_height = ((46.0 * scale).round() as u32).max(target_ring_size);

            if let Some(ring_img) = crate::platform::ring_badge::render_ring_badge_image_at_size(
                data_ref,
                &settings,
                target_ring_size,
                target_gap,
            ) {
                let y_offset = (target_canvas_height.saturating_sub(ring_img.height())) / 2;
                let mut padded_img = image::RgbaImage::new(ring_img.width(), target_canvas_height);
                for y in 0..ring_img.height() {
                    for x in 0..ring_img.width() {
                        padded_img.put_pixel(x, y + y_offset, *ring_img.get_pixel(x, y));
                    }
                }
                let bgra = crate::platform::ring_badge::rgba_to_bgra_premultiplied(&padded_img);
                let rendered = theme_engine::RenderedTheme {
                    width: padded_img.width(),
                    height: target_canvas_height,
                    pixels: bgra,
                    warnings: Vec::new(),
                };
                let mut positioned = theme_for_surface(&theme, 0);
                positioned.canvas.width = (padded_img.width() as f64 / scale).round().max(1.0) as u32;
                positioned.canvas.height = (target_canvas_height as f64 / scale).round().max(1.0) as u32;
                let placement = theme_engine::resolve_surface_placement(
                    &theme,
                    0,
                    usage_data.as_ref(),
                    surface_runtime,
                );
                positioned.placement.offset_x = placement.offset_x;
                positioned.placement.offset_y = placement.offset_y;
                position_custom_theme(regular_hwnd, &positioned, scale);
                render_custom_window(regular_hwnd, &rendered, false);
                unsafe {
                    let _ = ShowWindow(regular_hwnd, SW_SHOWNOACTIVATE);
                }
                continue;
            }
        }

        let scale = theme_surface_scale(&theme, surface_index);
        let rendered = theme_engine::render_theme_surface_with_runtime_at_scale(
            &theme,
            surface_index,
            usage_data.as_ref(),
            surface_runtime,
            scale,
        );
        let mut positioned = theme_for_surface(&theme, surface_index);
        let (logical_width, logical_height) = theme_engine::resolve_surface_size(
            &theme,
            surface_index,
            usage_data.as_ref(),
            surface_runtime,
        );
        positioned.canvas.width = logical_width;
        positioned.canvas.height = logical_height;
        let placement = theme_engine::resolve_surface_placement(
            &theme,
            surface_index,
            usage_data.as_ref(),
            surface_runtime,
        );
        positioned.placement.offset_x = placement.offset_x;
        positioned.placement.offset_y = placement.offset_y;
        position_custom_theme(target_hwnd, &positioned, scale);
        if desktop_nested {
            unsafe {
                let _ = ShowWindow(target_hwnd, SW_SHOWNOACTIVATE);
            }
        }
        render_custom_window(target_hwnd, &rendered, desktop_nested);
        unsafe {
            let show = nest != SurfaceNest::Floating
                || !foreground_is_fullscreen_on_display(positioned.placement.reference.display);
            let _ = ShowWindow(target_hwnd, if show { SW_SHOWNOACTIVATE } else { SW_HIDE });
        }
    }

    for target in std::iter::once(hwnd)
        .chain(mirror_hwnds.iter().map(|mirror| mirror.to_hwnd()))
        .skip(target_count)
    {
        unsafe {
            let _ = ShowWindow(target, SW_HIDE);
        }
    }
}
fn theme_for_surface(theme: &ThemeDocument, surface_index: usize) -> ThemeDocument {
    let mut result = theme.clone();
    if let Some(surface) = theme.surfaces.get(surface_index) {
        result.canvas.width_expression = Some(surface.width.clone());
        result.canvas.height_expression = Some(surface.height.clone());
        result.canvas.background = match &surface.background {
            crate::theme_engine::LayerBackground::Colour { colour } => colour.clone(),
            crate::theme_engine::LayerBackground::None
            | crate::theme_engine::LayerBackground::Gradient { .. }
            | crate::theme_engine::LayerBackground::Image { .. } => Default::default(),
        };
        result.placement = surface.placement.clone();
        result.children = surface.children.clone();
    }
    result
}

fn request_poll(hwnd: HWND) {
    request_poll_inner(hwnd, true);
}

/// 이미 실행 중인 폴링 주기를 연장하지 않고 타이머 기반 폴링을 요청합니다.
fn request_scheduled_poll(hwnd: HWND) {
    request_poll_inner(hwnd, false);
}

fn request_poll_inner(hwnd: HWND, queue_if_busy: bool) {
    if POLL_IN_FLIGHT
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        if queue_if_busy {
            POLL_PENDING.store(true, Ordering::Release);
        }
        return;
    }
    let send_hwnd = SendHwnd::from_hwnd(hwnd);
    std::thread::spawn(move || poll_worker(send_hwnd));
}

fn poll_worker(send_hwnd: SendHwnd) {
    loop {
        do_poll_once(send_hwnd.to_hwnd());
        if POLL_PENDING.swap(false, Ordering::AcqRel) {
            continue;
        }

        POLL_IN_FLIGHT.store(false, Ordering::Release);
        if !POLL_PENDING.swap(false, Ordering::AcqRel) {
            break;
        }

        // 대기 확인(pending check)과 진행 중(in-flight) 플래그 해제 사이에 새로운 요청이 도착할 수 있습니다.
        // 해당 요청이 이미 대체 워커를 시작하지 않은 한 다시 소유권을 획득합니다.
        if POLL_IN_FLIGHT
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            break;
        }
    }
}

fn do_poll_once(hwnd: HWND) {
    let (enabled_providers, accounts, previous, force) = {
        let mut state = lock_state();
        state
            .as_mut()
            .map(|state| {
                (
                    state.providers,
                    state.accounts.clone(),
                    state.data.clone(),
                    std::mem::take(&mut state.force_notify_auth_error),
                )
            })
            .unwrap_or_default()
    };

    match poller::poll(enabled_providers, &accounts, previous.as_ref(), force) {
        Ok(data) => {
            let mut state = lock_state();
            if state
                .as_ref()
                .is_some_and(|s| s.providers != enabled_providers || s.accounts != accounts)
            {
                return;
            }
            let mut data = match state.as_ref().and_then(|s| s.data.as_ref()) {
                Some(previous) => poller::carry_forward_failures(data, previous, enabled_providers),
                None => data,
            };
            data.select_accounts(&accounts);
            let notifications: Vec<_> = data
                .new_auth_failures(previous.as_ref(), force)
                .into_iter()
                .map(|account| (account.provider, account.profile.name.clone()))
                .collect();
            let threshold_alerts = data.threshold_crossings(previous.as_ref());
            let language = state
                .as_ref()
                .map(|state| state.language)
                .unwrap_or(LanguageId::Korean);
            let cache_data = data.clone();
            if let Some(s) = state.as_mut() {
                // 리셋 데이터가 최신 상태가 되면 고속 폴링 중지
                if !poller::app_is_past_reset(&data) {
                    unsafe {
                        let _ = KillTimer(Some(hwnd), TIMER_RESET_POLL);
                    }
                }

                s.data = Some(data);
                s.last_poll_ok = true;

                // 오류에서 복구됨 — 정상 폴링 주기로 복원
                if s.retry_count > 0 {
                    s.retry_count = 0;
                    let interval = s.poll_interval_ms;
                    unsafe {
                        SetTimer(Some(hwnd), TIMER_POLL, interval, None);
                    }
                }
                s.auth_error_paused_polling = false;
                s.auth_watch_mode = poller::CredentialWatchMode::ActiveSource(
                    s.providers.first().unwrap_or_default(),
                );
                s.auth_watch_snapshot.clear();
            }
            drop(state);
            let _ = app_settings::save_usage_cache(&cache_data, true);
            if !notifications.is_empty() {
                let body = notifications
                    .iter()
                    .map(|(provider, name)| {
                        format!(
                            "{} ({name}): {}",
                            language.text(provider.descriptor().display_name),
                            language.provider_auth_error(*provider).1
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                tray_icon::notify_balloon(hwnd, language.text("Sign in again"), &body);
            }
            if !threshold_alerts.is_empty() {
                let title = if language.code() == "ko" {
                    "🔔 사용량 경고"
                } else {
                    "🔔 Usage alert"
                };
                let strings = language.strings();
                let body = threshold_alerts
                    .iter()
                    .map(|alert| {
                        let name = language.text(alert.provider.descriptor().display_name);
                        let mut line = if language.code() == "ko" {
                            match alert.level {
                                crate::models::ThresholdLevel::Warn => {
                                    format!("{name} 70% 초과 ({:.0}%)", alert.percentage)
                                }
                                crate::models::ThresholdLevel::Critical => {
                                    format!("{name} 한도 임박! ({:.0}%)", alert.percentage)
                                }
                            }
                        } else {
                            match alert.level {
                                crate::models::ThresholdLevel::Warn => {
                                    format!("{name} over 70% ({:.0}%)", alert.percentage)
                                }
                                crate::models::ThresholdLevel::Critical => {
                                    format!("{name} almost at limit! ({:.0}%)", alert.percentage)
                                }
                            }
                        };
                        if let Some(resets_at) = alert.resets_at {
                            if let Ok(remaining) = resets_at.duration_since(std::time::SystemTime::now()) {
                                let total_mins = remaining.as_secs() / 60;
                                if total_mins > 0 {
                                    let hours = total_mins / 60;
                                    let mins = total_mins % 60;
                                    if language.code() == "ko" {
                                        if hours > 0 {
                                            line.push_str(&format!(
                                                " · {hours}{} {mins}{}",
                                                strings.hour_suffix, strings.minute_suffix
                                            ));
                                        } else {
                                            line.push_str(&format!(" · {mins}{}", strings.minute_suffix));
                                        }
                                    } else if hours > 0 {
                                        line.push_str(&format!(" · reset in {hours}h {mins}m"));
                                    } else {
                                        line.push_str(&format!(" · reset in {mins}m"));
                                    }
                                }
                            }
                        }
                        line
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                tray_icon::notify_balloon(hwnd, title, &body);
            }

            unsafe {
                let _ = PostMessageW(Some(hwnd), WM_APP_USAGE_UPDATED, WPARAM(0), LPARAM(0));
            }
        }
        Err(failure) => {
            if lock_state()
                .as_ref()
                .is_some_and(|s| s.providers != enabled_providers || s.accounts != accounts)
            {
                return;
            }
            let auth_watch = match failure.error {
                poller::PollError::AuthRequired
                | poller::PollError::TokenExpired
                | poller::PollError::HttpStatus(401 | 403) => {
                    let mode = poller::CredentialWatchMode::ActiveSource(failure.provider);
                    Some((mode, poller::credential_watch_snapshot(mode)))
                }
                poller::PollError::NoCredentials => {
                    let mode = poller::CredentialWatchMode::AllSources(failure.provider);
                    Some((mode, poller::credential_watch_snapshot(mode)))
                }
                poller::PollError::RequestFailed | poller::PollError::HttpStatus(_) => None,
            };
            // 인증 필요 오류와 일시적 오류를 구분합니다.
            let (notify_auth_error, cache_data, cache_poll_ok) = {
                let mut state = lock_state();
                if state
                    .as_ref()
                    .is_some_and(|s| s.providers != enabled_providers || s.accounts != accounts)
                {
                    return;
                }
                let mut should_notify = false;
                if let Some(s) = state.as_mut() {
                    if failure.error.is_transient() {
                        if let Some(previous) = s.data.as_ref() {
                            let carried = poller::carry_forward_failures(
                                AppUsageData::default(),
                                previous,
                                enabled_providers,
                            );
                            s.data = Some(carried);
                        }
                    }
                    s.last_poll_ok = false;
                    match auth_watch {
                        Some((watch_mode, watch_snapshot)) => {
                            // 반복적인 스팸을 방지하기 위해 첫 번째 실패 시에만 말풍선 알림을 표시합니다.
                            if s.retry_count == 0 || force {
                                should_notify = true;
                            }
                            s.auth_error_paused_polling = true;
                            s.auth_watch_mode = watch_mode;
                            s.auth_watch_snapshot = watch_snapshot;
                            s.retry_count = s.retry_count.saturating_add(1);
                            unsafe {
                                let _ = KillTimer(Some(hwnd), TIMER_POLL);
                                let _ = KillTimer(Some(hwnd), TIMER_RESET_POLL);
                                let _ = KillTimer(Some(hwnd), TIMER_COUNTDOWN);
                                SetTimer(Some(hwnd), TIMER_POLL, s.poll_interval_ms, None);
                            }
                        }
                        _ => {
                            // 일시적인 네트워크 / 인증 정보 누락 오류: 지수 백오프 적용.
                            s.auth_error_paused_polling = false;
                            s.auth_watch_mode = poller::CredentialWatchMode::ActiveSource(
                                s.providers.first().unwrap_or_default(),
                            );
                            s.auth_watch_snapshot.clear();
                            s.retry_count = s.retry_count.saturating_add(1);
                            let retry_ms = poller::poll_retry_backoff_ms(
                                s.retry_count,
                                s.poll_interval_ms,
                            );
                            unsafe {
                                let _ = KillTimer(Some(hwnd), TIMER_RESET_POLL);
                                SetTimer(Some(hwnd), TIMER_POLL, retry_ms, None);
                            }
                        }
                    }
                }
                let cache_data = state
                    .as_ref()
                    .and_then(|state| state.data.clone())
                    .unwrap_or_default();
                let cache_poll_ok = state.as_ref().is_some_and(|state| {
                    poll_display_state(
                        state.last_poll_ok,
                        state.retry_count,
                        state.auth_error_paused_polling,
                        state.data.as_ref(),
                    )
                    .0
                });
                (should_notify, cache_data, cache_poll_ok)
            };
            // 모니터는 이 캐시를 따릅니다. 사용 가능한 이전 데이터가 있는
            // 일시적 오류는 계속 표시될 수 있으며, 치명적 오류나 읽을 수 없는 실패는 오류 상태로 유지됩니다.
            let _ = app_settings::save_usage_cache(&cache_data, cache_poll_ok);

            if notify_auth_error {
                let balloon = {
                    let state = lock_state();
                    state
                        .as_ref()
                        .map(|state| state.language.provider_auth_error(failure.provider))
                };
                if let Some((title, body)) = balloon {
                    tray_icon::notify_balloon(hwnd, title, body);
                }
            }

            unsafe {
                let _ = PostMessageW(Some(hwnd), WM_APP_USAGE_UPDATED, WPARAM(0), LPARAM(0));
            }
        }
    }
}

fn schedule_countdown_timer() {
    let state = lock_state();
    let s = match state.as_ref() {
        Some(s) => s,
        None => return,
    };

    let hwnd = s.hwnd.to_hwnd();
    if !s.last_poll_ok {
        unsafe {
            let _ = KillTimer(Some(hwnd), TIMER_COUNTDOWN);
            let _ = KillTimer(Some(hwnd), TIMER_RESET_POLL);
        }
        return;
    }

    // 리셋 시간이 지난 경우 최신 데이터를 가져오기 위해 5초마다 폴링
    if s.data.as_ref().is_some_and(poller::app_is_past_reset) {
        unsafe {
            SetTimer(Some(hwnd), TIMER_RESET_POLL, 5_000, None);
        }
    }

    let min_delay = s.data.as_ref().and_then(|data| {
        data.all_usage()
            .flat_map(|usage| [&usage.session, &usage.weekly])
            .filter_map(|section| poller::time_until_display_change(section.resets_at))
            .min()
    });

    let ms = min_delay
        .unwrap_or(Duration::from_secs(60))
        .as_millis()
        .max(1000) as u32;

    unsafe {
        SetTimer(Some(hwnd), TIMER_COUNTDOWN, ms, None);
    }
}

fn schedule_clock_timer() {
    let state = lock_state();
    let Some(s) = state.as_ref() else {
        return;
    };
    let hwnd = s.hwnd.to_hwnd();
    let Some(interval) = s.theme_clock_interval else {
        unsafe {
            let _ = KillTimer(Some(hwnd), TIMER_CLOCK);
        }
        return;
    };
    let ms = time_until_next_clock_refresh(interval).as_millis().max(1) as u32;
    unsafe {
        SetTimer(Some(hwnd), TIMER_CLOCK, ms, None);
    }
}

fn time_until_next_clock_refresh(interval: Duration) -> Duration {
    let interval_ms = interval.as_millis().max(1);
    let elapsed_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or(0);
    let remaining_ms = interval_ms - elapsed_ms % interval_ms;
    Duration::from_millis(remaining_ms as u64)
}

fn check_theme_change() {
    let new_dark = theme::is_dark_mode();
    let changed = {
        let mut state = lock_state();
        if let Some(s) = state.as_mut() {
            if s.is_dark != new_dark {
                s.is_dark = new_dark;
                true
            } else {
                false
            }
        } else {
            false
        }
    };
    if changed {
        render_layered();
    }
}

fn check_language_change() {
    if update_language_change() {
        render_layered();
    }
}

fn reload_external_settings(hwnd: HWND) {
    let settings = load_settings();
    let language_override = settings.language.as_deref().and_then(LanguageId::from_code);
    let theme_path = settings.active_theme_path.as_ref().map(PathBuf::from);
    let providers_changed;
    {
        let mut state = lock_state();
        let Some(state) = state.as_mut() else {
            return;
        };
        providers_changed =
            state.providers != settings.enabled_providers() || state.accounts != settings.accounts;
        state.accounts = settings.accounts.clone();
        if let Some(data) = state.data.as_mut() {
            data.select_accounts(&settings.accounts);
        }
        state.poll_interval_ms = settings.poll_interval_ms;
        state.providers = settings.enabled_providers();
        state.usage_countdown = settings.usage_countdown;
        state.taskbar_ring_badge = settings.taskbar_ring_badge;
        state.taskbar_index = settings.taskbar_index;
        state.tray_offset = settings.tray_offset;
        state.placement_override = settings.placement_override;
        state.floating_card_opacity = settings.floating_card_opacity;
        apply_language_to_state(state, language_override);
    }
    unsafe {
        SetTimer(Some(hwnd), TIMER_POLL, settings.poll_interval_ms, None);
    }
    let _ = apply_custom_theme(hwnd, settings.custom_theme_enabled, theme_path, None);
    if providers_changed {
        request_poll(hwnd);
    }
    sync_tray_icon(hwnd);
    position_at_taskbar();
    render_layered();
}

fn suppress_tray_reposition_for(duration: Duration) {
    let mut until = SUPPRESS_TRAY_REPOSITION_UNTIL
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    *until = Some(Instant::now() + duration);
}

fn tray_reposition_is_suppressed() -> bool {
    let now = Instant::now();
    let mut until = SUPPRESS_TRAY_REPOSITION_UNTIL
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    match *until {
        Some(deadline) if now < deadline => true,
        Some(_) => {
            *until = None;
            false
        }
        None => false,
    }
}

mod host_geometry;
mod message_loop;
use host_geometry::*;
use message_loop::wnd_proc;
mod positioning;
use positioning::*;
mod mouse;
use mouse::*;
mod window_context_menu;
use window_context_menu::*;

#[cfg(test)]
mod placement_tests;

#[cfg(test)]
mod layered_window_tests;

#[cfg(test)]
mod tray_usage_summary_tests {
    use super::*;
    use crate::models::{UsageData, UsageSection};

    fn usage(session: f64, weekly: f64, weekly_label: Option<&str>) -> UsageData {
        UsageData {
            session: UsageSection {
                available: true,
                percentage: session,
                resets_at: None,
            },
            weekly: UsageSection {
                available: true,
                percentage: weekly,
                resets_at: None,
            },
            weekly_label: weekly_label.map(str::to_string),
            ..Default::default()
        }
    }

    #[test]
    fn tray_summary_formats_enabled_provider_usage() {
        let data = [(ProviderId::Claude, usage(4.6, 42.4, None))]
            .into_iter()
            .collect();

        assert_eq!(
            tray_usage_summary_lines(
                &data,
                ProviderSet::from_enabled([ProviderId::Claude]),
                LanguageId::Korean,
                false,
            ),
            ["Claude Code 5h: 5% | 7d: 42%"]
        );
    }

    #[test]
    fn tray_summary_counts_down_when_the_widget_shows_what_is_left() {
        let data = [(ProviderId::Claude, usage(4.6, 42.4, None))]
            .into_iter()
            .collect();

        assert_eq!(
            tray_usage_summary_lines(
                &data,
                ProviderSet::from_enabled([ProviderId::Claude]),
                LanguageId::Korean,
                true,
            ),
            ["Claude Code 5h: 95% | 7d: 58%"]
        );
    }

    #[test]
    fn tray_summary_uses_provider_window_labels_and_selection() {
        let data = [
            (ProviderId::Claude, usage(10.0, 20.0, None)),
            (ProviderId::OpenCode, usage(30.0, 40.0, Some("30d"))),
        ]
        .into_iter()
        .collect();

        assert_eq!(
            tray_usage_summary_lines(
                &data,
                ProviderSet::from_enabled([ProviderId::OpenCode]),
                LanguageId::Korean,
                false,
            ),
            ["OpenCode 5h: 30% | 30d: 40%"]
        );
    }
}

#[cfg(test)]
mod poll_display_state_tests {
    use super::*;
    use crate::models::UsageData;

    fn cached_usage(stale: bool) -> AppUsageData {
        let usage = UsageData {
            stale,
            ..Default::default()
        };
        [(ProviderId::Claude, usage)].into_iter().collect()
    }

    #[test]
    fn transient_failure_keeps_a_stale_reading_displayable() {
        let data = cached_usage(true);
        assert_eq!(
            poll_display_state(false, 1, false, Some(&data)),
            (true, false)
        );
    }

    #[test]
    fn failures_without_stale_data_remain_errors() {
        let fresh = cached_usage(false);
        let stale = cached_usage(true);

        assert_eq!(poll_display_state(false, 1, false, None), (false, true));
        assert_eq!(
            poll_display_state(false, 1, false, Some(&fresh)),
            (false, true)
        );
        assert_eq!(
            poll_display_state(false, 1, true, Some(&stale)),
            (false, true)
        );
    }
}

#[cfg(test)]
mod startup_config_tests {
    use super::*;

    fn args(flags: &[&str]) -> Vec<String> {
        flags.iter().map(|flag| flag.to_string()).collect()
    }

    #[test]
    fn run_args_default_to_single_instance_with_poll() {
        let options = parse_run_args(&args(&[]));
        assert!(!options.allow_multiple);
        assert!(!options.no_poll);
    }

    #[test]
    fn run_args_recognize_allow_multiple_and_no_poll() {
        let options = parse_run_args(&args(&["--allow-multiple", "--no-poll"]));
        assert!(options.allow_multiple);
        assert!(options.no_poll);
    }

    #[test]
    fn run_args_ignore_unknown_flags() {
        let options = parse_run_args(&args(&["--verbose"]));
        assert!(!options.allow_multiple);
        assert!(!options.no_poll);
    }

    fn startup_config() -> StartupConfig {
        StartupConfig {
            settings: SettingsFile::default(),
            active_theme_path: None,
            active_theme: Some(ThemeDocument::starter()),
            custom_theme_enabled: true,
            theme_clock_interval: None,
            tray_theme_uses_current_time: false,
            language_override: None,
            language: LanguageId::Korean,
        }
    }

    #[test]
    fn initial_state_mirrors_settings_and_theme() {
        let config = startup_config();
        let hwnd = HWND(std::ptr::null_mut());
        let state = build_initial_state(hwnd, true, &config);
        assert_eq!(state.hwnd.to_hwnd(), hwnd);
        assert!(state.is_dark);
        assert_eq!(state.providers, config.settings.enabled_providers());
        assert_eq!(state.accounts, config.settings.accounts);
        assert_eq!(state.poll_interval_ms, config.settings.poll_interval_ms);
        assert_eq!(state.usage_countdown, config.settings.usage_countdown);
        assert!(state.data.is_none());
        assert!(state.active_theme.is_some());
        assert_eq!(state.language, LanguageId::Korean);
    }
}

#[cfg(test)]
mod placement_regression_tests;
