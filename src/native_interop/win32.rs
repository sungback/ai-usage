use std::sync::Mutex;

use windows::core::BOOL;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{HWND, LPARAM, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO,
};
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows::Win32::UI::WindowsAndMessaging::*;

use super::{wide_str, WS_CHILD_STYLE, WS_CLIPSIBLINGS_STYLE, WS_POPUP_STYLE};

pub fn is_taskbar_horizontal(rect: RECT) -> bool {
    (rect.right - rect.left) >= (rect.bottom - rect.top)
}

static DESKTOP_HOST: Mutex<Option<(isize, isize)>> = Mutex::new(None);

#[derive(Clone, Copy, Debug)]
pub struct TaskbarWindow {
    pub hwnd: HWND,
    pub rect: RECT,
}

#[derive(Clone, Copy, Debug)]
pub struct DisplayMonitor {
    pub handle: HMONITOR,
    pub rect: RECT,
    pub primary: bool,
}

/// 바탕화면에 중첩되는 테마 서피스의 부모 지정 및 형제(sibling) 윈도우 배치 정보입니다.
/// `insert_after`는 서피스를 Explorer의 바탕화면 Z-순서 대역에 배치합니다.
#[derive(Clone, Copy, Debug)]
pub struct DesktopHost {
    pub parent: HWND,
    pub insert_after: HWND,
}

pub fn find_monitors() -> Vec<DisplayMonitor> {
    unsafe extern "system" fn enum_proc(
        monitor: HMONITOR,
        _dc: HDC,
        _rect: *mut RECT,
        data: LPARAM,
    ) -> BOOL {
        let result = &mut *(data.0 as *mut Vec<DisplayMonitor>);
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if unsafe { GetMonitorInfoW(monitor, &mut info).as_bool() } {
            result.push(DisplayMonitor {
                handle: monitor,
                rect: info.rcMonitor,
                primary: info.dwFlags & MONITORINFOF_PRIMARY != 0,
            });
        }
        BOOL(1)
    }
    let mut result: Vec<DisplayMonitor> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(enum_proc),
            LPARAM(&mut result as *mut _ as isize),
        );
    }
    result.sort_by_key(|display| (!display.primary, display.rect.left, display.rect.top));
    result
}

pub fn find_taskbars() -> Vec<TaskbarWindow> {
    unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let taskbars = &mut *(lparam.0 as *mut Vec<TaskbarWindow>);
        let mut class_name = [0u16; 64];
        let len = unsafe { GetClassNameW(hwnd, &mut class_name) };
        if len > 0 {
            let class_name = String::from_utf16_lossy(&class_name[..len as usize]);
            if class_name == "Shell_TrayWnd" || class_name == "Shell_SecondaryTrayWnd" {
                if let Some(rect) = get_taskbar_rect(hwnd) {
                    taskbars.push(TaskbarWindow { hwnd, rect });
                }
            }
        }
        BOOL(1)
    }

    let mut taskbars: Vec<TaskbarWindow> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(enum_proc), LPARAM(&mut taskbars as *mut _ as isize));
    }
    taskbars.sort_by_key(|taskbar| {
        (
            taskbar.rect.top,
            taskbar.rect.left,
            taskbar.rect.bottom,
            taskbar.rect.right,
        )
    });
    taskbars
}

/// 클래스 이름으로 자식 윈도우를 검색합니다.
pub fn find_child_window(parent: HWND, class_name: &str) -> Option<HWND> {
    unsafe {
        let class = wide_str(class_name);
        match FindWindowExW(
            Some(parent),
            None,
            PCWSTR::from_raw(class.as_ptr()),
            PCWSTR::null(),
        ) {
            Ok(h) if h != HWND::default() => Some(h),
            _ => None,
        }
    }
}

/// 작업 표시줄 위치를 안전하게 가져옵니다.
/// GetWindowRect는 explorer.exe가 멈추거나 응답하지 않더라도 즉시 반환되는
/// 비차단(non-blocking) 커널 모드 쿼리이므로 get_window_rect_safe를 직접 사용합니다.
/// 반면 SHAppBarMessage는 explorer.exe의 UI 메시지 루프로 동기 LPC 메시지를 전송하므로,
/// Explorer가 멈췄을 때 모니터 스레드가 영구 교착(Event 1002) 상태에 빠집니다.
pub fn get_taskbar_rect(taskbar_hwnd: HWND) -> Option<RECT> {
    get_window_rect_safe(taskbar_hwnd)
}

/// 윈도우의 경계 사각형(bounding rect)을 가져옵니다.
pub fn get_window_rect_safe(hwnd: HWND) -> Option<RECT> {
    unsafe {
        let mut rect = RECT::default();
        if GetWindowRect(hwnd, &mut rect).is_ok() {
            Some(rect)
        } else {
            None
        }
    }
}

pub fn window_class_name(hwnd: HWND) -> Option<String> {
    unsafe {
        let mut class_name = [0u16; 128];
        let len = GetClassNameW(hwnd, &mut class_name);
        (len > 0).then(|| String::from_utf16_lossy(&class_name[..len as usize]))
    }
}

/// 셸이 소유한 윈도우 내부에 레이어드 서피스를 호스팅합니다. 부모를 지정하면
/// 독립적인 최상위 팝업으로서 경쟁하는 대신, 호스트의 표시 여부 및 Z-순서를 공유합니다.
pub fn embed_as_child(hwnd: HWND, parent: HWND) {
    unsafe {
        let current_parent = GetParent(hwnd).ok();
        let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE);
        let _ = SetWindowLongW(
            hwnd,
            GWL_EXSTYLE,
            (ex_style | WS_EX_TOOLWINDOW.0 as i32 | WS_EX_NOACTIVATE.0 as i32)
                & !(WS_EX_TOPMOST.0 as i32),
        );

        let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
        let new_style = (style & !WS_POPUP_STYLE) | WS_CHILD_STYLE | WS_CLIPSIBLINGS_STYLE;
        let _ = SetWindowLongW(hwnd, GWL_STYLE, new_style as i32);

        if current_parent != Some(parent) {
            let _ = SetParent(hwnd, Some(parent));
            // Windows 11에서는 부모가 재지정된 레이어드 서피스가 작업 표시줄의
            // DirectComposition 비주얼 뒤에 가려질 수 있습니다. 일반적인 위치 지정 중에는
            // 재바인딩하지 않고, 부모 변경이 성공한 후 딱 한 번만 재바인딩합니다.
            // 데스크톱 DirectComposition 윈도우는 레이어드 윈도우가 아니며 상태를 유지해야 합니다.
            // 호출자는 위치 지정 후 새로운 픽셀을 표시합니다.
            if GetParent(hwnd).ok() == Some(parent) {
                let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE);
                if ex_style & WS_EX_LAYERED.0 as i32 != 0 {
                    let _ = SetWindowLongW(hwnd, GWL_EXSTYLE, ex_style & !(WS_EX_LAYERED.0 as i32));
                    let _ = SetWindowLongW(hwnd, GWL_EXSTYLE, ex_style);
                }
            }
        }
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOP),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        );
    }
}

/// 셸 호스팅 서피스를 일반 최상위 팝업으로 복원합니다.
pub fn make_popup(hwnd: HWND, topmost: bool) {
    unsafe {
        let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
        let detaching = style & WS_CHILD_STYLE != 0;
        let screen_rect = if detaching {
            let Some(rect) = get_window_rect_safe(hwnd) else {
                return;
            };
            Some(rect)
        } else {
            None
        };
        let restore_visibility = detaching && style & WS_VISIBLE.0 != 0;
        if restore_visibility {
            // SetParent는 이전 클라이언트 좌표를 일시적으로 그대로 남겨둡니다.
            // 이러한 중간 위치가 컴포지터에 노출되지 않도록 숨깁니다.
            let _ = SetWindowPos(
                hwnd,
                None,
                0,
                0,
                0,
                0,
                SWP_HIDEWINDOW | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
        // 문서화된 SetParent의 자식->데스크톱 전환 순서: 먼저 부모 연결을 해제한 후
        // WS_CHILD를 제거합니다. 숨김 상태가 유지되도록 스타일을 다시 읽습니다.
        if detaching {
            let _ = SetParent(hwnd, None);
        }
        let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
        let new_style = (style & !WS_CHILD_STYLE & !WS_CLIPSIBLINGS_STYLE) | WS_POPUP_STYLE;
        let _ = SetWindowLongW(hwnd, GWL_STYLE, new_style as i32);

        let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE);
        let ex_style = if topmost {
            ex_style | WS_EX_TOPMOST.0 as i32
        } else {
            ex_style & !(WS_EX_TOPMOST.0 as i32)
        };
        let _ = SetWindowLongW(hwnd, GWL_EXSTYLE, ex_style);
        let mut flags = SWP_NOSIZE | SWP_NOACTIVATE | SWP_FRAMECHANGED;
        let (x, y) = if let Some(rect) = screen_rect {
            (rect.left, rect.top)
        } else {
            flags |= SWP_NOMOVE;
            (0, 0)
        };
        if restore_visibility {
            flags |= SWP_SHOWWINDOW;
        }
        let _ = SetWindowPos(
            hwnd,
            Some(if topmost {
                HWND_TOPMOST
            } else {
                HWND_NOTOPMOST
            }),
            x,
            y,
            0,
            0,
            flags,
        );
    }
}

/// Explorer의 데스크톱 렌더링 대역(band)을 찾습니다. 현재 Windows 11 빌드에서는
/// 레이어드 자식 윈도우가 Progman의 자식으로 들어가되, SHELLDLL_DefView 아래이자
/// 배경화면 WorkerW 위에 위치해야 합니다. 이전 버전의 셸에서는 별도의
/// 최상위 배경화면 WorkerW의 자식으로 들어갑니다.
pub fn find_desktop_host() -> Option<DesktopHost> {
    unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
        if find_child_window(hwnd, "SHELLDLL_DefView").is_some() {
            let class = wide_str("WorkerW");
            if let Ok(worker) = unsafe {
                FindWindowExW(
                    None,
                    Some(hwnd),
                    PCWSTR::from_raw(class.as_ptr()),
                    PCWSTR::null(),
                )
            } {
                if !worker.is_invalid() {
                    let result = unsafe { &mut *(lparam.0 as *mut HWND) };
                    *result = worker;
                    return BOOL(0);
                }
            }
        }
        BOOL(1)
    }

    unsafe {
        let cached = *DESKTOP_HOST
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some((parent_value, insert_after_value)) = cached {
            let parent = HWND(parent_value as *mut _);
            let insert_after = HWND(insert_after_value as *mut _);
            let sibling_is_valid = insert_after == HWND_BOTTOM
                || insert_after == HWND_TOP
                || IsWindow(Some(insert_after)).as_bool();
            if IsWindow(Some(parent)).as_bool()
                && IsWindowVisible(parent).as_bool()
                && sibling_is_valid
            {
                return Some(DesktopHost {
                    parent,
                    insert_after,
                });
            }
        }

        let progman_class = wide_str("Progman");
        let progman = FindWindowW(PCWSTR::from_raw(progman_class.as_ptr()), PCWSTR::null())
            .ok()
            .filter(|hwnd| !hwnd.is_invalid())?;

        // 데스크톱 호스팅 서피스가 요청되기 전까지 배경화면 WorkerW를 유지하지 않는
        // 셸 버전의 경우, Explorer에 WorkerW 생성을 요청합니다.
        let _ = SendMessageTimeoutW(
            progman,
            0x052C,
            WPARAM(0xD),
            LPARAM(0),
            SMTO_NORMAL,
            1_000,
            None,
        );
        let _ = SendMessageTimeoutW(
            progman,
            0x052C,
            WPARAM(0xD),
            LPARAM(1),
            SMTO_NORMAL,
            1_000,
            None,
        );

        // 승격된 데스크톱 아키텍처에서 Progman은 DefView와 배경화면 WorkerW를 직속 자식으로 둡니다.
        // WorkerW의 레이어드 자식은 DirectComposition 배경화면 서피스에 가려집니다.
        // 대신 우리의 자식 윈도우는 DefView 바로 아래의 Progman 형제여야 합니다.
        // DefView는 대부분 투명하며 우리 위에 아이콘들을 그립니다.
        let direct_def_view = find_child_window(progman, "SHELLDLL_DefView");
        let host = if let Some(def_view) = direct_def_view {
            DesktopHost {
                parent: progman,
                insert_after: def_view,
            }
        } else {
            // 이전 버전의 셸은 DefView를 다른 최상위 윈도우 아래로 옮기고
            // 그 뒤에 별도의 배경화면 WorkerW를 노출합니다.
            let mut top_level_worker = HWND::default();
            let _ = EnumWindows(
                Some(enum_proc),
                LPARAM(&mut top_level_worker as *mut _ as isize),
            );
            DesktopHost {
                parent: if top_level_worker.is_invalid() {
                    progman
                } else {
                    top_level_worker
                },
                insert_after: if top_level_worker.is_invalid() {
                    HWND_TOP
                } else {
                    HWND_BOTTOM
                },
            }
        };
        *DESKTOP_HOST
            .lock()
            .unwrap_or_else(|error| error.into_inner()) =
            Some((host.parent.0 as isize, host.insert_after.0 as isize));
        Some(host)
    }
}

/// 윈도우를 이동합니다.
pub fn move_window(hwnd: HWND, x: i32, y: i32, w: i32, h: i32) {
    unsafe {
        let _ = MoveWindow(hwnd, x, y, w, h, true);
    }
}

/// 트레이 위치 변경 감지를 위한 WinEvent 후크를 설정합니다.
pub fn set_tray_event_hook(
    thread_id: u32,
    callback: unsafe extern "system" fn(HWINEVENTHOOK, u32, HWND, i32, i32, u32, u32),
) -> Option<HWINEVENTHOOK> {
    unsafe {
        let hook = SetWinEventHook(
            EVENT_OBJECT_LOCATIONCHANGE,
            EVENT_OBJECT_LOCATIONCHANGE,
            None,
            Some(callback),
            0,
            thread_id,
            WINEVENT_OUTOFCONTEXT,
        );
        if hook.is_invalid() {
            None
        } else {
            Some(hook)
        }
    }
}

/// 윈도우를 소유한 스레드 ID를 가져옵니다.
pub fn get_window_thread_id(hwnd: HWND) -> u32 {
    unsafe { GetWindowThreadProcessId(hwnd, None) }
}

/// WinEvent 후크를 해제합니다.
pub fn unhook_win_event(hook: HWINEVENTHOOK) {
    unsafe {
        let _ = UnhookWinEvent(hook);
    }
}
