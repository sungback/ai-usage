use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, UNIX_EPOCH};

use serde::Deserialize;

use super::{build_agent, unix_to_system_time, CommandExtHelper, PollError};
use crate::app_settings;
use crate::models::{CodexCreditsState, CreditsSection, UsageData, UsageSection};

const CODEX_USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";

#[derive(Deserialize)]
struct CodexAuthFile {
    tokens: Option<CodexTokenData>,
}

#[derive(Clone, Deserialize)]
struct CodexTokenData {
    access_token: String,
    account_id: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct CodexUsageResponse {
    rate_limit: Option<Option<Box<CodexRateLimitDetails>>>,
    credits: Option<Option<Box<CodexCredits>>>,
}

#[derive(Deserialize)]
struct CodexCredits {
    #[serde(default)]
    has_credits: bool,
    #[serde(default)]
    unlimited: bool,
    #[serde(default)]
    overage_limit_reached: bool,
    /// 통화 단위가 아닌 크레딧 단위의 10진수 문자열로 전달됩니다.
    balance: Option<String>,
}

/// Codex는 1달러당 25크레딧으로 청구합니다. 표시되는 금액만 이 비율에 의존하며,
/// 게이지 바는 의존하지 않습니다. 두 크레딧 수치의 비율은 단위가 없으므로
/// 이 환율이 변경되어도 바가 잘못 표시되지 않습니다.
const CODEX_CREDITS_PER_DOLLAR: f64 = 25.0;

#[derive(Deserialize)]
struct CodexRateLimitDetails {
    primary_window: Option<Option<Box<CodexRateLimitWindow>>>,
    secondary_window: Option<Option<Box<CodexRateLimitWindow>>>,
    /// 어느 윈도우든 하나라도 소진되면 true가 됩니다. 직접 매핑한 윈도우에서
    /// 퍼센티지를 다시 읽는 것보다 낫고, 5시간 윈도우가 다시 활성화되어도 계속 잘 동작합니다.
    #[serde(default)]
    limit_reached: bool,
}

#[derive(Deserialize)]
pub(super) struct CodexRateLimitWindow {
    used_percent: f64,
    reset_at: i64,
    limit_window_seconds: Option<i64>,
}

/// 이 길이 이상의 윈도우는 세션 한도가 아닌 주간 한도로 취급합니다.
/// Codex는 현재 주간에 604800초, 5시간 윈도우에 18000초를 보내므로 1일 이상이면 명확하게 주간입니다.
const WEEKLY_WINDOW_THRESHOLD_SECONDS: i64 = 86_400;

pub(super) fn poll_codex() -> Result<UsageData, PollError> {
    let path = codex_auth_path().ok_or(PollError::NoCredentials)?;
    poll_account(&path)
}

pub(super) fn poll_account(path: &Path) -> Result<UsageData, PollError> {
    let creds = match read_codex_credentials_at(path) {
        Some(creds) => creds,
        None => {
            return Err(PollError::NoCredentials);
        }
    };

    match fetch_codex_usage_at(&creds.access_token, creds.account_id.as_deref(), Some(path)) {
        Ok(data) => Ok(data),
        Err(PollError::AuthRequired) => {
            if path.file_name().is_some_and(|name| name == "auth.json") {
                if let Some(directory) = path.parent() {
                    cli_refresh_codex_token(directory);
                }
            }
            let refreshed = read_codex_credentials_at(path).ok_or(PollError::TokenExpired)?;
            fetch_codex_usage_at(
                &refreshed.access_token,
                refreshed.account_id.as_deref(),
                Some(path),
            )
        }
        Err(error) => Err(error),
    }
}

fn fetch_codex_usage_at(
    token: &str,
    account_id: Option<&str>,
    path: Option<&Path>,
) -> Result<UsageData, PollError> {
    let account_id = account_id.filter(|value| !value.is_empty());
    let agent = build_agent()?;
    let mut request = agent
        .get(CODEX_USAGE_URL)
        .header("Authorization", &format!("Bearer {token}"))
        .header("User-Agent", "codex-cli");

    if let Some(account_id) = account_id {
        request = request.header("ChatGPT-Account-Id", account_id);
    }

    let mut resp = match request.call() {
        Ok(resp) => resp,
        Err(ureq::Error::StatusCode(code)) if code == 401 || code == 403 => {
            return Err(PollError::AuthRequired);
        }
        Err(_) => {
            return Err(PollError::RequestFailed);
        }
    };

    let response: CodexUsageResponse = match resp.body_mut().read_json() {
        Ok(response) => response,
        Err(_) => {
            return Err(PollError::RequestFailed);
        }
    };

    codex_usage_from_response_at(response, account_id, path).ok_or(PollError::RequestFailed)
}

#[cfg(test)]
pub(super) fn codex_usage_from_response(
    response: CodexUsageResponse,
    account_id: Option<&str>,
) -> Option<UsageData> {
    codex_usage_from_response_at(response, account_id, None)
}

fn codex_usage_from_response_at(
    response: CodexUsageResponse,
    account_id: Option<&str>,
    path: Option<&Path>,
) -> Option<UsageData> {
    let credits = response.credits.flatten();
    let details = *response.rate_limit.flatten()?;
    let mut data = UsageData::default();

    // 슬롯 순서가 아니라 윈도우 길이에 따라 할당합니다. Codex는 5시간 윈도우가 꺼져 있을 때
    // `secondary_window`를 비우고 `primary_window`에 주간 한도를 보내기도 하므로,
    // 슬롯 순서만 믿으면 세션 바에 주간 수치가 들어가게 됩니다.
    for (window, default_is_weekly) in [
        (details.primary_window.flatten(), false),
        (details.secondary_window.flatten(), true),
    ]
    .into_iter()
    .filter_map(|(window, default_is_weekly)| window.map(|window| (window, default_is_weekly)))
    {
        let section = codex_section_from_window(&window);
        if window_is_weekly(&window).unwrap_or(default_is_weekly) {
            data.weekly = section;
        } else {
            data.session = section;
        }
    }

    data.credits = credits.and_then(|credits| {
        let state_path = path.map(|path| {
            app_settings::app_data_directory().join(credit_state_file_name(path, account_id))
        });
        let previous = match &state_path {
            Some(path) => std::fs::read(path)
                .ok()
                .and_then(|bytes| serde_json::from_slice(&bytes).ok())
                .or_else(|| {
                    app_settings::load_codex_credits().filter(|state| {
                        account_id.is_some() && state.account_id.as_deref() == account_id
                    })
                }),
            None => app_settings::load_codex_credits(),
        };
        let (state, section) = codex_credits(previous, &credits, details.limit_reached, account_id);
        let saved = match &state_path {
            Some(path) => app_settings::write_json_atomic(path, &state),
            None => app_settings::save_codex_credits(&state),
        };
        let _ = saved;
        section
    });

    Some(data)
}

fn credit_state_file_name(path: &Path, account_id: Option<&str>) -> String {
    format!(
        "codex-credits-{}.json",
        crate::accounts::fingerprint(&format!(
            "{}|{account_id:?}",
            crate::accounts::source_key(path)
        ))
    )
}

/// 폴링 간 잔액을 추적하여 게이지로 변환합니다.
///
/// 잔액은 크레딧이 소비될 때만 감소하므로, 잔액이 증가하는 것은 충전(top-up)을 의미하며
/// 게이지의 기준점(baseline)을 재설정합니다. 바가 숨겨져 있을 때 발생한 충전도
/// 기준점을 이동시켜야 하므로, 게이지 표시 여부와 무관하게 추적은 계속됩니다.
fn codex_credits(
    previous: Option<CodexCreditsState>,
    credits: &CodexCredits,
    limit_reached: bool,
    account_id: Option<&str>,
) -> (CodexCreditsState, Option<CreditsSection>) {
    let balance = credits
        .balance
        .as_deref()
        .and_then(|balance| balance.parse::<f64>().ok())
        .filter(|balance| balance.is_finite() && *balance >= 0.0)
        .unwrap_or_default();

    let previous = previous.filter(|state| state.account_id.as_deref() == account_id);
    let baseline = match previous {
        // 잔액 증가는 충전으로만 발생합니다. 처음 확인한 잔액을 기준점으로 삼으며,
        // 다음 충전으로 바로잡히기 전까지는 손대지 않은 상태로 읽힙니다.
        Some(previous) if balance <= previous.balance => previous.baseline.max(balance),
        _ => balance,
    };
    let state = CodexCreditsState {
        account_id: account_id.map(str::to_owned),
        balance,
        baseline,
    };

    // 두 가지 조건이 동시에 만족될 때까지 바는 일반 윈도우 상태를 유지합니다:
    // 한도가 소진되었고, 현재 충전액에서 실제로 크레딧이 차감되기 시작했을 때입니다.
    // 후자는 공급자가 크레딧을 청구하는 시점에 대한 가정이 아니라 관측된 사실에 기반하며,
    // 유휴 상태에서도 안정적으로 유지되어 변화가 없는 폴링에서 게이지가 깜빡이며 사라지지 않습니다.
    let in_use = balance < baseline;
    let applicable =
        credits.has_credits && !credits.unlimited && limit_reached && in_use && baseline > 0.0;
    if !applicable {
        return (state, None);
    }

    let percentage = if credits.overage_limit_reached {
        100.0
    } else {
        (((baseline - balance) / baseline) * 100.0).clamp(0.0, 100.0)
    };

    (
        state,
        Some(CreditsSection {
            percentage,
            remaining: balance / CODEX_CREDITS_PER_DOLLAR,
            total: baseline / CODEX_CREDITS_PER_DOLLAR,
        }),
    )
}

/// API에서 기간(duration)을 생략한 경우 분류를 반환하지 않습니다. 호출자는
/// 기본 슬롯 매핑(primary는 세션, secondary는 주간)을 유지합니다.
fn window_is_weekly(window: &CodexRateLimitWindow) -> Option<bool> {
    window
        .limit_window_seconds
        .map(|seconds| seconds >= WEEKLY_WINDOW_THRESHOLD_SECONDS)
}

pub(super) fn codex_section_from_window(window: &CodexRateLimitWindow) -> UsageSection {
    UsageSection {
        available: true,
        percentage: window.used_percent,
        resets_at: unix_to_system_time(Some(window.reset_at)),
    }
}

pub(super) fn credential_watch_snapshot() -> Vec<String> {
    let Some(path) = codex_auth_path() else {
        return vec!["codex:auth-path-missing".into()];
    };
    let key = format!("codex:{}", path.display());
    let signature = match std::fs::metadata(path) {
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
    };
    vec![signature]
}

pub(super) fn codex_auth_path() -> Option<PathBuf> {
    if std::env::var_os("CODEX_HOME").is_some_and(|value| !value.is_empty()) {
        let codex_home =
            crate::accounts::environment_directory(crate::providers::ProviderId::Codex)?;
        return Some(codex_home.join("auth.json"));
    }
    Some(dirs::home_dir()?.join(".codex").join("auth.json"))
}

fn read_codex_credentials_at(auth_path: &Path) -> Option<CodexTokenData> {
    let content = match std::fs::read_to_string(auth_path) {
        Ok(content) => content,
        Err(_) => {
            return None;
        }
    };
    let auth: CodexAuthFile = serde_json::from_str(&content).ok()?;
    auth.tokens
        .filter(|tokens| !tokens.access_token.trim().is_empty())
}

fn cli_refresh_codex_token(directory: &Path) {
    let codex_path = resolve_windows_codex_path();
    let is_cmd = codex_path.to_lowercase().ends_with(".cmd");
    let is_ps1 = codex_path.to_lowercase().ends_with(".ps1");

    let args: &[&str] = &["exec", "."];
    let mut command = if is_cmd {
        let mut command = Command::new("cmd.exe");
        command.arg("/c").arg(&codex_path).args(args);
        command
    } else if is_ps1 {
        let mut command = Command::new("powershell.exe");
        command
            .arg("-NoProfile")
            .arg("-ExecutionPolicy")
            .arg("Bypass")
            .arg("-File")
            .arg(&codex_path)
            .args(args);
        command
    } else {
        let mut command = Command::new(&codex_path);
        command.args(args);
        command
    };
    command
        .env("CODEX_HOME", directory)
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
fn resolve_windows_codex_path() -> String {
    for candidate in [
        dirs::home_dir().map(|home| home.join(".local").join("bin").join("codex")),
        Some(PathBuf::from("/opt/homebrew/bin/codex")),
        Some(PathBuf::from("/usr/local/bin/codex")),
    ]
    .into_iter()
    .flatten()
    {
        if candidate.exists() {
            return candidate.to_string_lossy().to_string();
        }
    }
    "codex".to_string()
}

#[cfg(windows)]
fn resolve_windows_codex_path() -> String {
    for name in ["codex.cmd", "codex.ps1", "codex.exe", "codex"] {
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

    for name in ["codex.cmd", "codex.ps1", "codex.exe", "codex"] {
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
    "codex.cmd".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credit_history_is_scoped_to_source_and_account() {
        let first = Path::new("C:\\account-tests\\work\\auth.json");
        let second = Path::new("C:\\account-tests\\personal\\auth.json");
        assert_ne!(
            credit_state_file_name(first, Some("work")),
            credit_state_file_name(second, Some("work"))
        );
        assert_ne!(
            credit_state_file_name(first, Some("work")),
            credit_state_file_name(first, Some("personal"))
        );
        assert_ne!(
            credit_state_file_name(first, None),
            credit_state_file_name(second, None)
        );
    }

    fn usage_from_json(json: &str) -> UsageData {
        let response: CodexUsageResponse =
            serde_json::from_str(json).expect("the fixture should deserialize");
        codex_usage_from_response(response, None).expect("the fixture should carry rate limits")
    }

    fn credits(balance: &str, has_credits: bool) -> CodexCredits {
        CodexCredits {
            has_credits,
            unlimited: false,
            overage_limit_reached: false,
            balance: Some(balance.into()),
        }
    }

    #[test]
    fn the_first_balance_seeds_the_baseline_and_reads_untouched() {
        let (state, section) = codex_credits(None, &credits("1026.112935", true), true, None);

        assert_eq!(state.baseline, 1026.112935);
        // 기준점이 설정된 이후 아직 사용된 내역이 없으므로,
        // 이후 폴링에서 잔액 감소를 감지할 때까지 바는 일반 윈도우 상태를 유지합니다.
        assert!(section.is_none());

        // 이후 폴링(1달러당 25크레딧 환율 적용).
        let previous = state;
        let (_, section) = codex_credits(Some(previous), &credits("1016.190898", true), true, None);
        let section = section.expect("a falling balance should expose the gauge");
        assert!(
            (section.remaining - 40.64763592).abs() < 1e-6,
            "{section:?}"
        );
    }

    #[test]
    fn spending_against_a_baseline_fills_the_gauge() {
        let previous = CodexCreditsState {
            account_id: None,
            balance: 2500.0,
            baseline: 2500.0,
        };
        let (state, section) = codex_credits(Some(previous), &credits("1250.0", true), true, None);

        assert_eq!(state.baseline, 2500.0);
        let section = section.expect("gauge");
        assert_eq!(section.percentage, 50.0);
        assert_eq!(section.remaining, 50.0);
        assert_eq!(section.total, 100.0);
    }

    #[test]
    fn a_rise_in_the_balance_is_a_reload_and_rebaselines() {
        let previous = CodexCreditsState {
            account_id: None,
            balance: 100.0,
            baseline: 2500.0,
        };
        let (state, section) = codex_credits(Some(previous), &credits("2600.0", true), true, None);

        assert_eq!(state.baseline, 2600.0);
        // 새로 충전된 직후에는 사용액이 0이므로,
        // 다시 크레딧 차감이 시작될 때까지 게이지는 대기 상태를 유지합니다.
        assert!(section.is_none());
    }

    #[test]
    fn changing_accounts_reseeds_the_credit_baseline() {
        let previous = CodexCreditsState {
            account_id: Some("old-account".into()),
            balance: 100.0,
            baseline: 2500.0,
        };
        let (state, section) = codex_credits(
            Some(previous),
            &credits("50.0", true),
            true,
            Some("new-account"),
        );

        assert_eq!(state.account_id.as_deref(), Some("new-account"));
        assert_eq!(state.baseline, 50.0);
        assert!(
            section.is_none(),
            "a different account's lower balance is not prior spending"
        );
    }

    #[test]
    fn the_gauge_hides_while_an_allowance_remains() {
        let previous = CodexCreditsState {
            account_id: None,
            balance: 2000.0,
            baseline: 2500.0,
        };
        let (state, section) = codex_credits(Some(previous), &credits("1000.0", true), false, None);

        // 바가 숨겨져 있어도 추적은 계속되어 충전 시 기준점이 올바르게 이동합니다.
        assert_eq!(state.balance, 1000.0);
        assert_eq!(state.baseline, 2500.0);
        assert!(section.is_none());
    }

    #[test]
    fn accounts_without_credits_get_no_gauge() {
        let (_, section) = codex_credits(None, &credits("0", false), true, None);
        assert!(section.is_none());

        let unlimited = CodexCredits {
            unlimited: true,
            ..credits("1000.0", true)
        };
        let (_, section) = codex_credits(None, &unlimited, true, None);
        assert!(section.is_none());
    }

    #[test]
    fn a_reached_overage_limit_pins_the_gauge_full() {
        let previous = CodexCreditsState {
            account_id: None,
            balance: 500.0,
            baseline: 1000.0,
        };
        let reached = CodexCredits {
            overage_limit_reached: true,
            ..credits("500.0", true)
        };
        let (_, section) = codex_credits(Some(previous), &reached, true, None);

        assert_eq!(section.expect("gauge").percentage, 100.0);
    }

    #[test]
    fn a_lone_weekly_window_lands_in_the_weekly_bar() {
        // Codex는 5시간 윈도우가 꺼져 있을 때 이 형태로 전달합니다:
        // `primary_window`에 주간 한도가 들어옵니다.
        let data = usage_from_json(
            r#"{
                "rate_limit": {
                    "primary_window": {
                        "used_percent": 100,
                        "limit_window_seconds": 604800,
                        "reset_at": 1787198224
                    },
                    "secondary_window": null
                }
            }"#,
        );

        assert_eq!(data.weekly.percentage, 100.0);
        assert_eq!(data.session.percentage, 0.0);
        assert!(data.weekly.resets_at.is_some());
        assert!(data.session.resets_at.is_none());
        assert!(data.weekly.available);
        assert!(!data.session.available);
    }

    #[test]
    fn a_reported_zero_usage_window_is_available_without_a_usable_reset() {
        let data = usage_from_json(
            r#"{
            "rate_limit": {
                "primary_window": {"used_percent":0,"limit_window_seconds":18000,"reset_at":-1},
                "secondary_window": null
            }
        }"#,
        );
        assert!(data.session.available);
        assert_eq!(data.session.percentage, 0.0);
        assert!(data.session.resets_at.is_none());
        assert!(!data.weekly.available);
    }

    #[test]
    fn windows_are_assigned_by_length_regardless_of_slot_order() {
        let data = usage_from_json(
            r#"{
                "rate_limit": {
                    "primary_window": {
                        "used_percent": 80,
                        "limit_window_seconds": 604800,
                        "reset_at": 1787198224
                    },
                    "secondary_window": {
                        "used_percent": 20,
                        "limit_window_seconds": 18000,
                        "reset_at": 1787100000
                    }
                }
            }"#,
        );

        assert_eq!(data.weekly.percentage, 80.0);
        assert_eq!(data.session.percentage, 20.0);
    }

    #[test]
    fn an_unlabelled_window_stays_in_the_session_bar() {
        let data = usage_from_json(
            r#"{
                "rate_limit": {
                    "primary_window": {"used_percent": 42, "reset_at": 1787100000},
                    "secondary_window": null
                }
            }"#,
        );

        assert_eq!(data.session.percentage, 42.0);
        assert_eq!(data.weekly.percentage, 0.0);
    }

    #[test]
    fn two_unlabelled_windows_keep_the_legacy_slot_mapping() {
        let data = usage_from_json(
            r#"{
                "rate_limit": {
                    "primary_window": {"used_percent": 20, "reset_at": 1787100000},
                    "secondary_window": {"used_percent": 80, "reset_at": 1787198224}
                }
            }"#,
        );

        assert_eq!(data.session.percentage, 20.0);
        assert_eq!(data.weekly.percentage, 80.0);
    }
}
