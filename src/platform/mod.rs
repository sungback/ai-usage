//! Platform abstraction layer for Windows and macOS.

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
