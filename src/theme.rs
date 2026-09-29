//! 시스템 테마(다크·라이트) 감지 — 처음 보시는 분을 위한 안내.
//!
//! - Windows 레지스트리에서 밝은 모드 여부를 읽습니다. 그 외 OS에서는 기본값을 씁니다.

#[cfg(windows)]
use windows::core::PCWSTR;
#[cfg(windows)]
use windows::Win32::System::Registry::*;

#[cfg(windows)]
const REGISTRY_PATH: &str = r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";
#[cfg(windows)]
const REGISTRY_KEY: &str = "SystemUsesLightTheme";

/// Check if the system is in dark mode
#[cfg(windows)]
pub fn is_dark_mode() -> bool {
    !is_light_theme()
}

#[cfg(windows)]
fn is_light_theme() -> bool {
    unsafe {
        let path = crate::native_interop::wide_str(REGISTRY_PATH);
        let key_name = crate::native_interop::wide_str(REGISTRY_KEY);

        let mut hkey = HKEY::default();
        let result = RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR::from_raw(path.as_ptr()),
            None,
            KEY_READ,
            &mut hkey,
        );

        if result.is_err() {
            return false; // Default to dark mode
        }

        let mut data: u32 = 0;
        let mut data_size: u32 = std::mem::size_of::<u32>() as u32;
        let result = RegQueryValueExW(
            hkey,
            PCWSTR::from_raw(key_name.as_ptr()),
            None,
            None,
            Some(&mut data as *mut u32 as *mut u8),
            Some(&mut data_size),
        );

        let _ = RegCloseKey(hkey);

        if result.is_err() {
            return false; // Default to dark mode
        }

        data == 1
    }
}

#[cfg(not(windows))]
pub fn is_dark_mode() -> bool {
    #[cfg(target_os = "macos")]
    {
        if let Ok(output) = std::process::Command::new("defaults")
            .args(["read", "-g", "AppleInterfaceStyle"])
            .output()
        {
            if String::from_utf8_lossy(&output.stdout).trim() == "Dark" {
                return true;
            }
        }
    }
    true // Default to dark mode
}
