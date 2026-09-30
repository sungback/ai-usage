//! 사용량 폴러 모음 — 처음 보시는 분을 위한 안내.
//!
//! - 각 AI 도구(Claude·Codex·Antigravity·OpenCode·Cursor)마다 아래 `mod` 하나가
//!   "사용량을 가져오는 법"을 들고 있습니다.
//! - 이 파일은 심부름꾼입니다. 켜진 공급자들을 최대 3개까지 동시에 물어보고,
//!   결과를 하나로 모아 돌려줍니다.
//! - 하나가 실패해도 나머지가 살아 있으면 성공으로 봅니다. 실패한 자리는
//!   `carry_forward_failures`가 직전 값을 "오래된 값" 표시와 함께 살려 둡니다.

use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::models::{AppUsageData, UsageData, UsageSection};
use crate::providers::{ProviderId, ProviderSet};

/// 폴링이 실패한 이유입니다. 화면의 "로그인 필요·토큰 만료" 표시에 쓰입니다.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PollError {
    AuthRequired,
    NoCredentials,
    TokenExpired,
    RequestFailed,
    /// 마지막 HTTP 실패 상태를 보존하여 계정 상태 표시에 활용합니다.
    HttpStatus(u16),
}

impl PollError {
    /// 로그인·권한 문제인지 봅니다. 401·403도 여기로 봅니다.
    /// [필수 주석: 인증 실패 분류 - 재시도해도 소용없어 로그인 유도가 먼저입니다]
    pub fn is_auth(self) -> bool {
        matches!(
            self,
            Self::AuthRequired | Self::TokenExpired | Self::HttpStatus(401 | 403)
        )
    }

    #[cfg_attr(not(any(windows, test)), allow(dead_code))]
    pub fn is_transient(self) -> bool {
        matches!(self, Self::RequestFailed | Self::HttpStatus(_)) && !self.is_auth()
    }
}

/// 자격 증명 파일 감시 방식입니다. 지금 쓰는 파일만 볼지, 전부를 볼지 정합니다.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CredentialWatchMode {
    ActiveSource(ProviderId),
    #[cfg_attr(not(windows), allow(dead_code))]
    AllSources(ProviderId),
}

pub type CredentialWatchSnapshot = Vec<String>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PollFailure {
    pub provider: ProviderId,
    pub error: PollError,
}

/// 폴러와 캐시 리더는 계정이 접근할 수 있는 모든 파일에 대해 동일한 기준을 가져야 합니다.
pub fn account_source_signature(provider: ProviderId, path: &std::path::Path) -> String {
    match provider {
        ProviderId::Claude => claude::account_watch_signature(path),
        _ => crate::accounts::file_signature(path),
    }
}

pub fn poll(
    enabled_providers: ProviderSet,
    settings: &crate::accounts::AccountSettings,
    previous: Option<&AppUsageData>,
    force: bool,
) -> Result<AppUsageData, PollFailure> {
    if enabled_providers
        .iter()
        .any(|provider| settings.get(provider).is_some())
    {
        accounts::poll_accounts(enabled_providers, settings, previous, force)
    } else {
        poll_concurrently_with(enabled_providers, poll_provider)
    }
}

/// 이번 주기에 실패한 공급자가 있으면 이전 데이터를 유지합니다.
///
/// 최소 하나의 공급자라도 응답하면 전체 폴링은 성공으로 간주됩니다.
/// 이를 유지하지 않으면, 한 공급자에 장애가 발생했을 때 다른 공급자가 업데이트되는 동안
/// 장애가 발생한 공급자의 화면 표시는 매번 빈칸이 됩니다.
/// 유지된 데이터는 최신 데이터로 취급되지 않고 오래된 값(stale)으로 표시됩니다.
#[cfg_attr(not(any(windows, test)), allow(dead_code))]
pub fn carry_forward_failures(
    fresh: AppUsageData,
    previous: &AppUsageData,
    enabled: ProviderSet,
) -> AppUsageData {
    let mut merged = fresh;
    accounts::carry_accounts(&mut merged, previous);
    for provider in enabled.iter() {
        if merged
            .accounts
            .iter()
            .any(|account| account.provider == provider)
            || previous
                .accounts
                .iter()
                .any(|account| account.provider == provider)
        {
            continue;
        }
        if merged.get(provider).is_some() {
            continue;
        }
        if let Some(last) = previous.get(provider) {
            let mut carried = last.clone();
            carried.stale = true;
            merged.insert(provider, carried);
        }
    }
    merged
}

fn poll_with(
    enabled_providers: ProviderSet,
    mut poll_provider: impl FnMut(ProviderId) -> Result<UsageData, PollError>,
) -> Result<AppUsageData, PollFailure> {
    let results = enabled_providers
        .iter()
        .map(|provider| (provider, poll_provider(provider)))
        .collect::<Vec<_>>();
    merge_poll_results(enabled_providers, results)
}

const MAX_CONCURRENT_PROVIDER_POLLS: usize = 3;

/// 실패한 폴링 주기를 재시도하기 전의 백오프(대기 시간) 간격입니다.
/// Windows 메시지 루프와 macOS 폴링 스레드 간에 공유되어, 
/// 양쪽 플랫폼 모두 30초, 60초, 120초... 순으로 증가하며 전체 폴링 주기를 초과하지 않습니다.
pub fn poll_retry_backoff_ms(retry_count: u32, poll_interval_ms: u32) -> u32 {
    const RETRY_BASE_MS: u32 = 30_000;
    let shift = retry_count.saturating_sub(1).min(u32::BITS);
    RETRY_BASE_MS
        .saturating_mul(1u32.checked_shl(shift).unwrap_or(u32::MAX))
        .min(poll_interval_ms)
}

/// 자식 프로세스(새로고침)가 종료될 때까지 대기하고, 일정 시간이 지나면 종료합니다.
/// `timeout`. Shared by the provider token-refresh spawns.
pub(crate) fn wait_for_refresh_exit(child: &mut std::process::Child, timeout: Duration) {
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() > timeout => {
                let _ = child.kill();
                break;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(500)),
            Err(_) => break,
        }
    }
}

fn poll_concurrently_with<F>(
    enabled_providers: ProviderSet,
    poll_provider: F,
) -> Result<AppUsageData, PollFailure>
where
    F: Fn(ProviderId) -> Result<UsageData, PollError> + Sync,
{
    let providers = enabled_providers.iter().collect::<Vec<_>>();
    if providers.len() <= 1 {
        return poll_with(enabled_providers, poll_provider);
    }

    let worker_count = providers.len().min(MAX_CONCURRENT_PROVIDER_POLLS);
    let next_provider = std::sync::atomic::AtomicUsize::new(0);
    let mut results = std::thread::scope(|scope| {
        let (sender, receiver) = std::sync::mpsc::channel();
        for _ in 0..worker_count {
            let sender = sender.clone();
            let providers = &providers;
            let poll_provider = &poll_provider;
            let next_provider = &next_provider;
            scope.spawn(move || loop {
                let index = next_provider.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let Some(provider) = providers.get(index).copied() else {
                    break;
                };
                if sender.send((provider, poll_provider(provider))).is_err() {
                    break;
                }
            });
        }
        drop(sender);
        receiver.into_iter().collect::<Vec<_>>()
    });
    results.sort_by_key(|(provider, _)| *provider);
    merge_poll_results(enabled_providers, results)
}

fn merge_poll_results(
    enabled_providers: ProviderSet,
    results: impl IntoIterator<Item = (ProviderId, Result<UsageData, PollError>)>,
) -> Result<AppUsageData, PollFailure> {
    let mut data = AppUsageData::default();
    let mut first_error = None;
    for (provider, result) in results {
        match result {
            Ok(usage) => {
                data.insert(provider, usage);
            }
            Err(error) => {
                first_error.get_or_insert(PollFailure { provider, error });
            }
        }
    }

    if data.is_empty() {
        Err(first_error.unwrap_or(PollFailure {
            provider: enabled_providers.first().unwrap_or_default(),
            error: PollError::RequestFailed,
        }))
    } else {
        Ok(data)
    }
}

mod accounts;
mod antigravity;
mod claude;
mod claude_desktop;
mod codex;
mod cursor;
mod opencode;

struct ProviderPoller {
    id: ProviderId,
    poll: fn() -> Result<UsageData, PollError>,
    credential_watch: fn(bool) -> CredentialWatchSnapshot,
}

const PROVIDER_POLLERS: [ProviderPoller; 5] = [
    ProviderPoller {
        id: ProviderId::Claude,
        poll: claude::poll_claude_code,
        credential_watch: claude::credential_watch_snapshot,
    },
    ProviderPoller {
        id: ProviderId::Codex,
        poll: codex::poll_codex,
        credential_watch: codex_credential_watch_snapshot,
    },
    ProviderPoller {
        id: ProviderId::Antigravity,
        poll: antigravity::poll_antigravity,
        credential_watch: antigravity_credential_watch_snapshot,
    },
    ProviderPoller {
        id: ProviderId::OpenCode,
        poll: opencode::poll_opencode,
        credential_watch: opencode::credential_watch_snapshot,
    },
    ProviderPoller {
        id: ProviderId::Cursor,
        poll: cursor::poll_cursor,
        credential_watch: cursor::credential_watch_snapshot,
    },
];

fn provider_poller(provider: ProviderId) -> Option<&'static ProviderPoller> {
    PROVIDER_POLLERS.iter().find(|poller| poller.id == provider)
}

fn poll_provider(provider: ProviderId) -> Result<UsageData, PollError> {
    provider_poller(provider)
        .ok_or(PollError::RequestFailed)
        .and_then(|poller| (poller.poll)())
}

pub fn credential_watch_snapshot(mode: CredentialWatchMode) -> CredentialWatchSnapshot {
    let (provider, all_sources) = match mode {
        CredentialWatchMode::ActiveSource(provider) => (provider, false),
        CredentialWatchMode::AllSources(provider) => (provider, true),
    };
    provider_poller(provider)
        .map(|poller| (poller.credential_watch)(all_sources))
        .unwrap_or_default()
}

fn codex_credential_watch_snapshot(_all_sources: bool) -> CredentialWatchSnapshot {
    codex::credential_watch_snapshot()
}

fn antigravity_credential_watch_snapshot(_all_sources: bool) -> CredentialWatchSnapshot {
    vec![antigravity::antigravity_credential_watch_signature()]
}

fn build_agent() -> Result<ureq::Agent, PollError> {
    static AGENT: OnceLock<Result<ureq::Agent, PollError>> = OnceLock::new();
    // 에이전트 클론들은 커넥션 풀, 쿠키, TLS 설정을 공유합니다.
    AGENT
        .get_or_init(|| {
            let tls = ureq::tls::TlsConfig::builder()
                .provider(ureq::tls::TlsProvider::NativeTls)
                .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                .build();
            Ok(ureq::Agent::config_builder()
                .timeout_global(Some(Duration::from_secs(30)))
                .tls_config(tls)
                .build()
                .into())
        })
        .clone()
}

type HttpResponse = ureq::http::Response<ureq::Body>;

/// 응답 헤더를 문자열로 꺼냅니다. `get_header_f64`·`get_header_i64`의 공통 다리입니다.
fn header_str<'a>(response: &'a HttpResponse, name: &str) -> Option<&'a str> {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
}

fn get_header_f64(response: &HttpResponse, name: &str) -> f64 {
    header_str(response, name)
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.0)
}

fn get_header_i64(response: &HttpResponse, name: &str) -> Option<i64> {
    header_str(response, name).and_then(|s| s.parse::<i64>().ok())
}

fn unix_to_system_time(unix_secs: Option<i64>) -> Option<SystemTime> {
    let secs = unix_secs?;
    if secs < 0 {
        return None;
    }
    Some(UNIX_EPOCH + Duration::from_secs(secs as u64))
}

/// ISO 8601 타임스탬프 문자열을 SystemTime으로 파싱합니다.
fn parse_iso8601(s: Option<&str>) -> Option<SystemTime> {
    let unix_secs = parse_datetime_to_unix(s?)?;
    UNIX_EPOCH.checked_add(Duration::from_secs(unix_secs))
}

/// 최소한의 datetime 파서 — chrono/time 크레이트의 의존성을 피하기 위해 자체 구현합니다.
fn parse_datetime_to_unix(s: &str) -> Option<u64> {
    let (datetime, offset_seconds) = split_timezone(s)?;
    let datetime = match datetime.split_once('.') {
        Some((base, fraction))
            if !fraction.is_empty() && fraction.bytes().all(|b| b.is_ascii_digit()) =>
        {
            base
        }
        Some(_) => return None,
        None => datetime,
    };
    let bytes = datetime.as_bytes();
    if bytes.len() != 19
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
    {
        return None;
    }

    let year = parse_digits(&bytes[0..4])?;
    let month = parse_digits(&bytes[5..7])?;
    let day = parse_digits(&bytes[8..10])?;
    let hour = parse_digits(&bytes[11..13])?;
    let minute = parse_digits(&bytes[14..16])?;
    let second = parse_digits(&bytes[17..19])?;
    if !(1970..=9999).contains(&year)
        || !(1..=12).contains(&month)
        || day == 0
        || day > days_in_month(year, month)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return None;
    }

    let mut days: u64 = 0;
    for y in 1970..year {
        days += if is_leap(y) { 366 } else { 365 };
    }

    let month_days = [0, 31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    for m in 1..month {
        days += month_days[m as usize];
        if m == 2 && is_leap(year) {
            days += 1;
        }
    }
    days += day - 1;

    let local_seconds = days
        .checked_mul(86_400)?
        .checked_add(hour * 3_600 + minute * 60 + second)?;
    u64::try_from(
        i64::try_from(local_seconds)
            .ok()?
            .checked_sub(offset_seconds)?,
    )
    .ok()
}

fn split_timezone(s: &str) -> Option<(&str, i64)> {
    if let Some(datetime) = s.strip_suffix('Z') {
        return Some((datetime, 0));
    }

    if s.len() >= 25 {
        let offset_start = s.len() - 6;
        let offset = &s.as_bytes()[offset_start..];
        if matches!(offset[0], b'+' | b'-') && offset[3] == b':' {
            let hours = parse_digits(&offset[1..3])?;
            let minutes = parse_digits(&offset[4..6])?;
            if hours > 23 || minutes > 59 {
                return None;
            }
            let seconds = i64::try_from(hours * 3_600 + minutes * 60).ok()?;
            return Some((
                &s[..offset_start],
                if offset[0] == b'+' { seconds } else { -seconds },
            ));
        }
    }

    Some((s, 0))
}

fn parse_digits(bytes: &[u8]) -> Option<u64> {
    if bytes.is_empty() || !bytes.iter().all(u8::is_ascii_digit) {
        return None;
    }
    bytes.iter().try_fold(0_u64, |value, byte| {
        value.checked_mul(10)?.checked_add(u64::from(byte - b'0'))
    })
}

fn days_in_month(year: u64, month: u64) -> u64 {
    match month {
        2 if is_leap(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        _ => 0,
    }
}

fn is_leap(y: u64) -> bool {
    (y.is_multiple_of(4) && !y.is_multiple_of(100)) || y.is_multiple_of(400)
}

/// 화면에 표시되는 텍스트가 변경될 때까지 남은 시간을 계산합니다.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn time_until_display_change(resets_at: Option<SystemTime>) -> Option<Duration> {
    let reset = resets_at?;
    let remaining = reset.duration_since(SystemTime::now()).ok()?;
    Some(time_until_display_change_from_secs(remaining.as_secs()))
}

#[cfg_attr(not(windows), allow(dead_code))]
fn time_until_display_change_from_secs(total_secs: u64) -> Duration {
    let total_mins = total_secs / 60;
    let total_hours = total_secs / 3600;
    let total_days = total_secs / 86400;

    let current_bucket_start = if total_days >= 1 {
        total_days * 86400
    } else if total_hours >= 1 {
        total_hours * 3600
    } else if total_mins >= 1 {
        total_mins * 60
    } else {
        total_secs
    };

    Duration::from_secs(total_secs.saturating_sub(current_bucket_start) + 1)
}

/// 두 구간 중 하나라도 "현재(now)"에 도달했는지(초기화 시간이 지났는지) 여부를 반환합니다.
#[cfg_attr(not(any(windows, test)), allow(dead_code))]
pub fn is_past_reset(data: &UsageData) -> bool {
    if data.stale {
        return false;
    }
    let now = SystemTime::now();
    let past = |s: &UsageSection| matches!(s.resets_at, Some(t) if now.duration_since(t).is_ok());
    past(&data.session) || past(&data.weekly)
}

#[cfg_attr(not(any(windows, test)), allow(dead_code))]
pub fn app_is_past_reset(data: &AppUsageData) -> bool {
    data.all_usage().any(is_past_reset)
}

#[cfg(test)]
mod tests;

// ---------------------------------------------------------------------------
// Windows 자식 프로세스 공통 헬퍼
// claude.rs · codex.rs 등 CLI를 호출하는 폴러가 공유합니다.
// ---------------------------------------------------------------------------

#[cfg(windows)]
pub(super) const CREATE_NO_WINDOW: u32 = 0x08000000;

pub(super) trait CommandExtHelper {
    fn no_window(&mut self) -> &mut Self;
}

impl CommandExtHelper for std::process::Command {
    fn no_window(&mut self) -> &mut Self {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            self.creation_flags(CREATE_NO_WINDOW);
        }
        self
    }
}
