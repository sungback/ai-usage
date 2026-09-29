// Keep the complete translation catalogue while the menu bar UI progressively
// adopts the legacy widget strings.

#[cfg(windows)]
use windows::core::PWSTR;
#[cfg(windows)]
use windows::Win32::Globalization::{
    GetUserDefaultLocaleName, GetUserDefaultUILanguage, GetUserPreferredUILanguages,
    LCIDToLocaleName, LOCALE_ALLOW_NEUTRAL_NAMES, MAX_LOCALE_NAME, MUI_LANGUAGE_NAME,
};

use crate::providers::ProviderId;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LanguageId(usize);

struct Locale {
    code: &'static str,
    #[allow(dead_code)]
    native_name: &'static str,
    locale_patterns: &'static [&'static str],
    #[allow(dead_code)]
    windows_font: Option<(&'static str, &'static str)>,
    strings: Strings,
    translations: &'static [(&'static str, &'static str)],
}

mod generated {
    use super::{LanguageId, Locale, Strings};

    include!(concat!(env!("OUT_DIR"), "/locales.rs"));
}

impl LanguageId {
    #[allow(non_upper_case_globals)]
    pub const Korean: Self = Self(generated::KOREAN_INDEX);

    pub(crate) const ALL: [Self; generated::LANGUAGE_COUNT] = generated::LANGUAGE_IDS;

    fn locale(self) -> &'static Locale {
        &generated::LOCALES[self.0]
    }

    pub fn code(self) -> &'static str {
        self.locale().code
    }

    pub fn strings(self) -> Strings {
        self.locale().strings
    }

    /// Translate user-interface text.
    ///
    /// The key is the canonical text in `ko.toml`, kept untranslated on purpose so a
    /// term awaiting review still renders as readable Korean rather than nothing.
    pub fn text(self, key: &'static str) -> &'static str {
        let locale = self.locale();
        locale
            .translations
            .binary_search_by(|(candidate, _)| (*candidate).cmp(key))
            .map(|index| locale.translations[index].1)
            .unwrap_or(key)
    }

    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn provider_auth_error(self, provider: ProviderId) -> (&'static str, &'static str) {
        let strings = self.strings();
        match provider {
            ProviderId::Claude => (strings.token_expired_title, strings.token_expired_body),
            ProviderId::Codex => (
                strings.codex_token_expired_title,
                strings.codex_token_expired_body,
            ),
            ProviderId::Antigravity => (
                strings.antigravity_token_expired_title,
                strings.antigravity_token_expired_body,
            ),
            ProviderId::OpenCode => (
                strings.opencode_token_expired_title,
                strings.opencode_token_expired_body,
            ),
            ProviderId::Cursor => (
                strings.cursor_token_expired_title,
                strings.cursor_token_expired_body,
            ),
        }
    }

    pub fn from_code(code: &str) -> Option<Self> {
        let normalized = code.trim().replace('_', "-").to_ascii_lowercase();
        if normalized.is_empty() || normalized == "system" {
            return None;
        }

        Self::ALL.into_iter().find(|language| {
            language.locale().locale_patterns.iter().any(|pattern| {
                normalized == *pattern
                    || normalized
                        .strip_prefix(pattern)
                        .is_some_and(|suffix| suffix.starts_with('-'))
            })
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Strings {
    pub window_title: &'static str,
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub models: &'static str,
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub claude_code_model: &'static str,
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub codex_model: &'static str,
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub antigravity_model: &'static str,
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub opencode_model: &'static str,
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub cursor_model: &'static str,
    #[cfg_attr(not(windows), allow(dead_code))]
    pub updates: &'static str,
    #[cfg_attr(not(windows), allow(dead_code))]
    pub update_in_progress: &'static str,
    #[cfg_attr(not(windows), allow(dead_code))]
    pub up_to_date: &'static str,
    #[cfg_attr(not(windows), allow(dead_code))]
    pub update_failed: &'static str,
    #[cfg_attr(not(windows), allow(dead_code))]
    pub update_available: &'static str,
    #[cfg_attr(not(windows), allow(dead_code))]
    pub update_prompt_now: &'static str,
    pub session_window: &'static str,
    pub weekly_window: &'static str,
    pub cursor_auto_window: &'static str,
    pub cursor_api_window: &'static str,
    pub now: &'static str,
    pub day_suffix: &'static str,
    pub hour_suffix: &'static str,
    pub minute_suffix: &'static str,
    pub second_suffix: &'static str,
    pub token_expired_title: &'static str,
    pub token_expired_body: &'static str,
    pub codex_token_expired_title: &'static str,
    pub codex_token_expired_body: &'static str,
    pub antigravity_token_expired_title: &'static str,
    pub antigravity_token_expired_body: &'static str,
    pub opencode_token_expired_title: &'static str,
    pub opencode_token_expired_body: &'static str,
    pub cursor_token_expired_title: &'static str,
    pub cursor_token_expired_body: &'static str,
}

pub fn resolve_language(language_override: Option<LanguageId>) -> LanguageId {
    language_override.unwrap_or_else(detect_system_language)
}

pub fn detect_system_language() -> LanguageId {
    #[cfg(windows)]
    {
        preferred_ui_languages()
            .into_iter()
            .find_map(|locale| LanguageId::from_code(&locale))
            .or_else(default_ui_locale)
            .or_else(default_locale_name)
            .unwrap_or(LanguageId::Korean)
    }
    #[cfg(not(windows))]
    {
        #[cfg(target_os = "macos")]
        if let Some(lang) = detect_macos_language() {
            return lang;
        }
        for var in ["LC_ALL", "LC_MESSAGES", "LANG"] {
            if let Ok(val) = std::env::var(var) {
                let code = val.split('.').next().unwrap_or(&val);
                if let Some(lang) = LanguageId::from_code(code) {
                    return lang;
                }
            }
        }
        LanguageId::Korean
    }
}

#[cfg(target_os = "macos")]
fn detect_macos_language() -> Option<LanguageId> {
    if let Ok(output) = std::process::Command::new("defaults")
        .args(["read", "-g", "AppleLocale"])
        .output()
    {
        if output.status.success() {
            let locale = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if let Some(lang) = LanguageId::from_code(&locale) {
                return Some(lang);
            }
        }
    }
    None
}

#[cfg(windows)]
fn preferred_ui_languages() -> Vec<String> {
    unsafe {
        let mut num_languages = 0u32;
        let mut buffer_len = 0u32;
        if GetUserPreferredUILanguages(MUI_LANGUAGE_NAME, &mut num_languages, None, &mut buffer_len)
            .is_err()
            || buffer_len == 0
        {
            return Vec::new();
        }

        let mut buffer = vec![0u16; buffer_len as usize];
        if GetUserPreferredUILanguages(
            MUI_LANGUAGE_NAME,
            &mut num_languages,
            Some(PWSTR(buffer.as_mut_ptr())),
            &mut buffer_len,
        )
        .is_err()
        {
            return Vec::new();
        }

        buffer
            .split(|unit| *unit == 0)
            .filter(|part| !part.is_empty())
            .map(String::from_utf16_lossy)
            .collect()
    }
}

#[cfg(windows)]
fn default_ui_locale() -> Option<LanguageId> {
    unsafe {
        let lang_id = GetUserDefaultUILanguage();
        let mut buffer = [0u16; MAX_LOCALE_NAME as usize];
        let len = LCIDToLocaleName(
            lang_id as u32,
            Some(&mut buffer),
            LOCALE_ALLOW_NEUTRAL_NAMES,
        );
        if len <= 1 {
            return None;
        }
        let locale = String::from_utf16_lossy(&buffer[..(len as usize - 1)]);
        LanguageId::from_code(&locale)
    }
}

#[cfg(windows)]
fn default_locale_name() -> Option<LanguageId> {
    unsafe {
        let mut buffer = [0u16; MAX_LOCALE_NAME as usize];
        let len = GetUserDefaultLocaleName(&mut buffer);
        if len <= 1 {
            return None;
        }
        let locale = String::from_utf16_lossy(&buffer[..(len as usize - 1)]);
        LanguageId::from_code(&locale)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shipped locale must actually translate these, not fall through to
    /// the catalogue key — an untranslated string is how a gap reaches the UI.
    /// Keys here are the ones the native menus and context menu still render.
    /// Provider names are absent on purpose: they are proper nouns that
    /// `ko.toml` keeps identical to the key.
    #[test]
    fn the_korean_locale_translates_every_key_the_app_still_renders() {
        let keys = [
            "Settings",
            "Refresh",
            "Refresh now",
            "Exit",
            "Providers",
            "Enabled",
            "Update frequency",
            "Every minute",
            "Every 5 minutes",
            "Every 15 minutes",
            "Every hour",
            "Start with Windows",
            "Show widget",
        ];
        for key in keys {
            assert_ne!(
                LanguageId::Korean.text(key),
                key,
                "Korean is missing the translation for {key:?}"
            );
        }
    }

    #[test]
    fn an_unknown_key_is_returned_unchanged() {
        assert_eq!(
            LanguageId::Korean.text("A future specialist label"),
            "A future specialist label"
        );
    }

    #[test]
    fn supported_locale_codes_are_recognized() {
        assert_eq!(LanguageId::Korean.code(), "ko");
        assert_eq!(LanguageId::from_code("ko"), Some(LanguageId::Korean));
        assert_eq!(LanguageId::from_code("ko-KR"), Some(LanguageId::Korean));
        assert_eq!(LanguageId::from_code("system"), None);
        assert_eq!(LanguageId::from_code("ja"), None);
    }

    #[test]
    fn the_translation_catalogue_is_sorted_and_populated() {
        let translations = LanguageId::Korean.locale().translations;
        assert!(!translations.is_empty());
        assert!(
            translations.windows(2).all(|pair| pair[0].0 < pair[1].0),
            "Korean translations are not sorted"
        );
        assert!(translations
            .iter()
            .all(|(_, value)| !value.trim().is_empty()));
    }
}
