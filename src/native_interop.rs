#![cfg_attr(not(windows), allow(dead_code))]

// Window style constants
pub const WS_POPUP_STYLE: u32 = 0x80000000;
pub const WS_CHILD_STYLE: u32 = 0x40000000;
pub const WS_CLIPSIBLINGS_STYLE: u32 = 0x04000000;

// Timer IDs
pub const TIMER_POLL: usize = 1;
pub const TIMER_COUNTDOWN: usize = 2;
pub const TIMER_RESET_POLL: usize = 3;
pub const TIMER_UPDATE_CHECK: usize = 4;
pub const TIMER_WINDOW_STATE: usize = 5;
pub const TIMER_MOUSE_CLICK: usize = 6;
pub const TIMER_TRAY_HOVER: usize = 7;
pub const TIMER_CLOCK: usize = 8;
pub const TIMER_TRAY_REPOSITION: usize = 9;

// Custom messages
pub const WM_APP: u32 = 0x8000;
pub const WM_APP_USAGE_UPDATED: u32 = WM_APP + 1;
pub const WM_APP_TRAY: u32 = WM_APP + 3;
pub const WM_APP_SETTINGS_UPDATED: u32 = WM_APP + 5;
pub const WM_APP_REFRESH_NOW: u32 = WM_APP + 6;
pub const WM_APP_QUIT: u32 = WM_APP + 7;
pub const WM_APP_TRAY_DISPATCH: u32 = WM_APP + 9;
pub const WM_APP_TASKBAR_COLLISION: u32 = WM_APP + 10;
pub const WM_APP_UPDATE_ACTION: u32 = WM_APP + 13;
pub const WM_APP_CHECK_FOR_UPDATES: u32 = WM_APP + 14;

/// Convert a Rust string to a null-terminated wide string
pub fn wide_str(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
mod win32;
#[cfg(windows)]
pub use win32::*;

#[cfg(not(windows))]
pub fn find_monitors() -> Vec<()> {
    vec![()]
}

#[cfg(all(test, windows))]
mod tests;
