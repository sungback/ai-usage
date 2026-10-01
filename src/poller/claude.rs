use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Deserialize;

use super::claude_desktop;
use super::{
    build_agent, get_header_f64, get_header_i64, parse_iso8601, unix_to_system_time, HttpResponse,
    PollError,
};
use super::CommandExtHelper;
use crate::models::{CreditsSection, UsageData};

const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const MESSAGES_URL: &str = "https://api.anthropic.com/v1/messages";
const MODEL_FALLBACK_CHAIN: &[&str] = &["claude-3-haiku-20240307", "claude-haiku-4-5-20251001"];

#[derive(Deserialize)]
struct UsageResponse {
    five_hour: Option<UsageBucket>,
    seven_day: Option<UsageBucket>,
    spend: Option<SpendResponse>,
}

/// 플랜 한도를 초과했을 때 계정을 유지해 주는 유료 크레딧입니다.
/// 금액은 고유한 지수(exponent)를 갖는 마이너 단위(minor unit, 예: 센트)이므로 통화가 자체 기술됩니다.
#[derive(Deserialize)]
struct SpendResponse {
    #[serde(default)]
    enabled: bool,
    used: Option<SpendAmount>,
    limit: Option<SpendAmount>,
}

#[derive(Deserialize)]
struct SpendAmount {
    amount_minor: f64,
    #[serde(default)]
    exponent: u32,
}

impl SpendAmount {
    fn major(&self) -> f64 {
        self.amount_minor / 10f64.powi(self.exponent as i32)
    }
}

#[derive(Deserialize)]
struct UsageBucket {
    utilization: f64,
    resets_at: Option<String>,
}

struct Credentials {
    access_token: String,
    expires_at: Option<i64>,
    source: CredentialSource,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum CredentialSource {
    Windows(PathBuf),
    /// Claude 데스크톱 앱 자체의 토큰 캐시입니다. Claude Code가 데스크톱 앱 내에서만
    /// 실행되었고 CLI 로그인이 `~/.claude/.credentials.json`을 작성하지 않은 경우에 사용됩니다.
    DesktopApp(PathBuf),
    Wsl {
        distro: String,
    },
    Keychain(String),
}

pub(super) fn poll_claude_code() -> Result<UsageData, PollError> {
    let creds = match read_first_credentials() {
        Some(c) => c,
        None => {
            return Err(PollError::NoCredentials);
        }
    };

    let creds = refresh_credentials(creds)?;

    fetch_usage_with_fallback(&creds.access_token)
}

/// 명시적 프로필은 토큰 갱신이 실패하더라도 하나의 소스에 고정(pin)됩니다.
///
/// 유일한 예외는 기본 CLI 경로에 위치한 프로필입니다. 해당 경로는 데스크톱 앱의
/// Claude Code 빌드가 로그인하는 위치이기도 하며, 데스크톱 앱이 로그인을 인계받으면
/// `.credentials.json` 파일은 존재하지만 토큰이 비어 있는 상태가 될 수 있습니다.
/// 여기서 "토큰 없음"을 탐색의 끝으로 취급하면 유효한 데스크톱 토큰을 놓치게 되므로,
/// 기본 경로(오직 기본 경로만)는 데스크톱 앱으로 폴스루(fall through)됩니다.
/// 커스텀 내보내기 경로는 고정 상태를 유지하므로, 다중 계정 설정에서 다른 계정의 토큰을 빌려오는 일은 없습니다.
pub(super) fn poll_account(path: &Path) -> Result<UsageData, PollError> {
    let is_default = crate::accounts::default_credential_path(crate::providers::ProviderId::Claude)
        .map(|default| {
            let explicit = std::env::var_os("CLAUDE_CONFIG_DIR").is_some_and(|value| !value.is_empty());
            desktop_fallback_allowed(path, &default, explicit)
        })
        .unwrap_or(false);

    let mut credentials =
        match read_credentials_from_source(&CredentialSource::Windows(path.to_path_buf())) {
            Some(credentials) => credentials,
            None => {
                #[cfg(target_os = "macos")]
                {
                    if is_default {
                        read_keychain_credentials("Claude Code-credentials")
                            .or_else(|| desktop_credentials_for_default_path(path))
                            .ok_or(PollError::NoCredentials)?
                    } else {
                        return Err(PollError::NoCredentials);
                    }
                }
                #[cfg(not(target_os = "macos"))]
                {
                    if is_default {
                        desktop_credentials_for_default_path(path).ok_or(PollError::NoCredentials)?
                    } else {
                        return Err(PollError::NoCredentials);
                    }
                }
            }
        };

    // 실제로 토큰을 생성한 소스를 대상으로 갱신을 진행합니다.
    let source = credentials.source.clone();
    if is_token_expired(credentials.expires_at) {
        cli_refresh_token(&source);
        credentials = read_credentials_from_source(&source).ok_or(PollError::TokenExpired)?;
        if is_token_expired(credentials.expires_at) {
            return Err(PollError::TokenExpired);
        }
    }
    fetch_usage_with_fallback(&credentials.access_token)
}

/// 기본 CLI 자격 증명 경로를 가리키는 프로필에 한해 데스크톱 앱의 토큰을 가져옵니다.
fn desktop_credentials_for_default_path(path: &Path) -> Option<Credentials> {
    let credentials = read_desktop_app_credentials(&desktop_fallback_path(path)?)?;
    Some(credentials)
}

fn desktop_fallback_path(path: &Path) -> Option<PathBuf> {
    let default = crate::accounts::default_credential_path(crate::providers::ProviderId::Claude)?;
    let explicit = std::env::var_os("CLAUDE_CONFIG_DIR").is_some_and(|value| !value.is_empty());
    desktop_fallback_allowed(path, &default, explicit)
        .then(claude_desktop::config_path)
        .flatten()
}

fn desktop_fallback_allowed(path: &Path, default: &Path, explicit_directory: bool) -> bool {
    // default_credential_path는 CLAUDE_CONFIG_DIR도 반영합니다. 이는
    // 명시적인 계정 선택이며, 데스크톱 로그인을 사용해도 된다는 권한이 아닙니다.
    !explicit_directory && crate::accounts::source_key(path) == crate::accounts::source_key(default)
}

pub(super) fn account_watch_signature(path: &Path) -> String {
    account_watch_signature_with_desktop(path, desktop_fallback_path(path).as_deref())
}

fn account_watch_signature_with_desktop(path: &Path, desktop: Option<&Path>) -> String {
    let signature = crate::accounts::file_signature(path);
    match desktop {
        // 기본 경로 프로필은 두 파일 중 하나를 읽을 수 있습니다. 데스크톱 로그인/교체가
        // 일시 정지된 계정을 재개하고 오래된 사용량을 무효화할 수 있도록 둘 다 감시합니다.
        Some(desktop) => crate::accounts::fingerprint(&format!(
            "{signature}|{}",
            claude_desktop::watch_signature(desktop)
        )),
        None => signature,
    }
}

pub(super) fn fetch_usage_with_fallback(token: &str) -> Result<UsageData, PollError> {
    // 전용 사용량 엔드포인트를 먼저 시도합니다.
    if let Some(data) = try_usage_endpoint(token)? {
        // 리셋 타이머가 누락된 경우 Messages API를 통해 채웁니다.
        if data.session.resets_at.is_none() || data.weekly.resets_at.is_none() {
            if let Ok(fallback) = fetch_usage_via_messages(token) {
                let mut merged = data;
                merged.session.available |= fallback.session.available;
                merged.weekly.available |= fallback.weekly.available;
                if merged.session.resets_at.is_none() {
                    merged.session.resets_at = fallback.session.resets_at;
                }
                if merged.weekly.resets_at.is_none() {
                    merged.weekly.resets_at = fallback.weekly.resets_at;
                }
                return Ok(merged);
            }
        }
        return Ok(data);
    }

    // 속도 제한 헤더가 포함된 Messages API로 폴백합니다.
    fetch_usage_via_messages(token)
}

pub(super) fn try_usage_endpoint(token: &str) -> Result<Option<UsageData>, PollError> {
    let agent = build_agent()?;

    let mut resp = match agent
        .get(USAGE_URL)
        .header("Authorization", &format!("Bearer {token}"))
        .header("anthropic-beta", "oauth-2025-04-20")
        .call()
    {
        Ok(resp) => resp,
        Err(error) => match classify_usage_failure(&error) {
            UsageEndpointFailure::Auth => {
                return Err(usage_request_error(&error));
            }
            UsageEndpointFailure::Transient => {
                return Err(usage_request_error(&error));
            }
            UsageEndpointFailure::Unsupported => {
                return Ok(None);
            }
        },
    };

    let response: UsageResponse = match resp.body_mut().read_json() {
        Ok(response) => response,
        Err(_) => return Ok(None),
    };
    Ok(Some(usage_from_response(response)))
}

fn usage_from_response(response: UsageResponse) -> UsageData {
    let mut data = UsageData::default();

    if let Some(bucket) = &response.five_hour {
        data.session.available = true;
        data.session.percentage = bucket.utilization;
        data.session.resets_at = parse_iso8601(bucket.resets_at.as_deref());
    }

    if let Some(bucket) = &response.seven_day {
        data.weekly.available = true;
        data.weekly.percentage = bucket.utilization;
        data.weekly.resets_at = parse_iso8601(bucket.resets_at.as_deref());
    }

    data.credits = response
        .spend
        .as_ref()
        .and_then(|spend| claude_credits(spend, &data));

    data
}

/// 사용량 엔드포인트 호출 실패 시 실패 원인의 분류입니다.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UsageEndpointFailure {
    /// 자격 증명이 거부되었습니다.
    Auth,
    /// 속도 제한(Rate limited), 서버 오류 또는 네트워크 장애입니다. 나중에 재시도하는 것이
    /// 올바른 대처입니다. Messages API로 대신 요청하면 헤더만 읽으려는 목적임에도
    /// 실제 쿼터를 소비하게 되며, 속도 제한 중에는 부하를 더욱 가중시킵니다.
    Transient,
    /// 해당 계정에서는 이 엔드포인트를 사용할 수 없습니다. Messages API 폴백이 존재하는 이유입니다.
    Unsupported,
}

fn classify_usage_failure(error: &ureq::Error) -> UsageEndpointFailure {
    match error {
        ureq::Error::StatusCode(401 | 403) => UsageEndpointFailure::Auth,
        ureq::Error::StatusCode(429) => UsageEndpointFailure::Transient,
        ureq::Error::StatusCode(code) if *code >= 500 => UsageEndpointFailure::Transient,
        ureq::Error::StatusCode(_) => UsageEndpointFailure::Unsupported,
        _ => UsageEndpointFailure::Transient,
    }
}

fn usage_request_error(error: &ureq::Error) -> PollError {
    match error {
        ureq::Error::StatusCode(status) => PollError::HttpStatus(*status),
        _ => PollError::RequestFailed,
    }
}

/// Codex와 달리 플랜 자체에 상한선이 명시되어 있으므로 게이지에 이전 기록(history)이 필요하지 않습니다.
/// `used`는 현재 한도에 대한 실제 사용액이며, 0이 아닌 값은 Codex 잔액이 줄어드는 것과 마찬가지로
/// "크레딧이 사용 중"임을 나타냅니다. 추가 사용량(extra usage)이 꺼진 계정은
/// 비활성화된 것으로 보고되며, 빈 게이지 대신 게이지 자체가 표시되지 않습니다.
fn claude_credits(spend: &SpendResponse, data: &UsageData) -> Option<CreditsSection> {
    let used = spend.used.as_ref()?.major();
    let total = spend.limit.as_ref()?.major();
    if !spend.enabled || !total.is_finite() || total <= 0.0 {
        return None;
    }

    // 일반 윈도우 중 하나가 모두 소진되어 크레딧이 초과분을 감당하기 시작할 때까지 게이지 표시를 보류합니다.
    let limit_reached = data.session.percentage >= 100.0 || data.weekly.percentage >= 100.0;
    if !limit_reached || used <= 0.0 {
        return None;
    }

    Some(CreditsSection {
        percentage: ((used / total) * 100.0).clamp(0.0, 100.0),
        remaining: (total - used).max(0.0),
        total,
    })
}

pub(super) fn fetch_usage_via_messages(token: &str) -> Result<UsageData, PollError> {
    let agent = build_agent()?;
    let mut last_error = PollError::RequestFailed;

    for model in MODEL_FALLBACK_CHAIN {
        let body = serde_json::json!({
            "model": model,
            "max_tokens": 1,
            "messages": [{"role": "user", "content": "."}]
        });

        let response = match agent
            .post(MESSAGES_URL)
            .header("Authorization", &format!("Bearer {token}"))
            .header("anthropic-version", "2023-06-01")
            .header("anthropic-beta", "oauth-2025-04-20")
            .config()
            .http_status_as_error(false)
            .build()
            .send_json(&body)
        {
            Ok(resp) => resp,
            Err(error) => {
                last_error = usage_request_error(&error);
                continue;
            }
        };

        let status = response.status().as_u16();
        if status == 401 || status == 403 {
            return Err(PollError::HttpStatus(status));
        }

        let h5 = response
            .headers()
            .get("anthropic-ratelimit-unified-5h-utilization");
        let h7 = response
            .headers()
            .get("anthropic-ratelimit-unified-7d-utilization");
        let hs = response.headers().get("anthropic-ratelimit-unified-status");

        if h5.is_some() || h7.is_some() || hs.is_some() {
            return Ok(parse_rate_limit_headers(&response));
        }
        last_error = if response.status().is_client_error() || response.status().is_server_error() {
            PollError::HttpStatus(status)
        } else {
            PollError::RequestFailed
        };
    }

    Err(last_error)
}

pub(super) fn parse_rate_limit_headers(response: &HttpResponse) -> UsageData {
    let mut data = UsageData::default();

    data.session.percentage =
        get_header_f64(response, "anthropic-ratelimit-unified-5h-utilization") * 100.0;
    data.session.resets_at = unix_to_system_time(get_header_i64(
        response,
        "anthropic-ratelimit-unified-5h-reset",
    ));

    data.weekly.percentage =
        get_header_f64(response, "anthropic-ratelimit-unified-7d-utilization") * 100.0;
    data.weekly.resets_at = unix_to_system_time(get_header_i64(
        response,
        "anthropic-ratelimit-unified-7d-reset",
    ));
    data.session.available = data.session.resets_at.is_some()
        || response
            .headers()
            .contains_key("anthropic-ratelimit-unified-5h-utilization");
    data.weekly.available = data.weekly.resets_at.is_some()
        || response
            .headers()
            .contains_key("anthropic-ratelimit-unified-7d-utilization");

    let overall_reset = get_header_i64(response, "anthropic-ratelimit-unified-reset");
    let claim = response
        .headers()
        .get("anthropic-ratelimit-unified-representative-claim")
        .and_then(|value| value.to_str().ok());
    data.session.available |= claim == Some("five_hour");
    data.weekly.available |= claim == Some("seven_day");

    if data.session.percentage == 0.0 && data.weekly.percentage == 0.0 {
        let status = response
            .headers()
            .get("anthropic-ratelimit-unified-status")
            .and_then(|value| value.to_str().ok());
        if status == Some("rejected") {
            match claim {
                Some("five_hour") => data.session.percentage = 100.0,
                Some("seven_day") => data.weekly.percentage = 100.0,
                _ => {}
            }
        }

        if data.session.resets_at.is_none() && overall_reset.is_some() {
            data.session.resets_at = unix_to_system_time(overall_reset);
            // 기존 리셋 바인딩을 유지하되, 공유 리셋만으로는
            // 5시간 윈도우가 존재한다고 단정할 수 없습니다.
        }
    }

    data
}

pub(super) fn credential_watch_snapshot(all_sources: bool) -> Vec<String> {
    let sources = if all_sources {
        all_known_credential_sources()
    } else {
        read_first_credentials()
            .map(|credentials| vec![credentials.source])
            .unwrap_or_else(all_known_credential_sources)
    };

    let mut snapshot: Vec<String> = sources
        .into_iter()
        .filter_map(|source| credential_watch_signature(&source))
        .collect();
    snapshot.sort();
    snapshot.dedup();
    snapshot
}

fn refresh_credentials(credentials: Credentials) -> Result<Credentials, PollError> {
    if !is_token_expired(credentials.expires_at) {
        return Ok(credentials);
    }
    let source = credentials.source;
    cli_refresh_token(&source);
    // 만료된 로그인은 여전히 선택된 계정입니다. 갱신에 실패하더라도
    // Desktop이나 WSL에서 발견된 다른 계정으로 대체하지 않습니다.
    read_credentials_from_source(&source)
        .filter(|credentials| !is_token_expired(credentials.expires_at))
        .ok_or(PollError::TokenExpired)
}

fn cli_refresh_token(source: &CredentialSource) {
    match source {
        CredentialSource::Windows(path) => {
            // CLI는 이 파일명만 직접 관리합니다. 커스텀 내보내기 파일은 읽기 전용입니다.
            if path
                .file_name()
                .is_some_and(|name| name == ".credentials.json")
            {
                if let Some(directory) = path.parent() {
                    cli_refresh_windows_token(directory);
                }
            }
        }
        // 데스크톱 앱이 이 토큰을 소유하고 스스로 갱신하므로, 여기서 직접 제어할 작업은 없습니다.
        // 캐시를 다시 읽는 것 자체가 전체 재시도 과정입니다.
        CredentialSource::DesktopApp(_) => {}
        CredentialSource::Wsl { distro } => cli_refresh_wsl_token(distro),
        CredentialSource::Keychain(_) => {
            #[cfg(target_os = "macos")]
            let _ = std::process::Command::new("claude")
                .args(["-p", "."])
                .output();
        }
    }
}

fn cli_refresh_windows_token(directory: &Path) {
    let claude_path = resolve_windows_claude_path();
    let is_cmd = claude_path.to_lowercase().ends_with(".cmd");

    let args: &[&str] = &["-p", "."];
    let mut command = if is_cmd {
        let mut command = Command::new("cmd.exe");
        command.arg("/c").arg(&claude_path).args(args);
        command
    } else {
        let mut command = Command::new(&claude_path);
        command.args(args);
        command
    };
    command
        .env("CLAUDE_CONFIG_DIR", directory)
        .env_remove("CLAUDECODE")
        .env_remove("CLAUDE_CODE_ENTRYPOINT")
        .no_window()
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(_) => {
            return;
        }
    };
    super::wait_for_refresh_exit(&mut child, Duration::from_secs(30));
}

fn cli_refresh_wsl_token(distro: &str) {
    let mut command = Command::new("wsl.exe");
    command
        .arg("-d")
        .arg(distro)
        .arg("--")
        .arg("bash")
        .arg("-lic")
        .arg("export CLAUDE_CONFIG_DIR=\"$HOME/.claude\"; if command -v claude >/dev/null 2>&1; then claude -p .; elif [ -x \"$HOME/.local/bin/claude\" ]; then \"$HOME/.local/bin/claude\" -p .; else exit 127; fi")
        .env_remove("CLAUDECODE")
        .env_remove("CLAUDE_CODE_ENTRYPOINT")
        .no_window()
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(_) => {
            return;
        }
    };
    super::wait_for_refresh_exit(&mut child, Duration::from_secs(30));
}

#[cfg(not(windows))]
fn resolve_windows_claude_path() -> String {
    "claude".to_string()
}

#[cfg(windows)]
fn resolve_windows_claude_path() -> String {
    for name in ["claude.cmd", "claude"] {
        if Command::new(name)
            .arg("--version")
            .no_window()
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok()
        {
            return name.to_string();
        }
    }

    for name in ["claude.cmd", "claude"] {
        if let Ok(output) = Command::new("where.exe")
            .arg(name)
            .no_window()
            .output()
        {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                if let Some(path) = stdout
                    .lines()
                    .next()
                    .map(str::trim)
                    .filter(|path| !path.is_empty())
                {
                    return path.to_string();
                }
            }
        }
    }

    if let Some(bundled) = bundled_desktop_claude_path() {
        return bundled.to_string_lossy().into_owned();
    }

    "claude.cmd".to_string()
}

/// 데스크톱 앱은 `%APPDATA%\Claude\claude-code\<version>\claude.exe` 경로에 자체 Claude Code 빌드를
/// 함께 제공하며, 독립 실행형 CLI를 설치한 적이 없을 때 존재하는 유일한 Claude 바이너리입니다.
#[cfg_attr(not(windows), allow(dead_code))]
fn bundled_desktop_claude_path() -> Option<PathBuf> {
    let versions = dirs::config_dir()?.join("Claude").join("claude-code");
    let mut candidates: Vec<PathBuf> = std::fs::read_dir(versions)
        .ok()?
        .flatten()
        .map(|entry| entry.path().join("claude.exe"))
        .filter(|path| path.is_file())
        .collect();
    // 디렉터리 순서가 버전 순서와 일치하지 않으므로 가장 최신 설치본을 선택합니다.
    candidates.sort_by(|left, right| {
        bundled_claude_version(left)
            .cmp(&bundled_claude_version(right))
            .then_with(|| left.cmp(right))
    });
    candidates.pop()
}

#[cfg_attr(not(any(windows, test)), allow(dead_code))]
fn bundled_claude_version(path: &Path) -> Option<Vec<u64>> {
    path.parent()?
        .file_name()?
        .to_str()?
        .split('.')
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()
}

fn read_first_credentials() -> Option<Credentials> {
    credential_sources_in_order().find_map(|source| read_credentials_from_source(&source))
}

fn read_windows_credentials(path: &Path) -> Option<Credentials> {
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(_) => {
            return None;
        }
    };
    parse_credentials(&content, CredentialSource::Windows(path.to_path_buf()))
}

fn read_desktop_app_credentials(path: &Path) -> Option<Credentials> {
    let token = claude_desktop::read_token(path)?;
    Some(Credentials {
        access_token: token.access_token,
        expires_at: token.expires_at,
        source: CredentialSource::DesktopApp(path.to_path_buf()),
    })
}

fn read_keychain_credentials(service: &str) -> Option<Credentials> {
    #[cfg(target_os = "macos")]
    {
        let output = std::process::Command::new("security")
            .args(["find-generic-password", "-s", service, "-w"])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let content = String::from_utf8_lossy(&output.stdout).trim().to_string();
        parse_credentials(&content, CredentialSource::Keychain(service.to_string()))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = service;
        None
    }
}

fn read_credentials_from_source(source: &CredentialSource) -> Option<Credentials> {
    match source {
        CredentialSource::Windows(path) => read_windows_credentials(path),
        CredentialSource::DesktopApp(path) => read_desktop_app_credentials(path),
        CredentialSource::Wsl { distro } => read_wsl_credentials(distro),
        CredentialSource::Keychain(service) => read_keychain_credentials(service),
    }
}

fn read_wsl_credentials(distro: &str) -> Option<Credentials> {
    let output = run_with_timeout(
        Command::new("wsl.exe")
            .arg("-d")
            .arg(distro)
            .arg("--")
            .arg("sh")
            .arg("-lc")
            .arg("cat ~/.claude/.credentials.json")
            .no_window()
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null()),
        Duration::from_secs(5),
    )?;

    if !output.status.success() {
        return None;
    }

    let content = String::from_utf8(output.stdout).ok()?;
    parse_credentials(
        &content,
        CredentialSource::Wsl {
            distro: distro.to_string(),
        },
    )
}

fn parse_credentials(content: &str, source: CredentialSource) -> Option<Credentials> {
    let json: serde_json::Value = serde_json::from_str(content).ok()?;
    let oauth = json.get("claudeAiOauth")?;
    Some(Credentials {
        access_token: oauth
            .get("accessToken")?
            .as_str()
            .filter(|token| !token.trim().is_empty())?
            .to_string(),
        expires_at: oauth.get("expiresAt").and_then(|value| value.as_i64()),
        source,
    })
}

/// 자격 증명 소스를 비용이 적게 드는 순서대로 탐색합니다. WSL 검사는 지연(lazy) 실행되므로,
/// 로컬에서 토큰을 찾은 기기에서는 `wsl.exe`를 실행할 필요가 없습니다.
fn credential_sources_in_order() -> impl Iterator<Item = CredentialSource> {
    let explicit = std::env::var_os("CLAUDE_CONFIG_DIR").is_some_and(|value| !value.is_empty());
    let keychain = if !explicit && cfg!(target_os = "macos") {
        Some(CredentialSource::Keychain("Claude Code-credentials".to_string()))
    } else {
        None
    };
    keychain
        .into_iter()
        .chain(windows_credential_source())
        .chain((!explicit).then(desktop_app_credential_source).flatten())
        .chain(
            std::iter::once_with(move || {
                if explicit {
                    Vec::new()
                } else {
                    list_wsl_distros()
                }
            })
            .flatten()
            .map(|distro| CredentialSource::Wsl { distro }),
        )
}

fn all_known_credential_sources() -> Vec<CredentialSource> {
    credential_sources_in_order().collect()
}

fn windows_credential_source() -> Option<CredentialSource> {
    if std::env::var_os("CLAUDE_CONFIG_DIR").is_some_and(|value| !value.is_empty()) {
        return crate::accounts::environment_directory(crate::providers::ProviderId::Claude)
            .map(|directory| CredentialSource::Windows(directory.join(".credentials.json")));
    }
    Some(CredentialSource::Windows(
        dirs::home_dir()?.join(".claude").join(".credentials.json"),
    ))
}

fn desktop_app_credential_source() -> Option<CredentialSource> {
    claude_desktop::config_path().map(CredentialSource::DesktopApp)
}

pub(super) fn native_credential_path() -> Option<PathBuf> {
    match windows_credential_source()? {
        CredentialSource::Windows(path) if read_windows_credentials(&path).is_some() => Some(path),
        _ => None,
    }
}

fn credential_watch_signature(source: &CredentialSource) -> Option<String> {
    match source {
        CredentialSource::Windows(path) => Some(windows_credential_watch_signature(path)),
        CredentialSource::DesktopApp(path) => Some(claude_desktop::watch_signature(path)),
        CredentialSource::Wsl { distro } => wsl_credential_watch_signature(distro),
        CredentialSource::Keychain(service) => {
            read_keychain_credentials(service)
                .map(|c| format!("keychain|{service}|{}", c.access_token.chars().take(12).collect::<String>()))
        }
    }
}

fn windows_credential_watch_signature(path: &PathBuf) -> String {
    let key = format!("win:{}", path.display());
    match std::fs::metadata(path) {
        Ok(metadata) => {
            let modified = metadata
                .modified()
                .ok()
                .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
                .map(|value| value.as_nanos())
                .unwrap_or(0);
            format!("{key}|present|{}|{modified}", metadata.len())
        }
        Err(_) => format!("{key}|missing"),
    }
}

fn wsl_credential_watch_signature(distro: &str) -> Option<String> {
    let output = run_with_timeout(
        Command::new("wsl.exe")
            .arg("-d")
            .arg(distro)
            .arg("--")
            .arg("sh")
            .arg("-lc")
            .arg(
                "if [ -f ~/.claude/.credentials.json ]; then stat -c 'present|%s|%Y' ~/.claude/.credentials.json; else echo missing; fi",
            )
            .no_window()
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null()),
        Duration::from_secs(5),
    )?;
    let state = if output.status.success() {
        decode_wsl_text(&output.stdout).trim().to_string()
    } else {
        format!("status-{}", output.status)
    };
    Some(format!("wsl:{distro}|{state}"))
}

#[cfg(not(windows))]
fn list_wsl_distros() -> Vec<String> {
    Vec::new()
}

#[cfg(windows)]
fn list_wsl_distros() -> Vec<String> {
    let output = match run_with_timeout(
        Command::new("wsl.exe")
            .args(["-l", "-q"])
            .no_window()
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null()),
        Duration::from_secs(5),
    ) {
        Some(output) if output.status.success() => output,
        _ => {
            return Vec::new();
        }
    };
    decode_wsl_text(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn decode_wsl_text(bytes: &[u8]) -> String {
    decode_utf16le(bytes).unwrap_or_else(|| String::from_utf8_lossy(bytes).into_owned())
}

fn decode_utf16le(bytes: &[u8]) -> Option<String> {
    if bytes.len() < 2 || !bytes.len().is_multiple_of(2) {
        return None;
    }
    let body = if bytes.starts_with(&[0xFF, 0xFE]) {
        &bytes[2..]
    } else if looks_like_utf16le(bytes) {
        bytes
    } else {
        return None;
    };
    Some(String::from_utf16_lossy(
        &body
            .as_chunks::<2>()
            .0
            .iter()
            .map(|chunk| u16::from_le_bytes(*chunk))
            .collect::<Vec<_>>(),
    ))
}

fn looks_like_utf16le(bytes: &[u8]) -> bool {
    let sample_len = bytes.len().min(128);
    let units = sample_len / 2;
    units > 0
        && bytes[..sample_len]
            .as_chunks::<2>()
            .0
            .iter()
            .filter(|chunk| chunk[1] == 0)
            .count()
            * 2
            >= units
}

fn is_token_expired(expires_at: Option<i64>) -> bool {
    expires_at.is_some_and(|expires_at| {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        now >= expires_at
    })
}

fn run_with_timeout(command: &mut Command, timeout: Duration) -> Option<std::process::Output> {
    let mut child = command.spawn().ok()?;
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return child.wait_with_output().ok(),
            Ok(None) if start.elapsed() > timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
            Err(_) => return None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// 기본 경로 프로필은 두 파일 중 하나를 읽을 수 있으므로, 데스크톱 토큰 캐시의
    /// 교체만으로도 감시 시그니처가 변경되어야 합니다.
    #[test]
    fn the_watch_signature_tracks_the_desktop_token_cache_too() {
        let directory = std::env::temp_dir().join(format!(
            "claude-watch-signature-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let cli = directory.join(".credentials.json");
        let desktop = directory.join("config.json");
        let missing = directory.join("absent.json");
        std::fs::write(&cli, "cli fixture").unwrap();

        // 데스크톱 폴백이 없으면 시그니처는 CLI 파일에만 의존하므로,
        // 데스크톱 캐시가 생성되거나 교체되어도 변경되지 않아야 합니다.
        let cli_only = account_watch_signature_with_desktop(&cli, None);
        std::fs::write(&desktop, "{}").unwrap();
        let empty_desktop = account_watch_signature_with_desktop(&cli, Some(&desktop));
        assert_ne!(
            cli_only, empty_desktop,
            "folding a desktop cache in must change the signature"
        );

        std::fs::write(&desktop, r#"{"oauth:tokenCacheV2":"desktop-token-v1"}"#).unwrap();
        let rotated = account_watch_signature_with_desktop(&cli, Some(&desktop));
        assert_ne!(
            empty_desktop, rotated,
            "rotating the desktop token cache must invalidate the watch"
        );
        assert_eq!(
            rotated,
            account_watch_signature_with_desktop(&cli, Some(&desktop)),
            "the signature must be stable while nothing rotates"
        );
        assert_eq!(
            cli_only,
            account_watch_signature_with_desktop(&cli, None),
            "no desktop fallback must ignore the desktop cache entirely"
        );
        assert_ne!(
            rotated,
            account_watch_signature_with_desktop(&cli, Some(&missing)),
            "a missing desktop cache must differ from a populated one"
        );

        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn http_failures_keep_their_status_for_account_display() {
        for status in [401, 403, 429, 500, 503] {
            assert_eq!(
                usage_request_error(&ureq::Error::StatusCode(status)),
                PollError::HttpStatus(status)
            );
        }
    }

    #[test]
    #[cfg(target_os = "macos")]
    #[ignore = "reads the live Claude Code token from this machine's Keychain"]
    fn test_read_keychain_credentials() {
        if let Some(creds) = read_keychain_credentials("Claude Code-credentials") {
            assert!(!creds.access_token.is_empty());
            assert!(creds.access_token.starts_with("sk-ant-"));
        }
    }

    /// 기본적으로 무시됨(ignored): 데스크톱 앱에만 토큰이 있는 기기에서 기본 프로필이
    /// 사용량을 정상 조회하는지 검증합니다. 데스크톱 앱에 로그인된 상태에서
    /// `cargo test -- --ignored`로 실행하세요.
    #[test]
    #[ignore = "requires a signed-in Claude desktop app on this machine"]
    fn the_default_profile_resolves_usage_from_the_desktop_app() {
        let path = crate::accounts::default_credential_path(crate::providers::ProviderId::Claude)
            .expect("a default credential path");
        let outcome = poll_account(&path);
        assert!(
            outcome.is_ok(),
            "the default profile should resolve usage from the desktop app, got {outcome:?}"
        );
    }

    #[test]
    fn a_custom_export_never_falls_back_to_the_desktop_app() {
        // 데스크톱 폴백 범위는 기본 CLI 경로로 제한됩니다. 다른 위치를 가리키는
        // 프로필은 데스크톱 앱에 사용 가능한 토큰이 있더라도 해당 파일에 고정되어야 합니다.
        let path = std::env::temp_dir().join("claude-custom-export.json");
        assert!(desktop_credentials_for_default_path(&path).is_none());
    }

    #[test]
    fn an_environment_selected_directory_never_uses_the_desktop_login() {
        let native = Path::new("C:/claude-fallback-test/.claude/.credentials.json");
        let custom = Path::new("C:/claude-fallback-test/work/.credentials.json");
        assert!(desktop_fallback_allowed(native, native, false));
        assert!(!desktop_fallback_allowed(custom, native, false));
        // 환경 변수로 선택된 경로 역시 "기본(default)"으로 반환됩니다.
        assert!(!desktop_fallback_allowed(custom, custom, true));
        assert!(!desktop_fallback_allowed(native, native, true));
    }

    #[test]
    fn default_profile_watches_desktop_login_and_rotation_without_window_state_noise() {
        let directory = std::env::temp_dir().join(format!(
            "claude-desktop-watch-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&directory).unwrap();
        let native = directory.join(".credentials.json");
        let desktop = directory.join("config.json");
        // 토큰이 없는 CLI 파일은 데스크톱 로그인 동안에도 변경되지 않고 유지됩니다.
        std::fs::write(&native, r#"{"claudeAiOauth":{"accessToken":""}}"#).unwrap();
        let pinned = account_watch_signature_with_desktop(&native, None);
        assert_eq!(pinned, crate::accounts::file_signature(&native));
        let missing = account_watch_signature_with_desktop(&native, Some(&desktop));
        std::fs::write(
            &desktop,
            r#"{"oauth:tokenCache":"legacy","oauth:tokenCacheV2":"first","window":1}"#,
        )
        .unwrap();
        let logged_in = account_watch_signature_with_desktop(&native, Some(&desktop));
        assert_ne!(missing, logged_in);
        std::fs::write(
            &desktop,
            r#"{"oauth:tokenCache":"legacy","oauth:tokenCacheV2":"first","window":2}"#,
        )
        .unwrap();
        assert_eq!(
            logged_in,
            account_watch_signature_with_desktop(&native, Some(&desktop))
        );
        std::fs::write(
            &desktop,
            r#"{"oauth:tokenCache":"legacy","oauth:tokenCacheV2":"rotated","window":2}"#,
        )
        .unwrap();
        assert_ne!(
            logged_in,
            account_watch_signature_with_desktop(&native, Some(&desktop))
        );
        assert_eq!(pinned, account_watch_signature_with_desktop(&native, None));
        std::fs::remove_file(&desktop).unwrap();
        assert_eq!(
            missing,
            account_watch_signature_with_desktop(&native, Some(&desktop))
        );
        std::fs::remove_file(&native).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn explicit_missing_or_expired_export_never_uses_another_login() {
        let directory = std::env::temp_dir().join(format!(
            "claude-profile-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("export.json");
        assert_eq!(poll_account(&path), Err(PollError::NoCredentials));
        std::fs::write(
            &path,
            r#"{"claudeAiOauth":{"accessToken":"fixture-token","expiresAt":0}}"#,
        )
        .unwrap();
        assert_eq!(poll_account(&path), Err(PollError::TokenExpired));
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn bundled_claude_versions_sort_numerically() {
        let older = bundled_claude_version(Path::new("Claude/claude-code/2.1.9/claude.exe"));
        let newer = bundled_claude_version(Path::new("Claude/claude-code/2.1.10/claude.exe"));

        assert!(newer > older);
    }

    #[test]
    fn bundled_claude_versions_reject_non_numeric_directories() {
        let version = bundled_claude_version(Path::new("Claude/claude-code/current/claude.exe"));

        assert_eq!(version, None);
    }

    fn usage_from_json(json: &str) -> UsageData {
        let response: UsageResponse =
            serde_json::from_str(json).expect("the fixture should deserialize");
        usage_from_response(response)
    }

    #[test]
    fn reported_windows_without_resets_remain_available_at_zero_usage() {
        for percentage in [0.0, 42.0] {
            let data = usage_from_json(&format!(
                r#"{{"five_hour":{{"utilization":{percentage},"resets_at":null}},"seven_day":null}}"#,
            ));
            assert!(data.session.available);
            assert_eq!(data.session.percentage, percentage);
            assert!(data.session.resets_at.is_none());
            assert!(!data.weekly.available);
        }
    }

    #[test]
    fn utilization_headers_report_windows_without_reset_headers() {
        for (header, session, weekly) in [
            ("anthropic-ratelimit-unified-5h-utilization", true, false),
            ("anthropic-ratelimit-unified-7d-utilization", false, true),
            ("anthropic-ratelimit-unified-status", false, false),
        ] {
            let response = ureq::http::Response::builder()
                .header(header, "0")
                .body(ureq::Body::builder().data(Vec::new()))
                .unwrap();
            let data = parse_rate_limit_headers(&response);
            assert_eq!(data.session.available, session);
            assert_eq!(data.weekly.available, weekly);
        }
    }

    #[test]
    fn shared_reset_headers_do_not_invent_a_session_window() {
        for status in ["allowed", "rejected"] {
            for (claim, session, weekly) in [
                ("five_hour", true, false),
                ("seven_day", false, true),
                ("unknown", false, false),
            ] {
                let response = ureq::http::Response::builder()
                    .header("anthropic-ratelimit-unified-status", status)
                    .header("anthropic-ratelimit-unified-reset", "1787198224")
                    .header("anthropic-ratelimit-unified-representative-claim", claim)
                    .body(ureq::Body::builder().data(Vec::new()))
                    .unwrap();
                let data = parse_rate_limit_headers(&response);
                assert_eq!(data.session.available, session);
                assert_eq!(data.weekly.available, weekly);
            }
        }
    }

    fn status_error(code: u16) -> ureq::Error {
        ureq::Error::StatusCode(code)
    }

    #[test]
    fn rate_limits_and_server_faults_do_not_trigger_the_messages_fallback() {
        // 속도 제한에 걸렸을 때 Messages API로 쿼터를 쓰는 것은 잘못된 대처이며
        // 오히려 원인이 된 과부하를 가중시킵니다.
        assert_eq!(
            classify_usage_failure(&status_error(429)),
            UsageEndpointFailure::Transient
        );
        assert_eq!(
            classify_usage_failure(&status_error(500)),
            UsageEndpointFailure::Transient
        );
        assert_eq!(
            classify_usage_failure(&status_error(503)),
            UsageEndpointFailure::Transient
        );
    }

    #[test]
    fn rejected_credentials_are_kept_separate_from_an_absent_endpoint() {
        assert_eq!(
            classify_usage_failure(&status_error(401)),
            UsageEndpointFailure::Auth
        );
        assert_eq!(
            classify_usage_failure(&status_error(403)),
            UsageEndpointFailure::Auth
        );
        // 404 상태 코드는 Messages API 폴백이 대응하도록 설계된 경우입니다.
        assert_eq!(
            classify_usage_failure(&status_error(404)),
            UsageEndpointFailure::Unsupported
        );
    }

    #[test]
    fn spend_becomes_a_credit_gauge_against_the_plan_cap() {
        // 실제 /api/oauth/usage 응답 형태를 기반으로 한 픽스처입니다.
        let data = usage_from_json(
            r#"{
                "seven_day": {"utilization": 100.0, "resets_at": null},
                "spend": {
                    "used": {"amount_minor": 1359, "currency": "USD", "exponent": 2},
                    "limit": {"amount_minor": 5000, "currency": "USD", "exponent": 2},
                    "percent": 27,
                    "enabled": true
                }
            }"#,
        );

        let credits = data.credits.expect("enabled spend should expose a gauge");
        assert!((credits.percentage - 27.18).abs() < 0.01, "{credits:?}");
        assert!((credits.remaining - 36.41).abs() < 0.001, "{credits:?}");
        assert_eq!(credits.total, 50.0);
    }

    #[test]
    fn disabled_or_uncapped_spend_gets_no_gauge() {
        assert!(usage_from_json(
            r#"{"seven_day": {"utilization": 100.0},
                "spend": {"used": {"amount_minor": 0, "exponent": 2},
                          "limit": {"amount_minor": 5000, "exponent": 2}, "enabled": false}}"#
        )
        .credits
        .is_none());

        assert!(usage_from_json(
            r#"{"seven_day": {"utilization": 100.0},
                "spend": {"used": {"amount_minor": 10, "exponent": 2},
                          "limit": {"amount_minor": 0, "exponent": 2}, "enabled": true}}"#
        )
        .credits
        .is_none());

        assert!(usage_from_json(r#"{"seven_day": {"utilization": 1.0}}"#)
            .credits
            .is_none());
    }

    #[test]
    fn the_gauge_waits_for_a_spent_window_and_for_credits_to_be_in_play() {
        let spend = r#""spend": {"used": {"amount_minor": 1359, "exponent": 2},
                                 "limit": {"amount_minor": 5000, "exponent": 2}, "enabled": true}"#;

        // 두 윈도우 모두 여유가 있으므로 게이지는 일반 한도 상태를 유지합니다.
        let json = format!(r#"{{"five_hour": {{"utilization": 40.0}}, {spend}}}"#);
        assert!(usage_from_json(&json).credits.is_none());

        // 5시간 윈도우 소진만으로 충분하며, 주간 윈도우까지 소진될 필요는 없습니다.
        let json = format!(r#"{{"five_hour": {{"utilization": 100.0}}, {spend}}}"#);
        assert!(usage_from_json(&json).credits.is_some());

        // 윈도우는 소진되었으나 아직 크레딧 청구는 시작되지 않은 상태입니다.
        let json = r#"{"five_hour": {"utilization": 100.0},
                       "spend": {"used": {"amount_minor": 0, "exponent": 2},
                                 "limit": {"amount_minor": 5000, "exponent": 2},
                                 "enabled": true}}"#;
        assert!(usage_from_json(json).credits.is_none());
    }
}
