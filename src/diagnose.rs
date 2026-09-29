//! 진단 로그(`--diagnose`) — 처음 보시는 분을 위한 안내.
//!
//! - 평소에는 아무 일도 하지 않습니다. `--diagnose`로 켠 뒤에만 파일에 씁니다.
//! - 로그 파일은 임시 폴더의 `ai-usage.log` 하나뿐입니다.
//! - `--diagnose`는 새로 쓰고, `--diagnose-append`는 이어 씁니다.
//! - 두 개의 앱이 동시에 같은 파일에 쓸 수 있어, 항상 "덧붙이기"로만 엽니다.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// 진단 로그의 속마음입니다.
/// - `file`: 열려 있는 로그 파일. `Mutex`는 "한 번에 한 명만 쓰기" 자물쇠입니다.
/// - `enabled`: 켜짐 스위치. `AtomicBool`은 잠금 없이도 스레드끼리 안전하게 봅니다.
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

    /// 로그 파일을 열고 스위치를 켭니다.
    /// 이미 켜져 있으면 `Ok(false)`로 "이미 켜져 있어요"라고만 알립니다.
    fn enable(&self, path: &Path, append: bool) -> Result<bool, String> {
        let mut file = self.file.lock().map_err(|error| error.to_string())?;
        if file.is_some() {
            return Ok(false);
        }
        // [필수 주석: 파일 열기 실패는 문자열로 감싸서 호출자에게 전달 - 진단 중 패닉 금지]
        *file = Some(open_log(path, append).map_err(|error| {
            format!(
                "Unable to open diagnostic log file {}: {error}",
                path.display()
            )
        })?);
        // `Release`로 저장해야 다른 스레드가 `Acquire`로 읽을 때 파일 핸들을 봅니다.
        self.enabled.store(true, Ordering::Release);
        Ok(true)
    }

    /// 한 줄을 파일에 씁니다. 꺼져 있거나 잠금이 깨져 있으면 조용히 넘어갑니다.
    /// 진단을 켰다고 앱이 죽으면 안 되니까, 여기는 절대 패닉하지 않습니다.
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

/// 앱 전체가 함께 쓰는 단 하나의 진단 상태입니다.
static DIAGNOSE_STATE: DiagnoseState = DiagnoseState::new();

/// 지금 시각(초)을 숫자로 돌려줍니다. 실패하면 0초로 둡니다.
fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

/// 로그 파일의 자리입니다. macOS·Windows 모두 임시 폴더를 씁니다.
pub fn log_path() -> PathBuf {
    std::env::temp_dir().join("ai-usage.log")
}

/// 새로 시작하는 진단 로그입니다. 기존 파일은 지워집니다.
pub fn init() -> Result<PathBuf, String> {
    init_file(false)
}

/// 이어 쓰는 진단 로그입니다. 기존 내용은 살려 둡니다.
pub fn init_append() -> Result<PathBuf, String> {
    init_file(true)
}

/// `init`·`init_append`의 공통 본체입니다. 두 번 켜면 두 번째는 그냥 지나갑니다.
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

fn open_log(path: &Path, append: bool) -> std::io::Result<File> {
    // 다른 프로세스가 덧붙인 줄을 지우지 않으려 항상 append로 엽니다.
    // `--diagnose`로 리셋할 때만 먼저 비우고( truncate ), 그 뒤도 append입니다.
    // [필수 주석: 멀티프로세스 append 규약 - 쓰기 핸들로 덮어쓰면 상대방 로그가 날아갑니다]
    if !append {
        File::create(path)?;
    }
    OpenOptions::new().create(true).append(true).open(path)
}

/// 진단 로그가 켜져 있는지 묻습니다.
pub fn is_enabled() -> bool {
    DIAGNOSE_STATE.enabled.load(Ordering::Acquire)
}

/// 한 줄을 기록합니다. 꺼져 있으면 글자를 만드는 수고도 하지 않고 끝납니다.
pub fn log(message: impl AsRef<str>) {
    if !is_enabled() {
        return;
    }

    let line = format!(
        "[{}] [pid {}] {}\n",
        now_secs(),
        std::process::id(),
        message.as_ref()
    );
    DIAGNOSE_STATE.write(line.as_bytes());
}

/// 에러를 `문맥: 원인` 모양으로 한 줄 남깁니다.
pub fn log_error(context: &str, error: impl std::fmt::Display) {
    log_lazy(|| format!("{context}: {error}"));
}

/// 무거운 글자는 켜져 있을 때만 만듭니다.
/// 꺼져 있으면 클로저를 실행조차 하지 않아 공짜입니다.
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
            "ai-usage-log-record-{}-{}.log",
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
