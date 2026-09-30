//! Windows 트레이 아이콘 저수준 제어 — 처음 보시는 분을 위한 안내.
//!
//! - 알림 영역에 아이콘을 달고, 말풍선·클릭 소식을 주고받습니다.
//! - Windows 전용이라 다른 OS 빌드에서는 빠집니다.

use std::sync::Mutex;

use windows::core::PCWSTR;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleFileNameW;
use windows::Win32::UI::Shell::{
    ExtractIconExW, Shell_NotifyIconGetRect, Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_MESSAGE,
    NIF_TIP, NIIF_WARNING, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW, NOTIFYICONIDENTIFIER,
};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::native_interop::WM_APP_TRAY;

const APP_TRAY_ICON_ID: u32 = 1;
const THEME_TRAY_ICON_ID_BASE: u32 = 1_000;

static REGISTERED_THEME_ICON_IDS: Mutex<Vec<u32>> = Mutex::new(Vec::new());

/// 실제 알림 영역 아이콘으로 노출할 래스터화된 테마 루트입니다.
pub struct ThemedTrayIcon {
    pub surface_index: usize,
    pub tooltip: String,
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u32>,
}

/// 트레이 메시지 핸들러가 메인 윈도우에 요청할 수 있는 동작들입니다.
pub enum TrayAction {
    None,
    ShowContextMenu,
}

/// build.rs에 의해 임베드된 애플리케이션 아이콘(src/icons/icon.ico)을 로드합니다.
/// 네이티브 윈도우와 시스템 트레이가 이 소스를 공유하여, Windows가 단일 비트맵을
/// 확대/축소하는 대신 정확한 크기의 대형/소형 아이콘을 선택할 수 있도록 합니다.
pub fn load_app_icons() -> (HICON, HICON) {
    unsafe {
        let mut exe_buf = [0u16; 260];
        let len = GetModuleFileNameW(None, &mut exe_buf) as usize;
        if len == 0 {
            return (HICON::default(), HICON::default());
        }

        let mut small_icon = HICON::default();
        let mut large_icon = HICON::default();
        let extracted = ExtractIconExW(
            PCWSTR::from_raw(exe_buf.as_ptr()),
            0,
            Some(&mut large_icon),
            Some(&mut small_icon),
            1,
        );

        if extracted == 0 {
            (HICON::default(), HICON::default())
        } else {
            (large_icon, small_icon)
        }
    }
}

fn load_app_icon() -> HICON {
    let (large_icon, small_icon) = load_app_icons();
    if !small_icon.is_invalid() {
        if !large_icon.is_invalid() {
            unsafe {
                let _ = DestroyIcon(large_icon);
            }
        }
        small_icon
    } else {
        large_icon
    }
}

fn themed_icon_id(surface_index: usize) -> u32 {
    THEME_TRAY_ICON_ID_BASE.saturating_add(surface_index.min(u32::MAX as usize) as u32)
}

pub fn themed_surface_index(id: u32) -> Option<usize> {
    (id >= THEME_TRAY_ICON_ID_BASE).then(|| (id - THEME_TRAY_ICON_ID_BASE) as usize)
}

pub fn cursor_over_themed_icon(hwnd: HWND, surface_index: usize) -> bool {
    unsafe {
        let identifier = NOTIFYICONIDENTIFIER {
            cbSize: std::mem::size_of::<NOTIFYICONIDENTIFIER>() as u32,
            hWnd: hwnd,
            uID: themed_icon_id(surface_index),
            ..Default::default()
        };
        let Ok(rect) = Shell_NotifyIconGetRect(&identifier) else {
            return false;
        };
        let mut point = POINT::default();
        GetCursorPos(&mut point).is_ok()
            && point.x >= rect.left
            && point.x < rect.right
            && point.y >= rect.top
            && point.y < rect.bottom
    }
}

fn create_themed_icon(icon: &ThemedTrayIcon) -> HICON {
    if icon.width == 0
        || icon.height == 0
        // 파일 탐색기는 궁극적으로 하나의 정사각형 알림 영역 슬롯을 표시합니다.
        // 최대 크기를 제한하여 테마 표현식의 실수로 인해 GDI와 셸이 거대한
        // 아이콘 비트맵을 유지하도록 요구하는 문제를 방지합니다.
        || icon.width > 512
        || icon.height > 512
        || icon.pixels.len() != icon.width as usize * icon.height as usize
    {
        return HICON::default();
    }

    unsafe {
        let screen_dc = GetDC(None);
        let memory_dc = CreateCompatibleDC(Some(screen_dc));
        let bitmap_info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: icon.width as i32,
                // 테마 픽셀은 상단 우선(top-down)이므로 DIB 역시 상단 우선 방식을 사용합니다.
                biHeight: -(icon.height as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        let color_bitmap = CreateDIBSection(
            Some(memory_dc),
            &bitmap_info,
            DIB_RGB_COLORS,
            &mut bits,
            None,
            0,
        )
        .unwrap_or_default();
        if color_bitmap.is_invalid() || bits.is_null() {
            let _ = DeleteDC(memory_dc);
            ReleaseDC(None, screen_dc);
            return HICON::default();
        }
        std::ptr::copy_nonoverlapping(icon.pixels.as_ptr(), bits.cast::<u32>(), icon.pixels.len());

        // 0으로 채워진 단색(monochrome) 마스크는 32비트 컬러 비트맵의 알파 채널이
        // 투명 픽셀 및 안티앨리어싱 가장자리를 정의하도록 합니다. Win32 요구사항에 따라
        // 1bpp 스캔라인은 16비트 WORD 정렬되어야 하며 0으로 채워져야 합니다.
        let stride = ((icon.width + 15) / 16) * 2;
        let mask_bytes = vec![0u8; (stride * icon.height) as usize];
        let mask_bitmap = CreateBitmap(
            icon.width as i32,
            icon.height as i32,
            1,
            1,
            Some(mask_bytes.as_ptr().cast()),
        );
        if mask_bitmap.is_invalid() {
            let _ = DeleteObject(color_bitmap.into());
            let _ = DeleteDC(memory_dc);
            ReleaseDC(None, screen_dc);
            return HICON::default();
        }
        let icon_info = ICONINFO {
            fIcon: TRUE,
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mask_bitmap,
            hbmColor: color_bitmap,
        };
        let result = CreateIconIndirect(&icon_info).unwrap_or_default();

        let _ = DeleteObject(mask_bitmap.into());
        let _ = DeleteObject(color_bitmap.into());
        let _ = DeleteDC(memory_dc);
        ReleaseDC(None, screen_dc);
        result
    }
}

fn remove_id(hwnd: HWND, id: u32) {
    unsafe {
        let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
        nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = hwnd;
        nid.uID = id;
        let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
    }
}

fn remove_registered_theme_icons(hwnd: HWND) {
    let mut registered = REGISTERED_THEME_ICON_IDS
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    for id in registered.drain(..) {
        remove_id(hwnd, id);
    }
}

/// 단일 영속 애플리케이션 트레이 아이콘을 등록하거나 갱신합니다.
pub fn sync(hwnd: HWND, tooltip: &str) {
    remove_registered_theme_icons(hwnd);
    let hicon = load_app_icon();
    if hicon.is_invalid() {
        return;
    }

    unsafe {
        let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
        nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = hwnd;
        nid.uID = APP_TRAY_ICON_ID;
        nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
        nid.uCallbackMessage = WM_APP_TRAY;
        nid.hIcon = hicon;
        copy_to_tip(tooltip, &mut nid.szTip);

        // 최초 등록 시 NIM_ADD가 성공합니다. 아이콘이 이미 존재하는 경우
        // NIM_MODIFY가 이미지, 콜백 및 툴팁을 갱신합니다.
        if !Shell_NotifyIconW(NIM_ADD, &nid).as_bool() {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
        }
        let _ = DestroyIcon(hicon);
    }
}

/// 테마 루트들을 독립적인 알림 영역 아이콘들로 등록합니다.
/// 다른 일반 앱 아이콘들과 마찬가지로 셸이 표시 순서 및 오버플로 배치를 관리합니다.
pub fn sync_themed(hwnd: HWND, icons: &[ThemedTrayIcon]) {
    remove_id(hwnd, APP_TRAY_ICON_ID);
    let mut refreshed_ids = Vec::with_capacity(icons.len());
    for icon in icons {
        let hicon = create_themed_icon(icon);
        if hicon.is_invalid() {
            continue;
        }
        let id = themed_icon_id(icon.surface_index);
        unsafe {
            let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
            nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
            nid.hWnd = hwnd;
            nid.uID = id;
            nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
            nid.uCallbackMessage = WM_APP_TRAY;
            nid.hIcon = hicon;
            copy_to_tip(&icon.tooltip, &mut nid.szTip);
            if !Shell_NotifyIconW(NIM_ADD, &nid).as_bool() {
                let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
            }
            let _ = DestroyIcon(hicon);
        }
        refreshed_ids.push(id);
    }

    let mut registered = REGISTERED_THEME_ICON_IDS
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    for id in registered
        .iter()
        .copied()
        .filter(|id| !refreshed_ids.contains(id))
    {
        remove_id(hwnd, id);
    }
    *registered = refreshed_ids;
}

/// 애플리케이션 트레이 아이콘에서 Windows 말풍선 알림을 표시합니다.
pub fn notify_balloon(hwnd: HWND, title: &str, message: &str) {
    let icon_id = REGISTERED_THEME_ICON_IDS
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .first()
        .copied()
        .unwrap_or(APP_TRAY_ICON_ID);
    unsafe {
        let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
        nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = hwnd;
        nid.uID = icon_id;
        nid.uFlags = NIF_INFO;
        nid.dwInfoFlags = NIIF_WARNING;
        copy_wide(title, &mut nid.szInfoTitle);
        copy_wide(message, &mut nid.szInfo);
        let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
    }
}

/// 셸에서 애플리케이션 트레이 아이콘을 제거합니다.
pub fn remove_all(hwnd: HWND) {
    remove_id(hwnd, APP_TRAY_ICON_ID);
    remove_registered_theme_icons(hwnd);
}

/// 트레이 콜백 메시지를 해석하여 수행할 동작을 반환합니다.
pub fn handle_message(lparam: LPARAM) -> TrayAction {
    let mouse_msg = lparam.0 as u32;
    match mouse_msg {
        WM_LBUTTONUP | WM_LBUTTONDBLCLK | WM_RBUTTONUP | WM_CONTEXTMENU => {
            TrayAction::ShowContextMenu
        }
        _ => TrayAction::None,
    }
}

fn copy_wide<const N: usize>(value: &str, buffer: &mut [u16; N]) {
    let wide: Vec<u16> = value.encode_utf16().collect();
    let mut len = wide.len().min(N - 1);
    if len > 0 && (0xD800..=0xDBFF).contains(&wide[len - 1]) {
        len -= 1;
    }
    buffer[..len].copy_from_slice(&wide[..len]);
    buffer[len] = 0;
}

fn copy_to_tip(value: &str, tooltip: &mut [u16; 128]) {
    copy_wide(value, tooltip);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tray_buttons_have_distinct_actions() {
        assert!(matches!(
            handle_message(LPARAM(WM_LBUTTONUP as isize)),
            TrayAction::ShowContextMenu
        ));
        assert!(matches!(
            handle_message(LPARAM(WM_LBUTTONDBLCLK as isize)),
            TrayAction::ShowContextMenu
        ));
        assert!(matches!(
            handle_message(LPARAM(WM_RBUTTONUP as isize)),
            TrayAction::ShowContextMenu
        ));
        assert!(matches!(
            handle_message(LPARAM(WM_CONTEXTMENU as isize)),
            TrayAction::ShowContextMenu
        ));
    }

    #[test]
    fn theme_icon_ids_do_not_overlap_the_application_icon() {
        assert_ne!(themed_icon_id(0), APP_TRAY_ICON_ID);
        assert_ne!(themed_icon_id(42), themed_icon_id(43));
    }

    #[test]
    fn test_create_themed_icon_valid_hicon() {
        let pixels = vec![0x80FF0000u32; 32 * 32];
        let icon = ThemedTrayIcon {
            surface_index: 1,
            tooltip: "Test".into(),
            width: 32,
            height: 32,
            pixels,
        };
        let hicon = create_themed_icon(&icon);
        assert!(!hicon.is_invalid());
        unsafe {
            let _ = DestroyIcon(hicon);
        }
    }
}
