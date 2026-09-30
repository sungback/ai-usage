//! Claude 데스크톱 앱이 번들된 Claude Code 빌드를 위해 보관하는 OAuth 토큰을 읽습니다.
//!
//! 데스크톱 앱을 통해서만 Claude Code를 실행했던 기기에는 `~/.claude/.credentials.json` 파일이 없습니다.
//! 해당 파일은 독립 실행형 CLI 로그인 과정에서만 생성되기 때문입니다.
//! 데스크톱 앱은 Electron 애플리케이션으로, 대신 Chromium의 OSCrypt 방식을 사용하여 토큰 캐시를 저장합니다.
//! `Local State` 파일 내에 DPAPI로 래핑된 AES-256-GCM 키가 들어있고,
//! 각 암호화된 값은 `"v10" || nonce || ciphertext || tag` 형태를 가집니다.
//!
//! 여기에 있는 모든 작업은 읽기 전용이며, 로그인한 사용자의 권한으로 실행되고,
//! 레이아웃이 예상과 다를 경우 안전하게 `None`으로 대체됩니다.

#![cfg_attr(not(windows), allow(dead_code))]

#[cfg(windows)]
use std::ffi::c_void;
use std::path::{Path, PathBuf};

/// 최신 레이아웃 우선. 데스크톱 앱이 캐시를 `oauth:tokenCacheV2`로 마이그레이션하면서
/// 기존의 `oauth:tokenCache` 키도 그대로 남겨두었으므로, 둘 다 시도하여 사용 가능한 토큰을 먼저 반환하는 쪽을 사용합니다.
const TOKEN_CACHE_KEYS: &[&str] = &["oauth:tokenCacheV2", "oauth:tokenCache"];
const DPAPI_KEY_PREFIX: &[u8] = b"DPAPI";
const OS_CRYPT_PREFIX: &[u8] = b"v10";
const GCM_NONCE_LEN: usize = 12;
const GCM_TAG_LEN: usize = 16;
/// 데스크톱 항목 키 형식은 `"<install>:<user>:<base url>:<scopes>"`입니다.
/// 추론(inference) 스코프가 있는 토큰이 사용량 엔드포인트에서 허용되는 토큰입니다.
const INFERENCE_SCOPE: &str = "user:inference";
const BCRYPT_INIT_AUTH_MODE_INFO_VERSION: u32 = 1;

pub(super) struct DesktopToken {
    pub(super) access_token: String,
    pub(super) expires_at: Option<i64>,
}

pub(super) fn config_path() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("Claude").join("config.json"))
}

fn local_state_path(config_path: &Path) -> PathBuf {
    config_path.with_file_name("Local State")
}

pub(super) fn read_token(config_path: &Path) -> Option<DesktopToken> {
    let config = std::fs::read_to_string(config_path).ok()?;

    let caches = token_cache_values(&config);
    if caches.is_empty() {
        return None;
    }
    let key = os_crypt_key(&local_state_path(config_path))?;

    for (_, cache) in &caches {
        let Some(plaintext) = decrypt_os_crypt_value(cache, &key) else {
            continue;
        };
        let Ok(plaintext) = String::from_utf8(plaintext) else {
            continue;
        };
        if let Some(token) = select_token(&plaintext) {
            return Some(token);
        }
    }

    None
}

/// 파일 수정 시간(mtime) 대신 암호화된 캐시 내용 자체의 시그니처입니다.
/// 데스크톱 앱은 창 위치 등 자격 증명과 무관한 상태 변경 시에도 `config.json`을 다시 쓰므로,
/// 이를 자격 증명 변경으로 오인하지 않도록 합니다.
pub(super) fn watch_signature(config_path: &Path) -> String {
    let key = format!("desktop:{}", config_path.display());
    let caches = std::fs::read_to_string(config_path)
        .ok()
        .map(|config| token_cache_values(&config))
        .unwrap_or_default();
    if caches.is_empty() {
        return format!("{key}|missing");
    }

    let mut signature = format!("{key}|present");
    for (name, cache) in caches {
        signature.push_str(&format!("|{name}:{}", fnv1a(cache.as_bytes())));
    }
    signature
}

/// 설정 파일에 포함된 모든 토큰 캐시 (최신 레이아웃 순).
fn token_cache_values(config: &str) -> Vec<(&'static str, String)> {
    let Ok(json) = serde_json::from_str::<serde_json::Value>(config) else {
        return Vec::new();
    };
    TOKEN_CACHE_KEYS
        .iter()
        .filter_map(|key| {
            let value = json.get(*key)?.as_str()?;
            (!value.is_empty()).then(|| (*key, value.to_string()))
        })
        .collect()
}

/// 추론 스코프를 포함하는 가장 최신 항목을 선택하며, 향후 키 레이아웃 변경 시에도 동작할 수 있도록
/// 스코프와 무관하게 가장 최신 항목으로 폴백합니다.
fn select_token(plaintext: &str) -> Option<DesktopToken> {
    let json: serde_json::Value = serde_json::from_str(plaintext).ok()?;
    let entries = json.as_object()?;

    let mut best: Option<(bool, i64, DesktopToken)> = None;
    for (key, entry) in entries {
        let Some(access_token) = entry.get("token").and_then(|value| value.as_str()) else {
            continue;
        };
        if access_token.is_empty() {
            continue;
        }
        let expires_at = entry.get("expiresAt").and_then(|value| value.as_i64());
        let rank = (
            key.contains(INFERENCE_SCOPE),
            expires_at.unwrap_or(i64::MIN),
        );
        if best
            .as_ref()
            .is_some_and(|(scoped, expiry, _)| (*scoped, *expiry) >= rank)
        {
            continue;
        }
        best = Some((
            rank.0,
            rank.1,
            DesktopToken {
                access_token: access_token.to_string(),
                expires_at,
            },
        ));
    }

    best.map(|(_, _, token)| token)
}

#[cfg(windows)]
fn os_crypt_key(local_state_path: &Path) -> Option<Vec<u8>> {
    let local_state = std::fs::read_to_string(local_state_path).ok()?;
    let json: serde_json::Value = serde_json::from_str(&local_state).ok()?;
    let encoded = json.get("os_crypt")?.get("encrypted_key")?.as_str()?;
    let wrapped = base64_decode(encoded)?;
    let wrapped = wrapped.strip_prefix(DPAPI_KEY_PREFIX)?;
    dpapi_unprotect(wrapped)
}

#[cfg(not(windows))]
fn os_crypt_key(_local_state_path: &Path) -> Option<Vec<u8>> {
    None
}

#[cfg(windows)]
fn decrypt_os_crypt_value(value: &str, key: &[u8]) -> Option<Vec<u8>> {
    let blob = base64_decode(value)?;
    let body = blob.strip_prefix(OS_CRYPT_PREFIX)?;
    if body.len() < GCM_NONCE_LEN + GCM_TAG_LEN {
        return None;
    }
    let (nonce, rest) = body.split_at(GCM_NONCE_LEN);
    let (ciphertext, tag) = rest.split_at(rest.len() - GCM_TAG_LEN);
    aes_gcm_decrypt(key, nonce, ciphertext, tag)
}

#[cfg(not(windows))]
fn decrypt_os_crypt_value(_value: &str, _key: &[u8]) -> Option<Vec<u8>> {
    None
}

fn base64_decode(input: &str) -> Option<Vec<u8>> {
    let input = input.trim_end_matches('=');
    if input.len() % 4 == 1 {
        return None;
    }
    let mut output = Vec::with_capacity(input.len() * 3 / 4);
    let mut buffer = 0u32;
    let mut bits = 0u32;
    for byte in input.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        } as u32;
        buffer = (buffer << 6) | value;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            output.push(((buffer >> bits) & 0xff) as u8);
        }
    }
    let padding_mask = (1u32 << bits).saturating_sub(1);
    (buffer & padding_mask == 0).then_some(output)
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in bytes {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    hash
}

#[cfg(windows)]
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
#[repr(C)]
struct CryptIntegerBlob {
    cb_data: u32,
    pb_data: *mut u8,
}

#[cfg(windows)]
#[repr(C)]
struct AuthenticatedCipherModeInfo {
    cb_size: u32,
    dw_info_version: u32,
    pb_nonce: *mut u8,
    cb_nonce: u32,
    pb_auth_data: *mut u8,
    cb_auth_data: u32,
    pb_tag: *mut u8,
    cb_tag: u32,
    pb_mac_context: *mut u8,
    cb_mac_context: u32,
    cb_aad: u32,
    cb_data: u64,
    dw_flags: u32,
}

#[cfg(windows)]
#[link(name = "crypt32")]
extern "system" {
    fn CryptUnprotectData(
        data_in: *const CryptIntegerBlob,
        data_description: *mut *mut u16,
        optional_entropy: *const CryptIntegerBlob,
        reserved: *mut c_void,
        prompt_struct: *mut c_void,
        flags: u32,
        data_out: *mut CryptIntegerBlob,
    ) -> i32;
}

#[cfg(windows)]
extern "system" {
    fn LocalFree(mem: *mut c_void) -> *mut c_void;
}

#[cfg(windows)]
#[link(name = "bcrypt")]
extern "system" {
    fn BCryptOpenAlgorithmProvider(
        algorithm: *mut *mut c_void,
        id: *const u16,
        implementation: *const u16,
        flags: u32,
    ) -> i32;
    fn BCryptCloseAlgorithmProvider(algorithm: *mut c_void, flags: u32) -> i32;
    fn BCryptGetProperty(
        object: *mut c_void,
        property: *const u16,
        output: *mut u8,
        output_len: u32,
        result: *mut u32,
        flags: u32,
    ) -> i32;
    fn BCryptSetProperty(
        object: *mut c_void,
        property: *const u16,
        input: *const u8,
        input_len: u32,
        flags: u32,
    ) -> i32;
    fn BCryptGenerateSymmetricKey(
        algorithm: *mut c_void,
        key: *mut *mut c_void,
        key_object: *mut u8,
        key_object_len: u32,
        secret: *const u8,
        secret_len: u32,
        flags: u32,
    ) -> i32;
    fn BCryptDestroyKey(key: *mut c_void) -> i32;
    fn BCryptDecrypt(
        key: *mut c_void,
        input: *const u8,
        input_len: u32,
        padding_info: *const c_void,
        iv: *mut u8,
        iv_len: u32,
        output: *mut u8,
        output_len: u32,
        result: *mut u32,
        flags: u32,
    ) -> i32;
}

#[cfg(windows)]
fn dpapi_unprotect(data: &[u8]) -> Option<Vec<u8>> {
    let input = CryptIntegerBlob {
        cb_data: u32::try_from(data.len()).ok()?,
        pb_data: data.as_ptr() as *mut u8,
    };
    let mut output = CryptIntegerBlob {
        cb_data: 0,
        pb_data: std::ptr::null_mut(),
    };

    let ok = unsafe {
        CryptUnprotectData(
            &input,
            std::ptr::null_mut(),
            std::ptr::null(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            0,
            &mut output,
        )
    };

    if ok == 0 || output.pb_data.is_null() {
        return None;
    }

    Some(unsafe {
        let key = std::slice::from_raw_parts(output.pb_data, output.cb_data as usize).to_vec();
        LocalFree(output.pb_data as *mut c_void);
        key
    })
}

#[cfg(windows)]
fn aes_gcm_decrypt(key: &[u8], nonce: &[u8], ciphertext: &[u8], tag: &[u8]) -> Option<Vec<u8>> {
    let algorithm_id = wide("AES");
    let mut algorithm: *mut c_void = std::ptr::null_mut();
    if unsafe {
        BCryptOpenAlgorithmProvider(&mut algorithm, algorithm_id.as_ptr(), std::ptr::null(), 0)
    } != 0
    {
        return None;
    }

    let plaintext = with_gcm_key(algorithm, key, |key_handle| {
        decrypt_with_key(key_handle, nonce, ciphertext, tag)
    });

    unsafe { BCryptCloseAlgorithmProvider(algorithm, 0) };
    plaintext
}

#[cfg(windows)]
fn with_gcm_key(
    algorithm: *mut c_void,
    key: &[u8],
    decrypt: impl FnOnce(*mut c_void) -> Option<Vec<u8>>,
) -> Option<Vec<u8>> {
    let chaining_property = wide("ChainingMode");
    let chaining_gcm = wide("ChainingModeGCM");
    if unsafe {
        BCryptSetProperty(
            algorithm,
            chaining_property.as_ptr(),
            chaining_gcm.as_ptr() as *const u8,
            u32::try_from(std::mem::size_of_val(chaining_gcm.as_slice())).ok()?,
            0,
        )
    } != 0
    {
        return None;
    }

    let object_length_property = wide("ObjectLength");
    let mut object_length = 0u32;
    let mut written = 0u32;
    if unsafe {
        BCryptGetProperty(
            algorithm,
            object_length_property.as_ptr(),
            &mut object_length as *mut u32 as *mut u8,
            u32::try_from(std::mem::size_of::<u32>()).ok()?,
            &mut written,
            0,
        )
    } != 0
    {
        return None;
    }

    // 키 객체 버퍼는 해당 버퍼를 사용하는 키 핸들보다 오래 유지되어야 합니다.
    let mut key_object = vec![0u8; object_length as usize];
    let mut key_handle: *mut c_void = std::ptr::null_mut();
    if unsafe {
        BCryptGenerateSymmetricKey(
            algorithm,
            &mut key_handle,
            key_object.as_mut_ptr(),
            object_length,
            key.as_ptr(),
            u32::try_from(key.len()).ok()?,
            0,
        )
    } != 0
    {
        return None;
    }

    let plaintext = decrypt(key_handle);
    unsafe { BCryptDestroyKey(key_handle) };
    drop(key_object);
    plaintext
}

#[cfg(windows)]
fn decrypt_with_key(
    key_handle: *mut c_void,
    nonce: &[u8],
    ciphertext: &[u8],
    tag: &[u8],
) -> Option<Vec<u8>> {
    let mut nonce = nonce.to_vec();
    let mut tag = tag.to_vec();
    let mode_info = AuthenticatedCipherModeInfo {
        cb_size: u32::try_from(std::mem::size_of::<AuthenticatedCipherModeInfo>()).ok()?,
        dw_info_version: BCRYPT_INIT_AUTH_MODE_INFO_VERSION,
        pb_nonce: nonce.as_mut_ptr(),
        cb_nonce: u32::try_from(nonce.len()).ok()?,
        pb_auth_data: std::ptr::null_mut(),
        cb_auth_data: 0,
        pb_tag: tag.as_mut_ptr(),
        cb_tag: u32::try_from(tag.len()).ok()?,
        pb_mac_context: std::ptr::null_mut(),
        cb_mac_context: 0,
        cb_aad: 0,
        cb_data: 0,
        dw_flags: 0,
    };

    let mut plaintext = vec![0u8; ciphertext.len()];
    let mut written = 0u32;
    let status = unsafe {
        BCryptDecrypt(
            key_handle,
            ciphertext.as_ptr(),
            u32::try_from(ciphertext.len()).ok()?,
            &mode_info as *const AuthenticatedCipherModeInfo as *const c_void,
            std::ptr::null_mut(),
            0,
            plaintext.as_mut_ptr(),
            u32::try_from(plaintext.len()).ok()?,
            &mut written,
            0,
        )
    };

    if status != 0 {
        return None;
    }

    plaintext.truncate(written as usize);
    Some(plaintext)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_the_freshest_inference_scoped_token() {
        let plaintext = r#"{
            "install:user:https://api.anthropic.com:user:profile": {
                "token": "profile-only",
                "expiresAt": 9000000000000
            },
            "install:user:https://api.anthropic.com:user:inference user:profile": {
                "token": "current",
                "expiresAt": 1818644595762
            },
            "install:old:https://api.anthropic.com:user:inference": {
                "token": "stale",
                "expiresAt": 1518644595762
            }
        }"#;

        let token = select_token(plaintext).expect("an inference token should be selected");
        assert_eq!(token.access_token, "current");
        assert_eq!(token.expires_at, Some(1818644595762));
    }

    #[test]
    fn ignores_entries_without_a_usable_token() {
        assert!(select_token(r#"{"install:user:scope": {"expiresAt": 1}}"#).is_none());
        assert!(select_token(r#"{"install:user:scope": {"token": ""}}"#).is_none());
        assert!(select_token("not json").is_none());
    }

    #[test]
    fn reads_the_token_cache_out_of_a_desktop_config() {
        let config = r#"{"locale": "en-US", "oauth:tokenCache": "djEwYWJj"}"#;
        assert_eq!(
            token_cache_values(config),
            vec![("oauth:tokenCache", "djEwYWJj".to_string())]
        );
        assert!(token_cache_values(r#"{"locale": "en-US"}"#).is_empty());
        assert!(token_cache_values("not json").is_empty());
    }

    #[test]
    fn prefers_the_v2_cache_but_keeps_the_legacy_one_as_a_fallback() {
        let config = r#"{"oauth:tokenCache": "djEwb2xk", "oauth:tokenCacheV2": "djEwbmV3"}"#;
        assert_eq!(
            token_cache_values(config),
            vec![
                ("oauth:tokenCacheV2", "djEwbmV3".to_string()),
                ("oauth:tokenCache", "djEwb2xk".to_string()),
            ]
        );
    }

    #[test]
    fn ignores_emptied_token_caches() {
        // 데스크톱 앱은 마이그레이션 후 기존 키를 빈 값으로 남겨두기도 합니다.
        // 이것이 다른 키에 저장된 유효한 캐시를 가려서는 안 됩니다.
        let config = r#"{"oauth:tokenCacheV2": "", "oauth:tokenCache": "djEwb2xk"}"#;
        assert_eq!(
            token_cache_values(config),
            vec![("oauth:tokenCache", "djEwb2xk".to_string())]
        );
    }

    #[test]
    fn rejects_blobs_that_are_not_os_crypt_v10() {
        // 유효한 base64이지만 버전 접두사가 "v10"이 아닙니다.
        assert!(decrypt_os_crypt_value("bm90LXYxMC1kYXRh", &[0u8; 32]).is_none());
        // 올바른 접두사이나 nonce와 tag를 담기에는 길이가 너무 짧습니다.
        assert!(decrypt_os_crypt_value("djEwc2hvcnQ", &[0u8; 32]).is_none());
        assert!(decrypt_os_crypt_value("!!!", &[0u8; 32]).is_none());
    }

    #[test]
    fn decodes_standard_base64_with_and_without_padding() {
        assert_eq!(base64_decode("djEw").unwrap(), b"v10");
        assert_eq!(base64_decode("YWJjZA==").unwrap(), b"abcd");
        assert!(base64_decode("a").is_none());
        assert!(base64_decode("a-b_").is_none());
    }

    /// 기본적으로 무시됨(ignored): 현재 기기의 Claude 데스크톱 앱 데이터를 대상으로
    /// 실제 DPAPI + AES-GCM 복호화 경로를 검증합니다. 앱에 로그인된 상태에서 `cargo test -- --ignored`로 실행하세요.
    #[test]
    #[ignore = "requires a signed-in Claude desktop app on this machine"]
    fn reads_a_token_from_the_installed_desktop_app() {
        let path = config_path().expect("a roaming config directory");
        let token = read_token(&path).expect("the desktop app should expose a token");
        assert!(token.access_token.starts_with("sk-ant-"));
        assert!(token.expires_at.unwrap_or_default() > 0);
    }

    #[test]
    fn watch_signature_reports_missing_config() {
        let signature = watch_signature(Path::new("C:/nonexistent/Claude/config.json"));
        assert!(signature.ends_with("|missing"), "{signature}");
    }
}
