//! Windows 및 macOS를 위한 플랫폼 추상화 계층입니다.

pub mod ring_badge;

#[cfg(windows)]
pub mod windows {
    pub fn run() {
        crate::window::run();
    }
}

#[cfg(target_os = "macos")]
pub mod macos;

#[cfg(all(not(windows), not(target_os = "macos")))]
pub mod unix;

pub fn run() {
    #[cfg(windows)]
    {
        windows::run();
    }
    #[cfg(target_os = "macos")]
    {
        macos::run();
    }
    #[cfg(all(not(windows), not(target_os = "macos")))]
    {
        eprintln!("Unsupported platform for GUI monitor.");
    }
}
