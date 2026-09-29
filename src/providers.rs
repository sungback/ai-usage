//! 공급자(Provider) 등록부 — 처음 보시는 분을 위한 안내.
//!
//! 이 파일은 "어떤 AI 도구를 추적할까?"의 정답지입니다.
//! - 새 공급자를 추가하는 순서는 딱 3단계입니다:
//!   1) 아래 `ProviderId`에 새 항목을 붙이고,
//!   2) `PROVIDER_DESCRIPTORS`에 설명 한 줄을 등록하고,
//!   3) `src/poller/`에 실제 사용량을 가져오는 코드를 연결합니다.
//! - `key` / `cache_key` / `native_menu_command_id`는 한 번 정하면 바꾸지 않습니다.
//!   설정 파일·캐시·네이티브 메뉴가 이 문자열과 숫자로 서로를 찾기 때문입니다.

use serde::{Deserialize, Serialize};

/// 설정·폴링·테마·메뉴가 함께 쓰는 공급자의 주민등록번호입니다.
///
/// 숫자는 저장 형식과 맞아야 해서(위 `repr(u8)`) 함부로 순서를 바꾸지 마세요.
/// 새 공급자는 맨 뒤에 추가하는 것이 가장 안전합니다.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum ProviderId {
    Claude = 0,
    Codex = 1,
    Antigravity = 2,
    OpenCode = 3,
    Cursor = 4,
}

/// 공급자 한 줄 설명서입니다. 화면에 보이는 이름이 아니라,
/// "서로를 찾는 데 쓰는 고정 키"를 모아 둔 표라고 이해하면 됩니다.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProviderDescriptor {
    pub id: ProviderId,
    /// 테마 표현식·메뉴 문서가 쓰는 고정 키 (예: `"claude"`).
    pub key: &'static str,
    /// 저장된 사용량 캐시가 쓰는 고정 키 (예: `"claude_code"`).
    pub cache_key: &'static str,
    /// 화면에 보여 줄 이름의 번역 키입니다. 실제 한글 문구는 `ko.toml`에 있습니다.
    pub display_name: &'static str,
    /// 설정 화면의 설명문 번역 키입니다.
    pub settings_description: &'static str,
    /// Windows 네이티브 메뉴가 쓰는 고정 명령 번호입니다. 겹치면 안 됩니다.
    pub native_menu_command_id: u16,
    /// 앱을 처음 설치했을 때 켜져 있을지 여부입니다.
    pub default_enabled: bool,
}

/// 공급자 설명서 5장짜리 표입니다. 순서가 `ProviderId` 숫자와 딱 맞아야 합니다.
pub const PROVIDER_DESCRIPTORS: [ProviderDescriptor; 5] = [
    ProviderDescriptor {
        id: ProviderId::Claude,
        key: "claude",
        cache_key: "claude_code",
        display_name: "Claude Code",
        settings_description: "Collect usage from Anthropic",
        native_menu_command_id: 60,
        default_enabled: true,
    },
    ProviderDescriptor {
        id: ProviderId::Codex,
        key: "codex",
        cache_key: "codex",
        display_name: "Codex",
        settings_description: "Collect usage from OpenAI",
        native_menu_command_id: 61,
        default_enabled: false,
    },
    ProviderDescriptor {
        id: ProviderId::Antigravity,
        key: "antigravity",
        cache_key: "antigravity",
        display_name: "Antigravity",
        settings_description: "Collect usage from Google",
        native_menu_command_id: 62,
        default_enabled: false,
    },
    ProviderDescriptor {
        id: ProviderId::OpenCode,
        key: "opencode",
        cache_key: "opencode",
        display_name: "OpenCode",
        settings_description: "Collect usage from OpenCode Go",
        native_menu_command_id: 63,
        default_enabled: false,
    },
    ProviderDescriptor {
        id: ProviderId::Cursor,
        key: "cursor",
        cache_key: "cursor",
        display_name: "Cursor",
        settings_description: "Collect usage from Cursor",
        native_menu_command_id: 64,
        default_enabled: false,
    },
];

impl ProviderId {
    pub const ALL: [Self; 5] = [
        Self::Claude,
        Self::Codex,
        Self::Antigravity,
        Self::OpenCode,
        Self::Cursor,
    ];

    /// 자신의 설명서를 꺼냅니다. 숫자와 표의 순서가 항상 일치한다는 전제를 믿습니다.
    pub const fn descriptor(self) -> &'static ProviderDescriptor {
        // NOTE(인수인계): 새 공급자를 맨 뒤가 아닌 중간에 끼워 넣으면
        // 이 인덱스가 어긋납니다. 그럴 땐 표의 순서도 함께 옮겨 주세요.
        // [필수 주석: 공개 API - 고정 키 계약을 깨면 설정·캐시·메뉴가 서로를 못 찾습니다]
        debug_assert!((self as usize) < PROVIDER_DESCRIPTORS.len());
        &PROVIDER_DESCRIPTORS[self as usize]
    }

    /// 조건에 맞는 첫 공급자를 찾는 작은 엔진입니다.
    /// 아래 `from_key` 3종이 겉모습만 다르고 속은 같은 이유입니다.
    fn find(matches: impl Fn(&ProviderDescriptor) -> bool) -> Option<Self> {
        PROVIDER_DESCRIPTORS
            .iter()
            .find(|descriptor| matches(descriptor))
            .map(|descriptor| descriptor.id)
    }

    /// 처음 설치된 앱이 켜야 할 기본 공급자입니다. 표에서 `default_enabled`를 찾습니다.
    fn default_id() -> Self {
        Self::find(|descriptor| descriptor.default_enabled).unwrap_or(Self::Claude)
    }

    /// 설정 파일에 적힌 짧은 키(예: `"codex"`)로 공급자를 되살립니다.
    pub fn from_key(key: &str) -> Option<Self> {
        Self::find(|descriptor| descriptor.key == key)
    }

    /// 저장된 캐시 키(예: `"claude_code"`)로 공급자를 되살립니다.
    /// 옛날 키와 새 키가 다를 수 있어 `from_key`와 따로 둡니다.
    pub fn from_cache_key(key: &str) -> Option<Self> {
        Self::find(|descriptor| descriptor.cache_key == key)
    }

    /// Windows 메뉴 번호(60~64)로 공급자를 되살립니다.
    /// macOS 단독 빌드에서는 메뉴가 없어 쉬는 함수라 경고를 꺼 둡니다.
    #[allow(dead_code)]
    pub fn from_native_menu_command_id(command_id: u16) -> Option<Self> {
        Self::find(|descriptor| descriptor.native_menu_command_id == command_id)
    }

    /// 켜짐·꺼짐을 비트 하나로 기억하기 위한 자리 번호입니다.
    /// `ProviderSet`이 `u64` 하나에 5개 스위치를 담는 비밀입니다.
    const fn bit(self) -> u64 {
        1 << self as u8
    }
}

impl Default for ProviderId {
    fn default() -> Self {
        Self::default_id()
    }
}

/// 켜진 공급자들의 명단입니다. `u64` 숫자 하나에 5개의 스위치를 담습니다.
/// - 가볍게 복사해서 설정·화면·폴링 스레드가 돌려 씁니다.
/// - "하나도 안 켜짐" 상태를 허용하므로, 호출하는 쪽에서 비었는지 확인해야 합니다.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProviderSet(u64);

impl ProviderSet {
    /// 아무것도 켜지 않은 빈 명단입니다.
    pub const fn empty() -> Self {
        Self(0)
    }

    /// 켤 목록을 받아 명단을 만듭니다. (예: `[Claude, Codex]` → 둘 다 켜짐)
    pub fn from_enabled(enabled: impl IntoIterator<Item = ProviderId>) -> Self {
        let mut providers = Self::empty();
        for provider in enabled {
            providers.set(provider, true);
        }
        providers
    }

    /// 이 공급자가 켜져 있는지 묻습니다.
    pub const fn contains(self, provider: ProviderId) -> bool {
        self.0 & provider.bit() != 0
    }

    /// 켜거나 끕니다. 존재하지 않는 공급자를 꺼도 조용히 무시되지 않고 비트 연산만 됩니다.
    pub fn set(&mut self, provider: ProviderId, enabled: bool) {
        if enabled {
            self.0 |= provider.bit();
        } else {
            self.0 &= !provider.bit();
        }
    }

    /// 켜짐·꺼짐을 뒤집습니다. 단, 마지막 1개까지 끄는 것은 거절하고 `false`를 돌려줍니다.
    /// (아무것도 안 켜진 앱이 되면 안 되니까요.)
    /// macOS는 설정 파일 경로로 토글해서 이 함수는 Windows 전용입니다.
    // [필수 주석: 마지막 공급자 끄기 금지 - 앱 불변 조건을 지키는 안전장치]
    #[cfg(windows)]
    pub fn toggle(&mut self, provider: ProviderId) -> bool {
        let enabled = self.contains(provider);
        if enabled && self.len() == 1 {
            return false;
        }
        self.set(provider, !enabled);
        true
    }

    /// 아무것도 안 켜졌는지 묻습니다.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// 켜진 개수를 셉니다.
    pub const fn len(self) -> usize {
        self.0.count_ones() as usize
    }

    /// 켜진 것 중 맨 앞(고정 순서상 첫 번째)을 꺼냅니다.
    pub fn first(self) -> Option<ProviderId> {
        self.iter().next()
    }

    /// 켜진 공급자들을 고정 순서대로 돌려줍니다.
    pub fn iter(self) -> impl Iterator<Item = ProviderId> {
        ProviderId::ALL
            .into_iter()
            .filter(move |provider| self.contains(*provider))
    }
}

impl Default for ProviderSet {
    /// 표에 `default_enabled`로 적힌 것들만 켠 명단입니다. 지금은 Claude 하나입니다.
    fn default() -> Self {
        Self::from_enabled(
            PROVIDER_DESCRIPTORS
                .iter()
                .filter(|descriptor| descriptor.default_enabled)
                .map(|descriptor| descriptor.id),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_provider_set_comes_from_descriptors() {
        assert_eq!(
            ProviderSet::default(),
            ProviderSet::from_enabled([ProviderId::Claude])
        );
    }

    #[cfg(windows)]
    #[test]
    fn provider_set_refuses_to_toggle_off_its_last_provider() {
        let mut providers = ProviderSet::from_enabled([ProviderId::Codex]);
        assert!(!providers.toggle(ProviderId::Codex));
        assert!(providers.contains(ProviderId::Codex));
    }

    #[test]
    fn provider_keys_round_trip_through_the_registry() {
        for descriptor in PROVIDER_DESCRIPTORS {
            assert_eq!(ProviderId::from_key(descriptor.key), Some(descriptor.id));
            assert_eq!(
                ProviderId::from_cache_key(descriptor.cache_key),
                Some(descriptor.id)
            );
            assert_eq!(
                ProviderId::from_native_menu_command_id(descriptor.native_menu_command_id),
                Some(descriptor.id)
            );
        }
    }
}
