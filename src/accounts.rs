//! 계정(자격 증명 위치) 관리 — 처음 보시는 분을 위한 안내.
//!
//! - 이 파일은 토큰(비밀)을 절대 들고 있지 않습니다. 들고 있는 것은
//!   "어느 폴더·어느 파일에서 토큰을 찾을지"라는 위치와 별명뿐입니다.
//! - `fingerprint`는 비밀 내용을 지문(해시)으로 바꿔 "바뀌었는지"만 봅니다.
//! - 새 공급자의 계정을 지원하려면 맨 아래 `credential_file_name`에
//!   파일 이름 한 줄만 추가하면 됩니다.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::providers::ProviderId;

/// 비밀 문자열을 유추할 수 없는 짧은 지문으로 바꿉니다 (FNV-1a 64비트).
/// 토큰 자체를 로그·캐시에 남기지 않기 위한 안전장치입니다.
pub fn fingerprint(value: &str) -> String {
    let hash = value.bytes().fold(0xcbf29ce484222325u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
    });
    format!("{hash:016x}")
}

/// 파일의 "바뀜 지문"을 만듭니다. 경로+크기+수정한 시각을 섞어
/// "토큰이 바뀌었을 것 같은데?"를 판단하는 재료로 씁니다.
pub fn file_signature(path: &Path) -> String {
    let metadata = std::fs::metadata(path).ok();
    fingerprint(&format!(
        "{}|{:?}|{:?}",
        source_key(path),
        metadata.as_ref().map(|m| m.len()),
        metadata.and_then(|m| m.modified().ok())
    ))
}

/// 경로의 정규 이름입니다. 풀 수 있으면 절대경로, 아니면 소문자로 둡니다.
/// 같은 파일을 가리키는 다른 표기를 하나로 모으는 용도입니다.
pub fn source_key(path: &Path) -> String {
    match std::fs::canonicalize(path) {
        Ok(path) => path.to_string_lossy().into_owned(),
        Err(_) => path.to_string_lossy().to_lowercase(),
    }
}

/// 공급자별 자격 증명 파일 이름입니다. 계정을 지원하는 공급자가 늘면 여기만 늘립니다.
/// [필수 주석: 파일명 계약 - 폴러가 이 이름으로 디스크에서 토큰을 찾습니다]
fn credential_file_name(provider: ProviderId) -> Option<&'static str> {
    match provider {
        ProviderId::Claude => Some(".credentials.json"),
        ProviderId::Codex => Some("auth.json"),
        _ => None,
    }
}

/// 설정 폴더 이름입니다. 환경변수가 없을 때 집 폴더 아래에서 찾는 이름입니다.
fn config_dir_name(provider: ProviderId) -> Option<&'static str> {
    match provider {
        ProviderId::Claude => Some(".claude"),
        ProviderId::Codex => Some(".codex"),
        _ => None,
    }
}

/// 계정 ID로 쓸 수 있는 글자인지 봅니다. 비어 있거나 영숫자·밑줄이 아니면 탈락입니다.
fn is_valid_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// 로그인된 도구가 토큰을 두는 기본 자리입니다.
/// 환경변수(예: `CLAUDE_CONFIG_DIR`)가 있으면 그곳, 없으면 집 폴더 아래를 씁니다.
pub fn default_credential_path(provider: ProviderId) -> Option<PathBuf> {
    let config = config_dir_name(provider)?;
    let credential = credential_file_name(provider)?;
    let directory = environment_directory(provider)
        .or_else(|| dirs::home_dir().map(|home| home.join(config)))?;
    Some(directory.join(credential))
}

/// 계정 한 장의 명함입니다. 토큰 대신 "어디에 있는지"만 적혀 있습니다.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AccountProfile {
    /// 고유 별명입니다. 한 번 쓰인 것은 버려도 재사용하지 않습니다.
    pub id: String,
    /// 화면에 보이는 이름입니다. 비어 있으면 `id`를 씁니다.
    pub name: String,
    /// 설정 폴더를 직접 지정했을 때의 경로입니다.
    pub config_dir: String,
    /// 자격 증명 파일을 직접 지정했을 때의 경로입니다.
    pub credentials_path: String,
    pub enabled: bool,
}

impl Default for AccountProfile {
    fn default() -> Self {
        Self {
            id: "default".into(),
            name: "Default".into(),
            config_dir: String::new(),
            credentials_path: String::new(),
            enabled: true,
        }
    }
}

/// 한 공급자의 계정첩입니다.
/// - `profiles`: 등록된 명함들.
/// - `selected`: 지금 쓰고 있는 명함의 `id`.
/// - `used_ids`: 은퇴한 ID의 묘지. 테마가 옛 계정을 가리키다 새 계정을 가리키는
///   사고를 막으려 절대 재사용하지 않습니다.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProviderAccounts {
    pub profiles: Vec<AccountProfile>,
    pub selected: String,
    /// Retain retired IDs so existing theme bindings never target a new account.
    pub used_ids: std::collections::BTreeSet<String>,
}

impl Default for ProviderAccounts {
    fn default() -> Self {
        Self {
            profiles: vec![AccountProfile::default()],
            selected: "default".into(),
            used_ids: ["default".into()].into(),
        }
    }
}

impl ProviderAccounts {
    /// 지금 쓸 명함을 고릅니다. 고른 게 꺼져 있으면 켜진 첫 명함으로 넘어갑니다.
    pub fn selected(&self) -> Option<&AccountProfile> {
        self.profiles
            .iter()
            .find(|profile| profile.enabled && profile.id == self.selected)
            .or_else(|| self.profiles.iter().find(|profile| profile.enabled))
    }

    #[cfg(test)]
    pub fn add(&mut self) {
        self.normalize();
        let (id, number) = self.allocate_id();
        self.profiles.push(AccountProfile {
            id,
            name: format!("Account {number}"),
            enabled: false,
            ..Default::default()
        });
    }

    fn allocate_id(&mut self) -> (String, usize) {
        let mut number = 1;
        while self.used_ids.contains(&format!("account_{number}")) {
            number += 1;
        }
        let id = format!("account_{number}");
        self.used_ids.insert(id.clone());
        (id, number)
    }

    pub fn normalize(&mut self) {
        let selected_index = self
            .profiles
            .iter()
            .position(|profile| profile.id == self.selected)
            .or_else(|| {
                self.profiles
                    .iter()
                    .position(|profile| profile.id.eq_ignore_ascii_case(&self.selected))
            });
        self.used_ids = std::mem::take(&mut self.used_ids)
            .into_iter()
            .map(|id| id.to_ascii_lowercase())
            .collect();
        // Reserve every existing ID before allocating replacements. An earlier
        // invalid profile must not steal a later valid profile's binding.
        self.used_ids.extend(
            self.profiles
                .iter()
                .map(|profile| profile.id.to_ascii_lowercase()),
        );
        let mut seen = std::collections::HashSet::new();
        for index in 0..self.profiles.len() {
            let profile = &self.profiles[index];
            if !is_valid_id(&profile.id) || !seen.insert(profile.id.to_ascii_lowercase()) {
                let (id, _) = self.allocate_id();
                seen.insert(id.to_ascii_lowercase());
                self.profiles[index].id = id;
            }
            let profile = &mut self.profiles[index];
            if profile.name.trim().is_empty() {
                profile.name = profile.id.clone();
            }
        }
        if let Some(index) = selected_index {
            self.selected = self.profiles[index].id.clone();
        }
        self.selected = self.selected().map(|p| p.id.clone()).unwrap_or_default();
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AccountSettings {
    pub claude: ProviderAccounts,
    pub codex: ProviderAccounts,
}

impl AccountSettings {
    pub fn get(&self, provider: ProviderId) -> Option<&ProviderAccounts> {
        match provider {
            ProviderId::Claude => Some(&self.claude),
            ProviderId::Codex => Some(&self.codex),
            _ => None,
        }
    }
}

pub fn environment_directory(provider: ProviderId) -> Option<PathBuf> {
    let variable = match provider {
        ProviderId::Claude => "CLAUDE_CONFIG_DIR",
        ProviderId::Codex => "CODEX_HOME",
        _ => return None,
    };
    environment_directory_value(std::env::var_os(variable))
}

fn environment_directory_value(value: Option<std::ffi::OsString>) -> Option<PathBuf> {
    value.filter(|value| !value.is_empty()).and_then(|value| {
        let path = Path::new(&value);
        expand_path(path).or_else(|| std::env::current_dir().ok().map(|cwd| cwd.join(path)))
    })
}

fn is_windows_drive_absolute(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/')
}

pub fn expand_path(path: &Path) -> Option<PathBuf> {
    let text = path.to_string_lossy();
    if text == "~" {
        return dirs::home_dir();
    }
    if let Some(tail) = text.strip_prefix("~/").or_else(|| text.strip_prefix("~\\")) {
        return Some(dirs::home_dir()?.join(tail));
    }
    if path.is_absolute() || is_windows_drive_absolute(&text) {
        Some(path.to_path_buf())
    } else {
        // Settings must behave the same when launched from a terminal or at login.
        None
    }
}

impl AccountProfile {
    /// 두 명함이 같은 출처를 가리키는지 봅니다. 이름이 달라도 출처가 같으면 같습니다.
    pub fn same_source(&self, other: &Self) -> bool {
        self.id == other.id
            && self.config_dir == other.config_dir
            && self.credentials_path == other.credentials_path
    }

    /// 이 명함의 토큰 파일이 어디 있는지 계산합니다.
    /// - 파일 직접 지정이 있으면 그곳을 씁니다.
    /// - 없으면 폴더 지정 + 파일 이름 조합, 기본 명함(`default`)은 "없음"입니다.
    pub fn credential_path(&self, provider: ProviderId) -> Result<Option<PathBuf>, String> {
        if !self.credentials_path.trim().is_empty() {
            return expand_path(Path::new(self.credentials_path.trim()))
                .map(Some)
                .ok_or_else(|| "Use an absolute credentials file path or ~/path".into());
        }
        let directory = if !self.config_dir.trim().is_empty() {
            expand_path(Path::new(self.config_dir.trim()))
                .ok_or("Use an absolute config directory or ~/path")?
        } else if self.id == "default" {
            return Ok(None);
        } else {
            return Err(
                "Set a config directory or credentials file before enabling this account".into(),
            );
        };
        Ok(Some(directory.join(
            credential_file_name(provider)
                .ok_or("This provider does not support account profiles")?,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deleted_ids_are_not_reused_after_saving_and_restarting() {
        let mut accounts = ProviderAccounts::default();
        accounts.add();
        let removed_id = accounts.profiles.pop().unwrap().id;
        let json = serde_json::to_string(&accounts).unwrap();
        let mut reloaded: ProviderAccounts = serde_json::from_str(&json).unwrap();
        reloaded.add();
        assert_ne!(reloaded.profiles.last().unwrap().id, removed_id);
        assert!(reloaded.used_ids.contains(&removed_id));
    }

    #[test]
    fn legacy_ids_are_reserved_and_case_collisions_keep_the_selected_account() {
        let mut accounts: ProviderAccounts = serde_json::from_str(
            r#"{
            "profiles": [
                {"id":"Work","name":"First","config_dir":"C:/first"},
                {"id":"work","name":"Second","config_dir":"C:/second"},
                {"id":"account_1","name":"Existing"}
            ], "selected":"work"
        }"#,
        )
        .unwrap();
        accounts.normalize();
        assert_eq!(accounts.profiles[0].id, "Work");
        assert_eq!(accounts.profiles[2].id, "account_1");
        assert_ne!(accounts.profiles[1].id.to_ascii_lowercase(), "work");
        assert_eq!(accounts.selected().unwrap().name, "Second");
        let normalized = accounts.clone();
        accounts.normalize();
        assert_eq!(accounts, normalized);
        accounts.profiles.clear();
        accounts.add();
        assert!(!normalized
            .profiles
            .iter()
            .any(|old| old.id.eq_ignore_ascii_case(&accounts.profiles[0].id)));
    }

    #[test]
    fn environment_overrides_handle_empty_tilde_and_spaces_without_global_mutation() {
        assert_eq!(environment_directory_value(None), None);
        assert_eq!(environment_directory_value(Some("".into())), None);
        assert_eq!(
            environment_directory_value(Some("C:\\Users\\Two Words\\.claude-work".into())),
            Some(PathBuf::from("C:\\Users\\Two Words\\.claude-work"))
        );
        assert_eq!(
            environment_directory_value(Some("~/.codex-work".into())),
            dirs::home_dir().map(|home| home.join(".codex-work"))
        );
    }

    #[test]
    fn explicit_file_overrides_directory_and_invalid_profiles_never_use_default() {
        let profile = AccountProfile {
            id: "work".into(),
            config_dir: "C:\\work".into(),
            credentials_path: "C:\\private\\work.json".into(),
            ..Default::default()
        };
        assert_eq!(
            profile.credential_path(ProviderId::Claude).unwrap(),
            Some(PathBuf::from("C:\\private\\work.json"))
        );
        let empty = AccountProfile {
            id: "work".into(),
            ..Default::default()
        };
        assert!(empty.credential_path(ProviderId::Codex).is_err());
        assert!(expand_path(Path::new("relative/path")).is_none());
        assert_eq!(
            AccountProfile::default()
                .credential_path(ProviderId::Claude)
                .unwrap(),
            None
        );
    }

    #[test]
    fn disabled_or_removed_selection_uses_first_enabled_account() {
        let mut accounts = ProviderAccounts::default();
        accounts.add();
        accounts.profiles[1].enabled = true;
        accounts.profiles[0].enabled = false;
        accounts.normalize();
        assert_eq!(accounts.selected, "account_1");
        accounts.profiles.clear();
        accounts.normalize();
        assert!(accounts.selected().is_none());
    }
}
