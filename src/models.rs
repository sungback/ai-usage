//! 사용량 데이터 모양 — 처음 보시는 분을 위한 안내.
//!
//! - 공급자마다 세션·주간·월간·크레딧 구간을 들고, 화면은 이 값을 읽어 그립니다.
//! - `stale`은 "실패해서 직전 값을 보여주는 중" 표시입니다.

use std::collections::BTreeMap;
use std::time::SystemTime;

use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::localization::LanguageId;
use crate::providers::ProviderId;

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct UsageSection {
    /// 사용되지 않았거나 리셋 시각이 없더라도 공급자가 이 윈도우를 보고했는지 여부입니다.
    pub available: bool,
    pub percentage: f64,
    pub resets_at: Option<SystemTime>,
}

impl<'de> Deserialize<'de> for UsageSection {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct StoredSection {
            available: Option<bool>,
            percentage: f64,
            resets_at: Option<SystemTime>,
        }
        let stored = StoredSection::deserialize(deserializer)?;
        Ok(Self {
            // 이전 버전의 캐시는 유휴 윈도우와 부재 윈도우 간의 구분을 잃어버렸습니다.
            // 새로운 폴링이 수행될 때까지 존재 흔적을 보존합니다.
            available: stored
                .available
                .unwrap_or(stored.resets_at.is_some() || stored.percentage != 0.0),
            percentage: stored.percentage,
            resets_at: stored.resets_at,
        })
    }
}

/// 기본 포함 한도를 초과했을 때 공급자를 계속 사용할 수 있게 해주는 유료 크레딧입니다.
///
/// [`UsageData`]의 `None`은 표시할 크레딧 정보가 없음을 의미합니다: 크레딧이 꺼져 있거나,
/// 플랜에서 제공되지 않거나, 기본 한도에 아직 여유가 있어 아직 사용되지 않는 상태입니다.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CreditsSection {
    /// 현재 한도 중 이미 소비된 비율 (0 ~ 100).
    pub percentage: f64,
    /// 정수 통화 단위로 표현된 잔여 금액.
    pub remaining: f64,
    /// 정수 통화 단위의 기준 한도(총액): 플랜 한도가 있는 경우 해당 상한액,
    /// 없는 경우 마지막 관측된 충전 시의 잔액입니다.
    pub total: f64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct UsageData {
    pub session: UsageSection,
    pub weekly: UsageSection,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weekly_label: Option<String>,
    /// 선택적 장기 윈도우 사용량 (예: OpenCode Go 월간 윈도우).
    /// 테마가 표시 방식을 선택할 수 있도록 `weekly`와 별도로 유지됩니다.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub monthly: Option<UsageSection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credits: Option<CreditsSection>,
    /// 이번 주기에서 공급자 호출이 실패하여 이전 폴링의 측정치가 이월된 경우 true입니다.
    /// 실제 수치이기는 하나 최신 상태는 아닙니다.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub stale: bool,
}

impl UsageData {
    /// 게이지 표시용 퍼센티지: 카운트다운 모드일 때는 잔여량, 그 외에는 소비량입니다. 항상 0..=100 범위로 제한됩니다.
    pub fn shown(percentage: f64, countdown: bool) -> f64 {
        if countdown {
            (100.0 - percentage).clamp(0.0, 100.0)
        } else {
            percentage.clamp(0.0, 100.0)
        }
    }

    /// 게이지용 링 채움 비율 (0.0..=1.0).
    pub fn fill(percentage: f64, countdown: bool) -> f64 {
        Self::shown(percentage, countdown) / 100.0
    }
}

/// 사용량 임계치 경고 수준 (세션 소진율 기준).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThresholdLevel {
    Warn,
    Critical,
}

impl ThresholdLevel {
    pub const WARN_AT: f64 = 70.0;
    pub const CRITICAL_AT: f64 = 90.0;
}

/// 새로 임계치를 넘은 공급자 1건.
#[derive(Clone, Debug, PartialEq)]
pub struct ThresholdAlert {
    pub provider: ProviderId,
    pub level: ThresholdLevel,
    pub percentage: f64,
    pub resets_at: Option<SystemTime>,
}

/// Codex는 상한선 없이 크레딧 잔액만 보고하므로 기준 분모를 학습해야 합니다.
/// 잔액 증가는 충전으로 간주되며, 그 시점에 기록된 잔액이 다음 충전 전까지
/// 게이지 측정의 기준점이 됩니다.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CodexCreditsState {
    /// 이 상태가 속한 계정 ID입니다. 이전 상태 파일에는 기록되지 않았으며,
    /// 다른 계정에 기반한 잘못된 게이지가 표시되는 위험을 피하기 위해
    /// 계정 ID를 사용할 수 있게 되었을 때 의도적으로 다시 시드(seed)합니다.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    /// 직전 폴링에서 확인된 잔액 (원시 크레딧 단위).
    pub balance: f64,
    /// 마지막으로 관측된 충전 시점의 잔액 (원시 크레딧 단위). 최초 확인된 잔액으로 초기화됩니다.
    pub baseline: f64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AppUsageData {
    providers: BTreeMap<ProviderId, UsageData>,
    pub accounts: Vec<AccountUsage>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AccountUsage {
    pub provider: ProviderId,
    pub profile: crate::accounts::AccountProfile,
    pub source_signature: String,
    #[serde(default)]
    pub source_path: Option<std::path::PathBuf>,
    pub usage: Option<UsageData>,
    pub error: Option<crate::poller::PollError>,
    #[serde(default)]
    pub selected: bool,
}

impl AppUsageData {
    /// 인증 실패는 이 소스가 변경되거나 사용자가 명시적으로 재시도를 요청할 때까지 일시 정지 상태를 유지합니다.
    /// 다른 계정들은 독립적으로 정상 폴링됩니다.
    pub fn auth_error_for_source(
        &self,
        provider: ProviderId,
        profile: &crate::accounts::AccountProfile,
        signature: &str,
    ) -> Option<crate::poller::PollError> {
        self.accounts.iter().find_map(|account| {
            (account.provider == provider
                && account.profile.same_source(profile)
                && account.source_signature == signature)
                .then_some(account.error)
                .flatten()
                .filter(|error| error.is_auth())
        })
    }

    #[allow(dead_code)]
    pub fn new_auth_failures(&self, previous: Option<&Self>, force: bool) -> Vec<&AccountUsage> {
        self.accounts
            .iter()
            .filter(|account| {
                account.error.is_some_and(crate::poller::PollError::is_auth)
                    && (force
                        || previous
                            .and_then(|previous| {
                                previous.auth_error_for_source(
                                    account.provider,
                                    &account.profile,
                                    &account.source_signature,
                                )
                            })
                            .is_none())
            })
            .collect()
    }

    pub fn get(&self, provider: ProviderId) -> Option<&UsageData> {
        self.providers.get(&provider)
    }

    pub fn insert(&mut self, provider: ProviderId, usage: UsageData) -> Option<UsageData> {
        self.providers.insert(provider, usage)
    }

    pub fn is_empty(&self) -> bool {
        self.providers.is_empty() && self.accounts.iter().all(|account| account.usage.is_none())
    }

    pub fn iter(&self) -> impl Iterator<Item = (ProviderId, &UsageData)> {
        self.providers
            .iter()
            .map(|(provider, usage)| (*provider, usage))
    }

    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn all_usage(&self) -> impl Iterator<Item = &UsageData> {
        self.providers.values().chain(
            self.accounts
                .iter()
                .filter_map(|account| account.usage.as_ref()),
        )
    }

    /// 사용자가 선택한 계정들로부터 기존 공급자 바인딩을 재구성합니다.
    /// 선택된 계정이 없더라도 다른 계정의 캐시된 사용량을 표시해서는 안 됩니다.
    pub fn select_accounts(&mut self, settings: &crate::accounts::AccountSettings) {
        for account in &mut self.accounts {
            if let Some(profile) = settings.get(account.provider).and_then(|configured| {
                configured
                    .profiles
                    .iter()
                    .find(|profile| profile.enabled && profile.same_source(&account.profile))
            }) {
                account.profile = profile.clone();
            }
            account.selected = settings
                .get(account.provider)
                .and_then(|configured| configured.selected())
                .is_some_and(|selected| *selected == account.profile);
        }
        for provider in [ProviderId::Claude, ProviderId::Codex] {
            let Some(configured) = settings.get(provider) else {
                continue;
            };
            let tracked = self
                .accounts
                .iter()
                .any(|account| account.provider == provider);
            if tracked || configured != &crate::accounts::ProviderAccounts::default() {
                self.providers.remove(&provider);
                if let Some(selected) = configured.selected() {
                    if let Some(usage) = self
                        .accounts
                        .iter()
                        .find(|account| {
                            account.provider == provider && account.profile == *selected
                        })
                        .and_then(|account| account.usage.clone())
                    {
                        self.providers.insert(provider, usage);
                    }
                }
            }
        }
        self.accounts.retain(|account| {
            settings.get(account.provider).is_some_and(|configured| {
                configured
                    .profiles
                    .iter()
                    .any(|profile| profile.enabled && *profile == account.profile)
            })
        });
    }

    pub fn selected_account_name(&self, provider: ProviderId) -> Option<&str> {
        self.accounts
            .iter()
            .find(|account| account.provider == provider && account.selected)
            .map(|account| account.profile.name.as_str())
    }

    /// 메뉴 맨 윗줄("리셋까지 N시간")용: 세션 리셋 중 가장 이른 미래 시각.
    /// 과거 시각·없음은 제외하므로 호출자는 None이면 헤더를 숨기면 된다.
    pub fn earliest_session_reset(&self) -> Option<SystemTime> {
        let now = SystemTime::now();
        self.all_usage()
            .filter_map(|usage| usage.session.resets_at)
            .filter(|resets_at| *resets_at > now)
            .min()
    }
    /// 공급자별 세션 사용량 묶음 (기본 맵 + 선택 계정).
    fn session_entries(&self) -> Vec<(ProviderId, &UsageData)> {
        let mut entries: Vec<(ProviderId, &UsageData)> =
            self.providers.iter().map(|(id, usage)| (*id, usage)).collect();
        entries.extend(
            self.accounts
                .iter()
                .filter_map(|account| account.usage.as_ref().map(|usage| (account.provider, usage))),
        );
        entries
    }

    /// 해당 공급자의 세션 사용량 중 가장 높은 값.
    fn session_percentage(&self, provider: ProviderId) -> Option<f64> {
        self.session_entries()
            .into_iter()
            .filter(|(id, _)| *id == provider)
            .map(|(_, usage)| usage.session.percentage)
            .max_by(f64::total_cmp)
    }

    /// 이전 측정치 대비 새로 임계치를 넘은 세션 사용량을 찾는다.
    /// `previous`가 없으면(첫 폴링) 조용히 넘어가 재시작 스팸을 막고,
    /// 사용량이 떨어졌다가 다시 오르면 다시 알린다.
    pub fn threshold_crossings(&self, previous: Option<&Self>) -> Vec<ThresholdAlert> {
        let Some(previous) = previous else {
            return Vec::new();
        };
        let mut highest: BTreeMap<ProviderId, &UsageData> = BTreeMap::new();
        for (id, usage) in self.session_entries() {
            highest
                .entry(id)
                .and_modify(|kept| {
                    if usage.session.percentage > kept.session.percentage {
                        *kept = usage;
                    }
                })
                .or_insert(usage);
        }
        let mut alerts = Vec::new();
        for (provider, usage) in highest {
            let next = usage.session.percentage;
            let (level, threshold) = if next >= ThresholdLevel::CRITICAL_AT {
                (ThresholdLevel::Critical, ThresholdLevel::CRITICAL_AT)
            } else if next >= ThresholdLevel::WARN_AT {
                (ThresholdLevel::Warn, ThresholdLevel::WARN_AT)
            } else {
                continue;
            };
            let Some(prev) = previous.session_percentage(provider) else {
                continue;
            };
            if prev < threshold {
                alerts.push(ThresholdAlert {
                    provider,
                    level,
                    percentage: next,
                    resets_at: usage.session.resets_at,
                });
            }
        }
        alerts
    }

    /// 캐시된 측정치는 로그인 변경이나 상속된 다른 설정 디렉터리를 넘어서 유지되지 않아야 합니다.
    /// 파일 상태(stat)만 확인하며, CLI나 WSL을 실행하지 않습니다.
    #[cfg(any(target_os = "macos", test))]
    pub fn invalidate_changed_credentials(&mut self) {
        for account in &mut self.accounts {
            let expected = account
                .profile
                .credential_path(account.provider)
                .ok()
                .flatten()
                .or_else(|| crate::accounts::default_credential_path(account.provider));
            let changed = match &account.source_path {
                Some(path) => {
                    expected.as_ref().is_none_or(|expected| {
                        crate::accounts::source_key(expected) != crate::accounts::source_key(path)
                    }) || crate::poller::account_source_signature(account.provider, path)
                        != account.source_signature
                }
                None => crate::accounts::environment_directory(account.provider).is_some(),
            };
            if changed {
                account.usage = None;
                account.error = None;
                self.providers.remove(&account.provider);
            }
        }
    }
}

/// 공급자별 사용량 요약 줄 데이터입니다.
#[derive(Clone, Debug, PartialEq)]
pub struct ProviderUsageSummaryItem {
    pub key: &'static str,
    pub header_text: String,
    pub reset_text: Option<String>,
}

/// 잔여 시각 포맷 헬퍼 ("3h 57m", "45m", "Now").
pub fn format_reset_time(resets_at: Option<SystemTime>) -> Option<String> {
    let resets_at = resets_at?;
    let now = SystemTime::now();
    if resets_at > now {
        let diff = resets_at.duration_since(now).ok()?;
        let total_mins = diff.as_secs() / 60;
        let hours = total_mins / 60;
        let mins = total_mins % 60;
        if hours > 0 {
            Some(format!("{hours}h {mins}m"))
        } else {
            Some(format!("{mins}m"))
        }
    } else {
        Some("Now".to_string())
    }
}

/// 트레이/위젯 컨텍스트 메뉴 최상단 세션 리셋 카운트다운 헤더 문구.
pub fn reset_countdown_header(
    data: &AppUsageData,
    lang: LanguageId,
) -> Option<String> {
    let resets_at = data.earliest_session_reset()?;
    let remaining = resets_at.duration_since(SystemTime::now()).ok()?;
    let strings = lang.strings();
    let total_mins = remaining.as_secs() / 60;
    if total_mins == 0 {
        return Some(format!("⏰ {}", strings.now));
    }
    if lang.code() == "ko" {
        let days = total_mins / (24 * 60);
        let hours = (total_mins % (24 * 60)) / 60;
        let mins = total_mins % 60;
        let body = if days > 0 {
            format!(
                "{}{} {}{} {}{}",
                days, strings.day_suffix, hours, strings.hour_suffix, mins, strings.minute_suffix
            )
        } else if hours > 0 {
            format!("{}{} {}{}", hours, strings.hour_suffix, mins, strings.minute_suffix)
        } else {
            format!("{}{}", mins, strings.minute_suffix)
        };
        Some(format!("⏰ 세션 리셋까지 {body}"))
    } else {
        Some(format!(
            "⏰ Session reset in {}",
            format_reset_time(Some(resets_at))?
        ))
    }
}

/// 컨텍스트 메뉴에 노출할 공급자별 사용량 요약 목록 빌드.
///
/// `ordered_providers`가 비어있지 않으면 해당 순서 및 필터링을 따르고,
/// 비어있다면 `data`에 포함된 모든 공급자를 순회합니다.
pub fn build_usage_summary_items(
    data: &AppUsageData,
    ordered_providers: &[ProviderId],
    countdown: bool,
    lang: LanguageId,
) -> Vec<ProviderUsageSummaryItem> {
    let strings = lang.strings();
    let providers_to_show: Vec<ProviderId> = if ordered_providers.is_empty() {
        data.iter().map(|(p, _)| p).collect()
    } else {
        ordered_providers.to_vec()
    };

    let mut items = Vec::new();
    for provider in providers_to_show {
        if let Some(usage) = data.get(provider) {
            let desc = provider.descriptor();
            let provider_name = lang.text(desc.display_name);
            let (session_pct, weekly_pct) = (
                UsageData::shown(usage.session.percentage, countdown),
                UsageData::shown(usage.weekly.percentage, countdown),
            );
            let header_text = format!(
                "{} - {}: {:.0}% | {}: {:.0}%{}",
                provider_name,
                strings.session_window,
                session_pct,
                usage.weekly_label.as_deref().unwrap_or(strings.weekly_window),
                weekly_pct,
                if usage.stale { " ⚠" } else { "" },
            );
            let reset_text = format_reset_time(usage.session.resets_at).map(|reset_str| {
                format!("  {} {reset_str}", lang.text("Resets in:"))
            });
            items.push(ProviderUsageSummaryItem {
                key: desc.key,
                header_text,
                reset_text,
            });
        }
    }

    items
}

impl FromIterator<(ProviderId, UsageData)> for AppUsageData {
    fn from_iter<T: IntoIterator<Item = (ProviderId, UsageData)>>(iter: T) -> Self {
        Self {
            providers: iter.into_iter().collect(),
            accounts: Vec::new(),
        }
    }
}

impl Serialize for AppUsageData {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(None)?;
        for (provider, usage) in &self.providers {
            map.serialize_entry(provider.descriptor().cache_key, usage)?;
        }
        if !self.accounts.is_empty() {
            map.serialize_entry("accounts", &self.accounts)?;
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for AppUsageData {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let mut values = BTreeMap::<String, serde_json::Value>::deserialize(deserializer)?;
        let accounts = values
            .remove("accounts")
            .map(serde_json::from_value)
            .transpose()
            .map_err(serde::de::Error::custom)?
            .unwrap_or_default();
        let mut data: Self = values
            .into_iter()
            .filter_map(|(key, usage)| {
                let usage = serde_json::from_value::<Option<UsageData>>(usage).ok()??;
                ProviderId::from_cache_key(&key)
                    .or_else(|| ProviderId::from_key(&key))
                    .map(|provider| (provider, usage))
            })
            .collect();
        data.accounts = accounts;
        Ok(data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shown_and_fill_cover_countdown_and_direct_modes() {        assert_eq!(UsageData::shown(30.0, true), 70.0);
        assert_eq!(UsageData::shown(30.0, false), 30.0);
        assert_eq!(UsageData::fill(30.0, true), 0.7);
        assert_eq!(UsageData::fill(30.0, false), 0.3);
        assert_eq!(UsageData::shown(-5.0, false), 0.0);
        assert_eq!(UsageData::shown(140.0, false), 100.0);
        assert_eq!(UsageData::shown(140.0, true), 0.0);
        assert_eq!(UsageData::fill(140.0, true), 0.0);
    }

    #[test]
    fn earliest_session_reset_picks_the_nearest_future() {
        use std::time::Duration;
        let now = SystemTime::now();
        let past = now - Duration::from_secs(60);
        let near = now + Duration::from_secs(3_600);
        let far = now + Duration::from_secs(7_200);
        let section = |resets_at: Option<SystemTime>| UsageSection {
            available: true,
            percentage: 10.0,
            resets_at,
        };
        let data: AppUsageData = [
            (
                ProviderId::Claude,
                UsageData {
                    session: section(Some(far)),
                    ..Default::default()
                },
            ),
            (
                ProviderId::Codex,
                UsageData {
                    session: section(Some(near)),
                    ..Default::default()
                },
            ),
            (
                ProviderId::Cursor,
                UsageData {
                    session: section(Some(past)),
                    ..Default::default()
                },
            ),
        ]
        .into_iter()
        .collect();
        assert_eq!(data.earliest_session_reset(), Some(near));
        assert_eq!(AppUsageData::default().earliest_session_reset(), None);
    }

    #[test]
    fn threshold_crossings_fire_once_per_rising_edge() {
        let entry = |percentage: f64| UsageData {
            session: UsageSection {
                available: true,
                percentage,
                resets_at: None,
            },
            ..Default::default()
        };
        let snapshot = |claude: f64, codex: f64| {
            [
                (ProviderId::Claude, entry(claude)),
                (ProviderId::Codex, entry(codex)),
            ]
            .into_iter()
            .collect::<AppUsageData>()
        };
        let levels = |alerts: &[ThresholdAlert]| {
            alerts
                .iter()
                .map(|alert| (alert.provider, alert.level))
                .collect::<Vec<_>>()
        };

        // 첫 폴링에서는 조용 (재시작 스팸 방지).
        assert!(snapshot(95.0, 95.0).threshold_crossings(None).is_empty());

        // 70% 상승 돌파에서 Warn 1건.
        let prev = snapshot(65.0, 10.0);
        let next = snapshot(75.0, 10.0);
        assert_eq!(
            levels(&next.threshold_crossings(Some(&prev))),
            [(ProviderId::Claude, ThresholdLevel::Warn)]
        );

        // 90% 돌파는 Critical 1건만 (Warn 중복 없음).
        let next = snapshot(95.0, 10.0);
        let alerts = next.threshold_crossings(Some(&prev));
        assert_eq!(levels(&alerts), [(ProviderId::Claude, ThresholdLevel::Critical)]);
        assert_eq!(alerts[0].percentage, 95.0);

        // 이미 넘은 상태에서는 조용.
        let higher = snapshot(96.0, 10.0);
        assert!(higher.threshold_crossings(Some(&next)).is_empty());

        // 떨어졌다가 다시 오르면 다시 알림.
        let dropped = snapshot(10.0, 10.0);
        assert!(dropped.threshold_crossings(Some(&higher)).is_empty());
        assert_eq!(
            levels(&next.threshold_crossings(Some(&dropped))),
            [(ProviderId::Claude, ThresholdLevel::Critical)]
        );

        // 공급자가 새로 나타나도 조용 (관측된 상승이 아님).
        let prev = snapshot(10.0, 10.0);
        let mut next = prev.clone();
        next.insert(ProviderId::Cursor, entry(99.0));
        assert!(next.threshold_crossings(Some(&prev)).is_empty());
    }
    /// 실제 기기의 파일 대신 임시 자격 증명 파일을 사용합니다:
    /// `invalidate_changed_credentials`가 소스를 다시 읽으므로, 두 번의 읽기 사이에
    /// 실제 Claude Code 로그인이 발생하면 단언(assertion)이 실패할 수 있기 때문입니다.
    #[test]
    fn cached_usage_is_dropped_when_the_credential_source_changes() {
        let provider = ProviderId::Claude;
        let directory = std::env::temp_dir().join(format!(
            "usage-source-signature-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join(".credentials.json");
        std::fs::write(&path, "fixture before rotation").unwrap();
        let profile = crate::accounts::AccountProfile {
            credentials_path: path.to_string_lossy().into_owned(),
            ..Default::default()
        };
        // 지문만 사용: 어떤 토큰도 복호화, 사용, 변경되지 않습니다.
        for error in [None, Some(crate::poller::PollError::HttpStatus(429))] {
            // 각 라운드는 예상되는 디스크 상태에서 시작합니다. 빠른 연속 쓰기 시
            // 파일 시스템이 mtime을 갱신하지 못할 수 있으므로, 루프 내부에서 시그니처를 읽습니다.
            std::fs::write(&path, "fixture before rotation").unwrap();
            let signature = crate::poller::account_source_signature(provider, &path);
            let mut data = AppUsageData::default();
            data.accounts.push(AccountUsage {
                provider,
                profile: profile.clone(),
                source_signature: signature.clone(),
                source_path: Some(path.clone()),
                usage: error.is_none().then(UsageData::default),
                error,
                selected: true,
            });
            let json = serde_json::to_string(&data).unwrap();
            let mut cached: AppUsageData = serde_json::from_str(&json).unwrap();
            cached.invalidate_changed_credentials();
            assert_eq!(cached.accounts, data.accounts);

            // 디스크의 토큰이 변경되면 캐시된 측정치가 무효화되어야 합니다.
            std::fs::write(&path, "fixture after rotation, now longer").unwrap();
            cached.invalidate_changed_credentials();
            assert!(cached.accounts[0].usage.is_none());
            assert!(cached.accounts[0].error.is_none());

            std::fs::write(&path, "fixture before rotation").unwrap();
            cached.accounts[0].source_signature.push_str("changed");
            cached.invalidate_changed_credentials();
            assert!(cached.accounts[0].usage.is_none());
            assert!(cached.accounts[0].error.is_none());
        }
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn usage_cache_preserves_idle_window_presence_and_reads_legacy_sections() {
        for available in [false, true] {
            let section = UsageSection {
                available,
                ..Default::default()
            };
            let json = serde_json::to_value(&section).unwrap();
            assert_eq!(json["available"], available);
            assert_eq!(
                serde_json::from_value::<UsageSection>(json).unwrap(),
                section
            );
        }
        for (json, expected) in [
            (r#"{"percentage":0,"resets_at":null}"#, false),
            (r#"{"percentage":42,"resets_at":null}"#, true),
            (
                r#"{"percentage":0,"resets_at":{"secs_since_epoch":0,"nanos_since_epoch":0}}"#,
                true,
            ),
            (
                r#"{"available":false,"percentage":42,"resets_at":null}"#,
                false,
            ),
        ] {
            let section: UsageSection = serde_json::from_str(json).unwrap();
            assert_eq!(section.available, expected);
        }
    }

    #[test]
    fn usage_cache_keeps_legacy_provider_keys() {
        let data: AppUsageData = [
            (ProviderId::Claude, UsageData::default()),
            (ProviderId::Codex, UsageData::default()),
            (
                ProviderId::OpenCode,
                UsageData {
                    weekly_label: Some("30d".into()),
                    monthly: Some(UsageSection {
                        available: true,
                        percentage: 43.0,
                        resets_at: None,
                    }),
                    ..Default::default()
                },
            ),
        ]
        .into_iter()
        .collect();

        let json = serde_json::to_value(&data).unwrap();
        assert!(json.get("claude_code").is_some());
        assert!(json.get("codex").is_some());
        assert_eq!(json["opencode"]["weekly_label"], "30d");
        assert_eq!(json["opencode"]["monthly"]["percentage"], 43.0);
        assert!(json.get("claude").is_none());

        let decoded: AppUsageData = serde_json::from_value(json).unwrap();
        assert_eq!(decoded, data);
    }

    #[test]
    fn usage_cache_accepts_nulls_from_the_legacy_struct_format() {
        let decoded: AppUsageData = serde_json::from_str(
            r#"{
                "claude_code": null,
                "codex": {"session":{"percentage":42.0,"resets_at":null},"weekly":{"percentage":0.0,"resets_at":null}},
                "antigravity": null
                ,"opencode": null
            }"#,
        )
        .unwrap();

        assert!(decoded.get(ProviderId::Claude).is_none());
        assert_eq!(
            decoded.get(ProviderId::Codex).unwrap().session.percentage,
            42.0
        );
        assert!(decoded.get(ProviderId::Antigravity).is_none());
        assert!(decoded.get(ProviderId::OpenCode).is_none());
    }

    #[test]
    fn test_format_reset_time() {
        assert_eq!(format_reset_time(None), None);

        let now = SystemTime::now();
        let past = now - std::time::Duration::from_secs(60);
        assert_eq!(format_reset_time(Some(past)), Some("Now".to_string()));

        let future_mins = now + std::time::Duration::from_secs(45 * 60 + 10);
        assert_eq!(format_reset_time(Some(future_mins)), Some("45m".to_string()));

        let future_hours = now + std::time::Duration::from_secs(3 * 3600 + 15 * 60 + 5);
        assert_eq!(format_reset_time(Some(future_hours)), Some("3h 15m".to_string()));

        let future_exact_hour = now + std::time::Duration::from_secs(3600 + 5);
        assert_eq!(format_reset_time(Some(future_exact_hour)), Some("1h 0m".to_string()));
    }

    #[test]
    fn test_reset_countdown_header() {
        let now = SystemTime::now();
        let mut data = AppUsageData::default();

        assert_eq!(reset_countdown_header(&data, LanguageId::Korean), None);

        let mut usage = UsageData::default();
        usage.session.available = true;
        usage.session.resets_at = Some(now + std::time::Duration::from_secs(2 * 3600 + 30 * 60 + 5));
        data.insert(ProviderId::Claude, usage);

        let header = reset_countdown_header(&data, LanguageId::Korean);
        assert!(header.is_some());
        let text = header.unwrap();
        assert!(text.contains("⏰ 세션 리셋까지"));
        assert!(text.contains("2시간 30분"));
    }

    #[test]
    fn test_build_usage_summary_items() {
        let now = SystemTime::now();
        let mut data = AppUsageData::default();

        let mut claude_usage = UsageData::default();
        claude_usage.session.percentage = 72.0;
        claude_usage.session.resets_at = Some(now + std::time::Duration::from_secs(3 * 3600 + 57 * 60 + 5));
        claude_usage.weekly.percentage = 42.0;
        data.insert(ProviderId::Claude, claude_usage);

        let mut codex_usage = UsageData::default();
        codex_usage.session.percentage = 96.0;
        codex_usage.session.resets_at = Some(now + std::time::Duration::from_secs(3600 + 12 * 60 + 5));
        codex_usage.weekly.percentage = 75.0;
        data.insert(ProviderId::Codex, codex_usage);

        let mut anti_usage = UsageData::default();
        anti_usage.session.percentage = 100.0;
        anti_usage.session.resets_at = Some(now + std::time::Duration::from_secs(4 * 3600 + 59 * 60 + 5));
        anti_usage.weekly.percentage = 59.0;
        anti_usage.stale = true;
        data.insert(ProviderId::Antigravity, anti_usage);

        // Standard used mode (countdown = false)
        let items = build_usage_summary_items(
            &data,
            &[ProviderId::Claude, ProviderId::Codex, ProviderId::Antigravity],
            false,
            LanguageId::Korean,
        );
        assert_eq!(items.len(), 3);

        assert_eq!(items[0].key, "claude");
        assert!(items[0].header_text.contains("Claude Code - 5시간: 72% | 7일: 42%"));
        assert_eq!(items[0].reset_text.as_deref(), Some("  Resets in: 3h 57m"));

        assert_eq!(items[1].key, "codex");
        assert!(items[1].header_text.contains("Codex - 5시간: 96% | 7일: 75%"));
        assert_eq!(items[1].reset_text.as_deref(), Some("  Resets in: 1h 12m"));

        assert_eq!(items[2].key, "antigravity");
        assert!(items[2].header_text.contains("Antigravity - 5시간: 100% | 7일: 59% ⚠"));
        assert_eq!(items[2].reset_text.as_deref(), Some("  Resets in: 4h 59m"));

        // Remaining mode (countdown = true)
        let remaining_items = build_usage_summary_items(
            &data,
            &[ProviderId::Claude],
            true,
            LanguageId::Korean,
        );
        assert_eq!(remaining_items.len(), 1);
        assert!(remaining_items[0].header_text.contains("Claude Code - 5시간: 28% | 7일: 58%"));
    }
}
