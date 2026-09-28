use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

struct DiagnoseState {
    file: Mutex<Option<File>>,
    enabled: AtomicBool,
}

impl DiagnoseState {
    const fn new() -> Self {
        Self {
            file: Mutex::new(None),
            enabled: AtomicBool::new(false),
        }
    }

    fn enable(&self, path: &std::path::Path, append: bool) -> Result<bool, String> {
        let mut file = self.file.lock().map_err(|error| error.to_string())?;
        if file.is_some() {
            return Ok(false);
        }
        *file = Some(open_log(path, append).map_err(|error| {
            format!(
                "Unable to open diagnostic log file {}: {error}",
                path.display()
            )
        })?);
        self.enabled.store(true, Ordering::Release);
        Ok(true)
    }

    fn write(&self, line: &[u8]) {
        if !self.enabled.load(Ordering::Acquire) {
            return;
        }
        if let Ok(mut guard) = self.file.lock() {
            if let Some(file) = guard.as_mut() {
                let _ = file.write_all(line);
                let _ = file.flush();
            }
        }
    }
}

static DIAGNOSE_STATE: DiagnoseState = DiagnoseState::new();

pub fn log_path() -> PathBuf {
    std::env::temp_dir().join("ai-usage.log")
}

pub fn init() -> Result<PathBuf, String> {
    init_file(false)
}

pub fn init_append() -> Result<PathBuf, String> {
    init_file(true)
}

fn init_file(append: bool) -> Result<PathBuf, String> {
    let path = log_path();
    if !DIAGNOSE_STATE.enable(&path, append)? {
        return Ok(path);
    }

    log(if append {
        "diagnostic logging enabled (append)"
    } else {
        "diagnostic logging enabled"
    });
    log(format!(
        "app version={} executable={}",
        env!("CARGO_PKG_VERSION"),
        std::env::current_exe().unwrap_or_default().display()
    ));
    Ok(path)
}

fn open_log(path: &std::path::Path, append: bool) -> std::io::Result<File> {
    // Both processes must always append, even after a --diagnose reset.
    // A plain write handle would overwrite lines appended by another process.
    if !append {
        File::create(path)?;
    }
    OpenOptions::new().create(true).append(true).open(path)
}

pub fn is_enabled() -> bool {
    DIAGNOSE_STATE.enabled.load(Ordering::Acquire)
}

pub fn log(message: impl AsRef<str>) {
    if !is_enabled() {
        return;
    }

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);

    let line = format!(
        "[{timestamp}] [pid {}] {}\n",
        std::process::id(),
        message.as_ref()
    );
    DIAGNOSE_STATE.write(line.as_bytes());
}

pub fn log_error(context: &str, error: impl std::fmt::Display) {
    log_lazy(|| format!("{context}: {error}"));
}

pub fn log_lazy(message: impl FnOnce() -> String) {
    if is_enabled() {
        log(message());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_logging_skips_formatting() {
        assert!(!is_enabled());
        log_lazy(|| panic!("disabled logging must not format messages"));
    }

    #[test]
    fn recording_appends_and_ignores_writes_before_enable() {
        let path = std::env::temp_dir().join(format!(
            "ccum-log-record-{}-{}.log",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut monitor = open_log(&path, false).unwrap();
        monitor.write_all(b"monitor\n").unwrap();
        // A second process appends; the first handle must not truncate it.
        let mut other = open_log(&path, true).unwrap();
        other.write_all(b"other\n").unwrap();
        monitor.write_all(b"poll complete\n").unwrap();
        drop(other);
        drop(monitor);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "monitor\nother\npoll complete\n"
        );

        let state = DiagnoseState::new();
        assert!(!state.enabled.load(Ordering::Acquire));
        state.write(b"dropped before enable\n");
        assert!(state.file.lock().unwrap().is_none());
        state.enable(&path, true).unwrap();
        state.write(b"recording on\n");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "monitor\nother\npoll complete\nrecording on\n"
        );
        assert!(state.enable(&path, true).is_ok_and(|opened| !opened));
        std::fs::remove_file(path).unwrap();
    }
}
