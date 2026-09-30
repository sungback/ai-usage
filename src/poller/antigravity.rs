//! Antigravity 사용량 폴러 — 처음 보시는 분을 위한 안내.
//!
//! - 순서: 키체인·자격 증명 저장소에서 토큰 읽기 → 필요하면 OAuth 갱신 →
//!   쿼터 API 호출 → Gemini·서드파티 구간 꺼내기.
//! - OAuth 클라이언트 값은 저장소에 올리지 않고 이 머신의 Antigravity 설치본에서
//!   읽습니다. 자세한 사유는 아래 `CONFIGURED_CLIENT_ID` 주석에 있습니다.

use std::collections::HashMap;
#[cfg(windows)]
use std::ffi::c_void;
#[cfg(any(windows, target_os = "macos"))]
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Mutex;

use serde::Deserialize;

use super::{build_agent, parse_iso8601, PollError};
use crate::models::{UsageData, UsageSection};

const ANTIGRAVITY_CREDENTIAL_TARGET: &str = "gemini:antigravity";
const ANTIGRAVITY_ENDPOINTS: &[&str] = &[
    "https://daily-cloudcode-pa.googleapis.com",
    "https://daily-cloudcode-pa.sandbox.googleapis.com",
    "https://cloudcode-pa.googleapis.com",
];

#[derive(Deserialize)]
struct AntigravityAuthFile {
    token: AntigravityTokenData,
}

#[derive(Deserialize, Clone)]
struct AntigravityTokenData {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    #[cfg_attr(not(test), allow(dead_code))]
    expiry: Option<String>,
    #[serde(default)]
    id_token: Option<String>,
}

/// 이 프로젝트가 소유한 자격 증명이 아니라 Antigravity 앱 자체의 Google "설치된 애플리케이션" OAuth 클라이언트입니다.
/// Antigravity가 이미 저장해 둔 리프레시 토큰을 교환할 때만 사용되며, 저장된 액세스 토큰이
/// 만료되는 1시간마다 한 번씩만 호출됩니다.
///
/// 이 값들은 공개 저장소에 직접 커밋하지 않고 로컬 기기의 Antigravity 설치본에서 읽어옵니다.
/// 공개 저장소에 실제 `GOCSPX-` 시크릿을 포함해 배포하면 유출될 뿐만 아니라, Google이 클라이언트를 교체하는 순간
/// 모든 사용자의 연동이 깨지기 때문입니다. `option_env!`가 우선하므로 로컬 환경 변수 오버라이드도 가능합니다.
const CONFIGURED_CLIENT_ID: Option<&str> = option_env!("ANTIGRAVITY_CLIENT_ID");
const CONFIGURED_CLIENT_SECRET: Option<&str> = option_env!("ANTIGRAVITY_CLIENT_SECRET");

const CLIENT_ID_SUFFIX: &[u8] = b".apps.googleusercontent.com";
const CLIENT_SECRET_PREFIX: &[u8] = b"GOCSPX-";
/// 현재 Antigravity에 포함된 두 클라이언트 모두 28자 시크릿을 사용합니다. Go 바이너리는
/// 문자열 리터럴을 연속 연결하므로, `GOCSPX-` 뒤의 문자열이 다음 리터럴과 이어져서
/// `main.js`의 따옴표 형식처럼 구분자로 경계를 나눌 수 없습니다.
const CLIENT_SECRET_LENGTH: usize = 28;

fn find_bytes(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if from >= haystack.len() || needle.is_empty() || haystack.len() - from < needle.len() {
        return None;
    }
    haystack[from..]
        .windows(needle.len())
        .position(|window| window == needle)
        .map(|offset| from + offset)
}

/// Antigravity 페이로드에서 `client_id` 뒤에 오는 클라이언트 시크릿을 반환합니다.
///
/// 클라이언트 ID를 기준점(anchor)으로 삼아야 안전합니다. 설치된 파일에는 둘 이상의 클라이언트가
/// 포함되어 있으므로 파일의 첫 번째 `GOCSPX-`를 무작정 가져오면 잘못된 ID와 시크릿이 짝지어져
/// Google에서 `invalid_client` 에러로 거부합니다.
fn extract_client_secret(payload: &[u8], client_id: &str) -> Option<String> {
    let anchor = find_bytes(payload, client_id.as_bytes(), 0)?;
    let secret_start = find_bytes(payload, CLIENT_SECRET_PREFIX, anchor + client_id.len())?;
    let value_start = secret_start + CLIENT_SECRET_PREFIX.len();
    let available = payload.len().saturating_sub(value_start);
    if available < CLIENT_SECRET_LENGTH {
        return None;
    }
    let value = payload.get(value_start..value_start + CLIENT_SECRET_LENGTH)?;
    if !value
        .iter()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return None;
    }
    // `GOCSPX-` 접두사도 시크릿 값의 일부입니다. 이것이 없으면 Google이 교환을 거부합니다.
    Some(format!(
        "{}{}",
        String::from_utf8_lossy(CLIENT_SECRET_PREFIX),
        String::from_utf8_lossy(value)
    ))
}

/// Antigravity OAuth 클라이언트가 포함된 파일 목록 (우선순위 순).
///
/// IDE 번들을 가장 선호합니다. 이 폴러가 읽는 키체인 항목을 작성한 주체이므로
/// 폴링이 가능한 환경이라면 항상 설치되어 있으며, 번들된 자바스크립트는 ID와 시크릿을
/// 인접한 따옴표 리터럴로 유지합니다. `agy`는 CLI 도구이며 선택 사항입니다.
fn oauth_client_source_files() -> Vec<std::path::PathBuf> {
    #[cfg(target_os = "macos")]
    let mut candidates = vec![
        std::path::PathBuf::from("/Applications/Antigravity IDE.app/Contents/Resources/app/out/main.js"),
        std::path::PathBuf::from(
            "/Applications/Antigravity IDE.app/Contents/Resources/app/extensions/antigravity/bin/language_server_macos_arm",
        ),
        std::path::PathBuf::from("/Applications/Antigravity.app/Contents/Resources/app/out/main.js"),
    ];
    #[cfg(windows)]
    let mut candidates = {
        let local = std::env::var_os("LOCALAPPDATA").map(std::path::PathBuf::from);
        let mut paths = Vec::new();
        if let Some(local) = local {
            // VS Code 기반 Electron 레이아웃: Contents/Resources/가 아니라 resources/입니다.
            for root in ["Programs/Antigravity IDE", "Programs/Antigravity"] {
                paths.push(local.join(root).join("resources/app/out/main.js"));
            }
            paths.push(local.join("agy/bin/agy.exe"));
        }
        paths
    };
    #[cfg(not(any(windows, target_os = "macos")))]
    let mut candidates: Vec<std::path::PathBuf> = Vec::new();

    if let Some(home) = dirs::home_dir() {
        candidates.push(home.join(".local/bin/agy"));
    }
    if let Ok(path) = std::env::var("PATH") {
        for directory in std::env::split_paths(&path) {
            candidates.push(directory.join(if cfg!(windows) { "agy.exe" } else { "agy" }));
        }
    }
    candidates
}

fn find_client_secret_on_disk(client_id: &str) -> Option<String> {
    for path in oauth_client_source_files() {
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        if let Some(secret) = extract_client_secret(&bytes, client_id) {
            return Some(secret);
        }
    }
    None
}

fn oauth_client_secret(client_id: &str) -> Option<String> {
    if let Some(secret) = CONFIGURED_CLIENT_SECRET {
        return Some(secret.to_owned());
    }
    static CACHE: std::sync::OnceLock<Mutex<Option<String>>> = std::sync::OnceLock::new();
    let mut cached = CACHE.get_or_init(|| Mutex::new(None)).lock().ok()?;
    if cached.is_none() {
        *cached = find_client_secret_on_disk(client_id);
    }
    cached.clone()
}

fn jwt_claim(token: &str, claim: &str) -> Option<String> {
    use base64::Engine;
    let payload = token.split('.').nth(1)?;
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload.trim_end_matches('='))
        .ok()?;
    serde_json::from_slice::<serde_json::Value>(&decoded)
        .ok()?
        .get(claim)?
        .as_str()
        .map(str::to_owned)
}

/// OAuth 클라이언트 ID를 확인합니다: 명시적 빌드 타임 오버라이드 -> Antigravity가 저장한 토큰의 aud 클레임 -> 로컬 설치본에서 검색된 클라이언트 ID 순입니다.
fn oauth_client_id(id_token: Option<&str>) -> String {
    if let Some(configured) = CONFIGURED_CLIENT_ID {
        return configured.to_owned();
    }
    if let Some(audience) = id_token.and_then(|token| jwt_claim(token, "aud")) {
        return audience;
    }
    discover_client_id().unwrap_or_default()
}

fn discover_client_id() -> Option<String> {
    for path in oauth_client_source_files() {
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        if let Some(id) = extract_first_client_id(&bytes) {
            return Some(id);
        }
    }
    None
}

/// 빌드 타임 오버라이드도 없고 `aud` 클레임도 없을 때 사용하는 최후의 수단(best-effort)입니다.
/// 설치 파일에는 여러 클라이언트가 있어 둘 중 하나를 선택할 수 있으며, 추측이 틀리면
/// 조용히 무시되는 대신 명확하게 토큰 교환 실패 에러가 발생합니다.
fn extract_first_client_id(payload: &[u8]) -> Option<String> {
    let suffix_at = payload
        .windows(CLIENT_ID_SUFFIX.len())
        .position(|window| window == CLIENT_ID_SUFFIX)?;
    let head = &payload[..suffix_at];
    let begin = head
        .iter()
        .rposition(|byte| !byte.is_ascii_alphanumeric() && *byte != b'-')
        .map_or(0, |index| index + 1);
    let prefix = std::str::from_utf8(&head[begin..]).ok()?;
    let prefix = prefix.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '-');
    let digits = prefix.chars().take_while(char::is_ascii_digit).count();
    if digits == 0 || !prefix[digits..].starts_with('-') {
        return None;
    }
    Some(format!("{prefix}{}", String::from_utf8_lossy(CLIENT_ID_SUFFIX)))
}



#[derive(Deserialize)]
struct AntigravityLoadResponse {
    #[serde(rename = "cloudaicompanionProject")]
    project: Option<String>,
}

#[derive(Deserialize)]
struct AntigravityModelsResponse {
    models: HashMap<String, AntigravityModelInfo>,
}

#[derive(Deserialize)]
struct AntigravityModelInfo {
    #[serde(rename = "quotaInfo")]
    quota_info: Option<AntigravityQuotaInfo>,
}

#[derive(Deserialize)]
pub(super) struct AntigravityQuotaInfo {
    #[serde(rename = "remainingFraction")]
    remaining_fraction: Option<f64>,
    #[serde(rename = "resetTime")]
    reset_time: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct AntigravityQuotaSummaryResponse {
    groups: Option<Vec<AntigravityQuotaSummaryGroup>>,
}

#[derive(Clone, Deserialize)]
pub(super) struct AntigravityQuotaSummaryGroup {
    #[serde(rename = "displayName")]
    display_name: Option<String>,
    description: Option<String>,
    buckets: Option<Vec<AntigravityQuotaSummaryBucket>>,
}

#[derive(Clone, Deserialize)]
pub(super) struct AntigravityQuotaSummaryBucket {
    #[serde(rename = "bucketId")]
    bucket_id: Option<String>,
    #[serde(rename = "displayName")]
    display_name: Option<String>,
    window: Option<String>,
    #[serde(rename = "remainingFraction")]
    remaining_fraction: Option<f64>,
    #[serde(rename = "resetTime")]
    reset_time: Option<String>,
}

#[cfg(windows)]
#[repr(C)]
struct CredentialW {
    flags: u32,
    type_: u32,
    target_name: *mut u16,
    comment: *mut u16,
    last_written: u64,
    credential_blob_size: u32,
    credential_blob: *mut u8,
    persist: u32,
    attribute_count: u32,
    attributes: *mut c_void,
    target_alias: *mut u16,
    user_name: *mut u16,
}

#[cfg(windows)]
#[link(name = "Advapi32")]
extern "system" {
    fn CredReadW(
        target_name: *const u16,
        type_: u32,
        reserved_flags: u32,
        credential: *mut *mut CredentialW,
    ) -> i32;
    fn CredFree(buffer: *mut c_void);
}

fn refresh_antigravity_token(refresh_token: &str, id_token: Option<&str>) -> Result<String, PollError> {
    let agent = build_agent()?;
    let client_id = oauth_client_id(id_token);
    let Some(client_secret) = oauth_client_secret(&client_id) else {
        return Err(PollError::AuthRequired);
    };
    let payload = format!(
        "client_id={}&client_secret={}&grant_type=refresh_token&refresh_token={}",
        client_id, client_secret, refresh_token
    );

    let mut resp = match agent
        .post("https://oauth2.googleapis.com/token")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .send(payload.as_bytes())
    {
        Ok(resp) => resp,
        Err(_) => {
            return Err(PollError::AuthRequired);
        }
    };

    #[derive(Deserialize)]
    struct RefreshResponse {
        access_token: String,
    }

    let token_resp: RefreshResponse = match resp.body_mut().read_json() {
        Ok(res) => res,
        Err(_) => {
            return Err(PollError::AuthRequired);
        }
    };

    if token_resp.access_token.is_empty() {
        return Err(PollError::AuthRequired);
    }

    Ok(token_resp.access_token)
}

struct CachedAntigravityToken {
    refresh_token: String,
    access_token: String,
}

static CACHED_ACCESS_TOKEN: Mutex<Option<CachedAntigravityToken>> = Mutex::new(None);

pub(super) fn poll_antigravity() -> Result<UsageData, PollError> {
    let creds = match read_antigravity_credentials() {
        Some(creds) => creds,
        None => {
            return Err(PollError::NoCredentials);
        }
    };

    // 1. 동일한 리프레시 토큰에 대한 캐시된 액세스 토큰이 있다면 먼저 시도하여,
    // 키체인/자격 증명 관리자의 토큰이 만료되었을 때 발생하는 401 왕복 지연을 방지합니다.
    let cached_token = {
        let lock = CACHED_ACCESS_TOKEN
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        lock.as_ref().and_then(|c| {
            if let Some(ref rf) = creds.refresh_token {
                if &c.refresh_token == rf {
                    Some(c.access_token.clone())
                } else {
                    None
                }
            } else {
                None
            }
        })
    };

    if let Some(token) = cached_token {
        match fetch_antigravity_usage(&token) {
            Ok(data) => return Ok(data),
            Err(PollError::AuthRequired) => {
                let mut lock = CACHED_ACCESS_TOKEN
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                *lock = None;
            }
            Err(other) => return Err(other),
        }
    }

    // 2. 자격 증명 자체의 액세스 토큰을 시도합니다.
    match fetch_antigravity_usage(&creds.access_token) {
        Ok(data) => Ok(data),
        Err(PollError::AuthRequired) => {
            if let Some(ref rf) = creds.refresh_token {
                let new_token = refresh_antigravity_token(rf, creds.id_token.as_deref())?;
                // 이후 폴링에서 바로 사용할 수 있도록 새로운 토큰을 메모리에 캐시합니다.
                {
                    let mut lock = CACHED_ACCESS_TOKEN
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    *lock = Some(CachedAntigravityToken {
                        refresh_token: rf.clone(),
                        access_token: new_token.clone(),
                    });
                }
                fetch_antigravity_usage(&new_token)
            } else {
                Err(PollError::AuthRequired)
            }
        }
        Err(other) => Err(other),
    }
}

pub(super) fn antigravity_credential_watch_signature() -> String {
    #[cfg(windows)]
    {
        let Some(content) = read_windows_generic_credential(ANTIGRAVITY_CREDENTIAL_TARGET) else {
            return format!("{ANTIGRAVITY_CREDENTIAL_TARGET}|missing");
        };

        let mut hasher = DefaultHasher::new();
        content.hash(&mut hasher);
        format!(
            "{ANTIGRAVITY_CREDENTIAL_TARGET}|present|{}|{}",
            content.len(),
            hasher.finish()
        )
    }
    #[cfg(target_os = "macos")]
    {
        let Some(creds) = read_antigravity_credentials() else {
            return format!("{ANTIGRAVITY_CREDENTIAL_TARGET}|missing");
        };

        let mut hasher = DefaultHasher::new();
        creds.access_token.hash(&mut hasher);
        if let Some(ref rf) = creds.refresh_token {
            rf.hash(&mut hasher);
        }
        format!(
            "{ANTIGRAVITY_CREDENTIAL_TARGET}|present|{}",
            hasher.finish()
        )
    }
    #[cfg(all(not(windows), not(target_os = "macos")))]
    {
        format!("{ANTIGRAVITY_CREDENTIAL_TARGET}|missing")
    }
}

pub(super) fn fetch_antigravity_usage(token: &str) -> Result<UsageData, PollError> {
    let mut auth_error = false;
    let mut last_error = PollError::RequestFailed;

    for base_url in ANTIGRAVITY_ENDPOINTS {
        match fetch_antigravity_usage_from_endpoint(base_url, token) {
            Ok(data) => return Ok(data),
            Err(PollError::AuthRequired) => auth_error = true,
            Err(error) => last_error = error,
        }
    }

    if auth_error {
        Err(PollError::AuthRequired)
    } else {
        Err(last_error)
    }
}

pub(super) fn fetch_antigravity_usage_from_endpoint(
    base_url: &str,
    token: &str,
) -> Result<UsageData, PollError> {
    let project = fetch_antigravity_project(base_url, token)?;
    if let Some(project) = project.as_deref() {
        match fetch_antigravity_quota_summary(base_url, token, project) {
            Ok(data) => return Ok(data),
            Err(PollError::AuthRequired) => return Err(PollError::AuthRequired),
            Err(_) => {}
        }
    }

    let session = fetch_antigravity_model_quota(base_url, token, project.as_deref())?;
    let weekly = UsageSection::default();

    Ok(UsageData {
        session,
        weekly,
        weekly_label: None,
        monthly: None,
        credits: None,
        stale: false,
    })
}

pub(super) fn fetch_antigravity_project(
    base_url: &str,
    token: &str,
) -> Result<Option<String>, PollError> {
    let agent = build_agent()?;
    let body = serde_json::json!({
        "metadata": {
            "ideType": "ANTIGRAVITY"
        }
    });

    let mut resp = match agent
        .post(&format!("{base_url}/v1internal:loadCodeAssist"))
        .header("Authorization", &format!("Bearer {token}"))
        .header("Content-Type", "application/json")
        .header("User-Agent", "antigravity")
        .send_json(&body)
    {
        Ok(resp) => resp,
        Err(ureq::Error::StatusCode(code)) if code == 401 || code == 403 => {
            return Err(PollError::AuthRequired);
        }
        Err(_) => {
            return Err(PollError::RequestFailed);
        }
    };

    let response: AntigravityLoadResponse = match resp.body_mut().read_json() {
        Ok(response) => response,
        Err(_) => {
            return Err(PollError::RequestFailed);
        }
    };

    Ok(response.project.filter(|project| !project.is_empty()))
}

pub(super) fn fetch_antigravity_model_quota(
    base_url: &str,
    token: &str,
    project: Option<&str>,
) -> Result<UsageSection, PollError> {
    let agent = build_agent()?;
    let body = match project {
        Some(project) => serde_json::json!({ "project": project }),
        None => serde_json::json!({}),
    };

    let mut resp = match agent
        .post(&format!("{base_url}/v1internal:fetchAvailableModels"))
        .header("Authorization", &format!("Bearer {token}"))
        .header("Content-Type", "application/json")
        .header("User-Agent", "antigravity")
        .send_json(&body)
    {
        Ok(resp) => resp,
        Err(ureq::Error::StatusCode(code)) if code == 401 || code == 403 => {
            return Err(PollError::AuthRequired);
        }
        Err(_) => {
            return Err(PollError::RequestFailed);
        }
    };

    let response: AntigravityModelsResponse = match resp.body_mut().read_json() {
        Ok(response) => response,
        Err(_) => {
            return Err(PollError::RequestFailed);
        }
    };

    best_antigravity_section(response.models.into_iter().filter_map(|(model, info)| {
        let quota = info.quota_info?;
        if !is_antigravity_display_model(&model) {
            return None;
        }
        antigravity_section_from_quota(quota)
    }))
    .ok_or(PollError::RequestFailed)
}

pub(super) fn fetch_antigravity_quota_summary(
    base_url: &str,
    token: &str,
    project: &str,
) -> Result<UsageData, PollError> {
    let agent = build_agent()?;
    let body = serde_json::json!({ "project": project });

    let mut resp = match agent
        .post(&format!("{base_url}/v1internal:retrieveUserQuotaSummary"))
        .header("Authorization", &format!("Bearer {token}"))
        .header("Content-Type", "application/json")
        .header("User-Agent", "antigravity")
        .send_json(&body)
    {
        Ok(resp) => resp,
        Err(ureq::Error::StatusCode(code)) if code == 401 || code == 403 => {
            return Err(PollError::AuthRequired);
        }
        Err(_) => {
            return Err(PollError::RequestFailed);
        }
    };

    let response: AntigravityQuotaSummaryResponse = match resp.body_mut().read_json() {
        Ok(response) => response,
        Err(_) => {
            return Err(PollError::RequestFailed);
        }
    };

    antigravity_usage_from_summary(response).ok_or(PollError::RequestFailed)
}

pub(super) fn antigravity_section_from_quota(quota: AntigravityQuotaInfo) -> Option<UsageSection> {
    let remaining = quota.remaining_fraction?.clamp(0.0, 1.0);
    Some(UsageSection {
        available: true,
        percentage: (1.0 - remaining) * 100.0,
        resets_at: parse_iso8601(quota.reset_time.as_deref()),
    })
}

pub(super) fn antigravity_section_from_summary_bucket(
    bucket: &AntigravityQuotaSummaryBucket,
) -> Option<UsageSection> {
    let remaining = bucket.remaining_fraction?.clamp(0.0, 1.0);
    Some(UsageSection {
        available: true,
        percentage: (1.0 - remaining) * 100.0,
        resets_at: parse_iso8601(bucket.reset_time.as_deref()),
    })
}

pub(super) fn antigravity_usage_from_summary(
    response: AntigravityQuotaSummaryResponse,
) -> Option<UsageData> {
    let groups = response.groups.unwrap_or_default();
    let gemini_group = groups.iter().find(|g| is_antigravity_gemini_summary_group(g));
    let third_party_group = groups.iter().find(|g| {
        let name = g.display_name.as_deref().unwrap_or("").to_ascii_lowercase();
        name.contains("claude") || name.contains("gpt") || name.contains("3p")
    });

    if let Some(gemini) = gemini_group {
        let mut gemini_usage = antigravity_usage_from_summary_group(gemini.clone())?;
        if let Some(third_party) = third_party_group {
            if let Some(tp_usage) = antigravity_usage_from_summary_group(third_party.clone()) {
                let tp_section = if tp_usage.weekly.available {
                    tp_usage.weekly
                } else {
                    tp_usage.session
                };
                if !gemini_usage.session.available && gemini_usage.weekly.available {
                    gemini_usage.session = gemini_usage.weekly;
                    gemini_usage.weekly = tp_section;
                } else if !gemini_usage.weekly.available {
                    gemini_usage.weekly = tp_section;
                }
            }
        }
        return Some(gemini_usage);
    }

    for group in groups {
        if let Some(usage) = antigravity_usage_from_summary_group(group) {
            return Some(usage);
        }
    }
    None
}

pub(super) fn antigravity_usage_from_summary_group(
    group: AntigravityQuotaSummaryGroup,
) -> Option<UsageData> {
    let mut data = UsageData::default();
    let mut has_quota = false;

    for bucket in group.buckets.unwrap_or_default() {
        let Some(section) = antigravity_section_from_summary_bucket(&bucket) else {
            continue;
        };

        match bucket.window.as_deref() {
            Some(window) if window.eq_ignore_ascii_case("5h") => {
                data.session = section;
                has_quota = true;
            }
            Some(window) if window.eq_ignore_ascii_case("weekly") => {
                data.weekly = section;
                has_quota = true;
            }
            _ => {
                if !data.session.available {
                    data.session = section;
                    has_quota = true;
                } else if !data.weekly.available {
                    data.weekly = section;
                    has_quota = true;
                }
            }
        }
    }

    has_quota.then_some(data)
}

pub(super) fn is_antigravity_gemini_summary_group(group: &AntigravityQuotaSummaryGroup) -> bool {
    group
        .display_name
        .as_deref()
        .is_some_and(|name| name.to_ascii_lowercase().contains("gemini"))
        || group
            .description
            .as_deref()
            .is_some_and(|description| description.to_ascii_lowercase().contains("gemini"))
        || group.buckets.as_ref().is_some_and(|buckets| {
            buckets.iter().any(|bucket| {
                bucket
                    .bucket_id
                    .as_deref()
                    .is_some_and(|id| id.to_ascii_lowercase().starts_with("gemini-"))
                    || bucket
                        .display_name
                        .as_deref()
                        .is_some_and(|name| name.to_ascii_lowercase().contains("gemini"))
            })
        })
}

pub(super) fn best_antigravity_section<I>(sections: I) -> Option<UsageSection>
where
    I: IntoIterator<Item = UsageSection>,
{
    sections.into_iter().max_by(|a, b| {
        a.percentage
            .partial_cmp(&b.percentage)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.resets_at.cmp(&b.resets_at))
    })
}

pub(super) fn is_antigravity_display_model(model: &str) -> bool {
    model.starts_with("gemini")
        || model.starts_with("claude")
        || model.starts_with("gpt")
        || model.starts_with("image")
        || model.starts_with("imagen")
}

fn parse_antigravity_keychain_payload(text: &str) -> Option<AntigravityTokenData> {
    let json_text = if let Some(stripped) = text.strip_prefix("go-keyring-base64:") {
        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD.decode(stripped.trim()).ok()?;
        String::from_utf8(bytes).ok()?
    } else {
        text.to_string()
    };

    if let Ok(auth) = serde_json::from_str::<AntigravityAuthFile>(&json_text) {
        if !auth.token.access_token.is_empty() {
            return Some(auth.token);
        }
    }
    if let Ok(token) = serde_json::from_str::<AntigravityTokenData>(&json_text) {
        if !token.access_token.is_empty() {
            return Some(token);
        }
    }
    None
}

#[cfg(windows)]
fn read_antigravity_credentials() -> Option<AntigravityTokenData> {
    let content = read_windows_generic_credential(ANTIGRAVITY_CREDENTIAL_TARGET)?;
    parse_antigravity_keychain_payload(&content)
}

#[cfg(not(windows))]
fn read_antigravity_credentials() -> Option<AntigravityTokenData> {
    #[cfg(target_os = "macos")]
    {
        // 1. 최신 Antigravity IDE 키체인 타깃 시도 (-s "gemini" -a "antigravity")
        if let Ok(output) = std::process::Command::new("security")
            .args(["find-generic-password", "-s", "gemini", "-a", "antigravity", "-w"])
            .output()
        {
            if output.status.success() {
                let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if let Some(token) = parse_antigravity_keychain_payload(&text) {
                    return Some(token);
                }
            }
        }

        // 2. 레거시 타깃으로 폴백 (-s "gemini:antigravity")
        if let Ok(output) = std::process::Command::new("security")
            .args(["find-generic-password", "-s", ANTIGRAVITY_CREDENTIAL_TARGET, "-w"])
            .output()
        {
            if output.status.success() {
                let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if let Some(token) = parse_antigravity_keychain_payload(&text) {
                    return Some(token);
                }
            }
        }
    }
    None
}

#[cfg(windows)]
fn read_windows_generic_credential(target: &str) -> Option<String> {
    const CRED_TYPE_GENERIC: u32 = 1;

    let target_wide: Vec<u16> = target.encode_utf16().chain(std::iter::once(0)).collect();
    let mut credential: *mut CredentialW = std::ptr::null_mut();
    let ok = unsafe { CredReadW(target_wide.as_ptr(), CRED_TYPE_GENERIC, 0, &mut credential) };
    if ok == 0 || credential.is_null() {
        return None;
    }

    unsafe {
        let credentials = &*credential;
        if credentials.credential_blob_size == 0 || credentials.credential_blob.is_null() {
            CredFree(credential as *mut c_void);
            return None;
        }
        let bytes = std::slice::from_raw_parts(
            credentials.credential_blob,
            credentials.credential_blob_size as usize,
        );
        let text = String::from_utf8(bytes.to_vec()).ok();
        CredFree(credential as *mut c_void);
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 기본적으로 무시됨(ignored): 실제 Antigravity API를 호출하며 로그인된 기기에서만
    /// 통과합니다. Antigravity에 로그인된 상태에서 `cargo test -- --ignored`로 실행하세요.
    #[test]
    #[ignore = "requires a signed-in Antigravity account and network access"]
    fn test_poll_antigravity_live() {
        let res = poll_antigravity();
        println!("poll_antigravity result: {:?}", res);
        assert!(res.is_ok(), "Antigravity poll should succeed: {:?}", res);
        let usage = res.unwrap();
        println!("Session available: {}, pct: {}", usage.session.available, usage.session.percentage);
        println!("Weekly available: {}, pct: {}", usage.weekly.available, usage.weekly.percentage);
        assert!(usage.session.available || usage.weekly.available);
    }

    #[test]
    fn test_antigravity_mock_summary_gemini_and_3p() {
        let json_str = r#"{
            "groups": [
                {
                    "displayName": "Gemini Models",
                    "buckets": [
                        {
                            "bucketId": "gemini-5h",
                            "window": "5h",
                            "remainingFraction": 0.85,
                            "resetTime": "2026-06-13T22:08:54Z"
                        }
                    ]
                },
                {
                    "displayName": "Claude and GPT models",
                    "buckets": [
                        {
                            "bucketId": "3p-weekly",
                            "window": "weekly",
                            "remainingFraction": 0.40,
                            "resetTime": "2026-06-20T18:32:02Z"
                        }
                    ]
                }
            ]
        }"#;

        let resp: AntigravityQuotaSummaryResponse = serde_json::from_str(json_str).unwrap();
        let usage = antigravity_usage_from_summary(resp).expect("usage should parse successfully");

        assert!(usage.session.available);
        assert!((usage.session.percentage - 15.0).abs() < 0.001);
        assert!(usage.session.resets_at.is_some());

        assert!(usage.weekly.available);
        assert!((usage.weekly.percentage - 60.0).abs() < 0.001);
        assert!(usage.weekly.resets_at.is_some());
    }

    #[test]
    fn test_antigravity_token_deserialization() {
        let json = r#"{"token":{"access_token":"ya29.sample","refresh_token":"1//refresh_token_test","expiry":"2026-09-20T12:00:00Z"}}"#;
        let auth: AntigravityAuthFile = serde_json::from_str(json).unwrap();
        assert_eq!(auth.token.access_token, "ya29.sample");
        assert_eq!(auth.token.refresh_token.as_deref(), Some("1//refresh_token_test"));
        assert_eq!(auth.token.expiry.as_deref(), Some("2026-09-20T12:00:00Z"));
    }

    #[test]
    fn test_antigravity_token_cache_logic() {
        {
            let mut lock = CACHED_ACCESS_TOKEN.lock().unwrap();
            *lock = Some(CachedAntigravityToken {
                refresh_token: "refresh_A".into(),
                access_token: "access_A".into(),
            });
        }

        // 동일한 리프레시 토큰은 캐시된 액세스 토큰을 찾아야 함
        {
            let lock = CACHED_ACCESS_TOKEN.lock().unwrap();
            let found = lock.as_ref().and_then(|c| {
                if c.refresh_token == "refresh_A" {
                    Some(c.access_token.clone())
                } else {
                    None
                }
            });
            assert_eq!(found.as_deref(), Some("access_A"));
        }

        // 다른 리프레시 토큰은 오래된 캐시를 무시해야 함
        {
            let lock = CACHED_ACCESS_TOKEN.lock().unwrap();
            let found = lock.as_ref().and_then(|c| {
                if c.refresh_token == "refresh_B" {
                    Some(c.access_token.clone())
                } else {
                    None
                }
            });
            assert_eq!(found, None);
        }

        // 정리(Clean up)
        {
            let mut lock = CACHED_ACCESS_TOKEN.lock().unwrap();
            *lock = None;
        }
    }

    fn jwt_with_claims(header: &str, payload: serde_json::Value) -> String {
        use base64::Engine;
        let encode = |value: &str| {
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(value.as_bytes())
        };
        format!(
            "{}.{}.sig",
            encode(header),
            encode(&payload.to_string())
        )
    }

    #[test]
    fn jwt_claim_reads_the_audience_from_an_id_token() {
        let audience = client_id("123");
        let token = jwt_with_claims(
            r#"{"alg":"RS256"}"#,
            serde_json::json!({
                "aud": audience,
                "sub": "11711031079867362152",
                "email": "someone@example.com",
            }),
        );
        assert_eq!(jwt_claim(&token, "aud").as_deref(), Some(audience.as_str()));
        assert_eq!(jwt_claim(&token, "email").as_deref(), Some("someone@example.com"));
    }

    #[test]
    fn jwt_claim_rejects_malformed_tokens_instead_of_panicking() {
        assert_eq!(jwt_claim("", "aud"), None);
        assert_eq!(jwt_claim("not-a-jwt", "aud"), None);
        assert_eq!(jwt_claim("only.two", "aud"), None);
        assert_eq!(jwt_claim("a.!!!not-base64!!!.c", "aud"), None);
        let token = jwt_with_claims("{}", serde_json::json!({"other": "value"}));
        assert_eq!(jwt_claim(&token, "aud"), None);
    }

    #[test]
    fn the_client_id_comes_from_the_token_when_no_build_override_is_set() {
        let token = jwt_with_claims(
            "{}",
            serde_json::json!({"aud": "from-token.apps.googleusercontent.com"}),
        );
        if CONFIGURED_CLIENT_ID.is_some() {
            assert_eq!(oauth_client_id(Some(&token)), CONFIGURED_CLIENT_ID.unwrap());
        } else {
            assert_eq!(
                oauth_client_id(Some(&token)),
                "from-token.apps.googleusercontent.com"
            );
        }
    }

    #[test]
    fn a_missing_audience_falls_back_to_a_discovered_client_id_or_nothing() {
        if CONFIGURED_CLIENT_ID.is_some() {
            return;
        }
        let discovered = oauth_client_id(None);
        let from_garbage = oauth_client_id(Some("garbage"));
        assert_eq!(discovered, from_garbage);
        assert!(discovered.is_empty() || discovered.ends_with(".apps.googleusercontent.com"));
    }

    #[test]
    fn a_build_time_secret_override_short_circuits_disk_discovery() {
        match CONFIGURED_CLIENT_SECRET {
            Some(secret) => assert_eq!(
                oauth_client_secret(&client_id("1234567890-x")),
                Some(secret.to_owned())
            ),
            None => assert!(oauth_client_secret(&client_id("1234567890-x"))
                .is_none_or(|secret| !secret.is_empty())),
        }
    }

    // 이전에 하드코딩된 값을 걸러냈던 시크릿 스캐너에 걸리지 않도록,
    // 의도적으로 실제 클라이언트 ID와 다른 형태로 만든 픽스처입니다.
    #[test]
    #[ignore = "requires a local Antigravity installation"]
    fn the_client_is_discovered_from_the_local_antigravity_install() {
        let Some(id) = discover_client_id() else {
            panic!("no Antigravity client id found on this machine");
        };
        assert!(id.ends_with(".apps.googleusercontent.com"), "{id}");
        let secret = oauth_client_secret(&id).expect("client id was found but no secret");
        assert!(secret.starts_with("GOCSPX-"), "{secret}");
        assert_eq!(secret.len(), CLIENT_SECRET_PREFIX.len() + CLIENT_SECRET_LENGTH);
    }

    /// GitHub 푸시 보호(push protection)가 Google OAuth 클라이언트 ID 패턴을 텍스트로 감지하므로,
    /// 접미사 옆에 전체 문자열을 적지 않고 여기서 가상 ID를 조립합니다.
    /// 실제 자격 증명이 아니며, 파서 관점에서 자격 증명처럼 보이기만 하면 됩니다.
    fn client_id(tag: &str) -> String {
        format!("{tag}{}", String::from_utf8_lossy(CLIENT_ID_SUFFIX))
    }

    fn sample_id() -> String {
        client_id("1-abcdefghijklmnopqrstuvwxyz0123456789")
    }

    fn other_id() -> String {
        client_id("1-zyxwvutsrqponmlkjihgfedcba9876543210")
    }

    const SAMPLE_SECRET: &str = "GOCSPX-AAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    const OTHER_SECRET: &str = "GOCSPX-BBBBBBBBBBBBBBBBBBBBBBBBBBBB";

    fn sample_payload(id: &str, secret: &str) -> Vec<u8> {
        format!("z9e=\"{id}\",$9e=\"{secret}\",Fze=\"x\"").into_bytes()
    }

    #[test]
    fn the_secret_is_read_from_the_literal_that_follows_its_own_client_id() {
        let id = sample_id();
        let payload = sample_payload(&id, SAMPLE_SECRET);
        assert_eq!(
            extract_client_secret(&payload, &id).as_deref(),
            Some(SAMPLE_SECRET)
        );
    }

    #[test]
    fn a_second_client_does_not_borrow_the_first_clients_secret() {
        let (first, second) = (sample_id(), other_id());
        let mut payload = sample_payload(&first, SAMPLE_SECRET);
        payload.push(b',');
        payload.extend_from_slice(&sample_payload(&second, OTHER_SECRET));

        assert_eq!(
            extract_client_secret(&payload, &first).as_deref(),
            Some(SAMPLE_SECRET)
        );
        assert_eq!(
            extract_client_secret(&payload, &second).as_deref(),
            Some(OTHER_SECRET)
        );
    }

    #[test]
    fn a_go_literal_run_past_the_secret_is_truncated_to_the_known_length() {
        // Go는 문자열 리터럴을 연결하므로 실제 바이트는 GOCSPX-<secret>decoding 형태가 됩니다.
        let id = sample_id();
        let mut payload = id.as_bytes().to_vec();
        payload.extend_from_slice(SAMPLE_SECRET.as_bytes());
        payload.extend_from_slice(b"decoding");

        assert_eq!(
            extract_client_secret(&payload, &id).as_deref(),
            Some(SAMPLE_SECRET)
        );
    }

    #[test]
    fn an_unknown_client_id_yields_no_secret() {
        let payload = sample_payload(&sample_id(), SAMPLE_SECRET);
        assert_eq!(extract_client_secret(&payload, &client_id("1-nope")), None);
    }

    #[test]
    fn a_truncated_secret_is_rejected_rather_than_guessed() {
        let id = sample_id();
        let payload = format!("{id}GOCSPX-tooshort");
        assert_eq!(extract_client_secret(payload.as_bytes(), &id), None);
    }

    #[test]
    fn a_client_id_is_recovered_from_the_bytes_surrounding_its_suffix() {
        let id = sample_id();
        let payload = format!("noise {id}$9e=\"x\"");
        assert_eq!(
            extract_first_client_id(payload.as_bytes()).as_deref(),
            Some(id.as_str())
        );
        assert_eq!(extract_first_client_id(b"nothing here"), None);
    }
}
