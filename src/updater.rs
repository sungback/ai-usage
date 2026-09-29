#![cfg_attr(not(windows), allow(dead_code))]

use std::fs::File;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

#[cfg(windows)]
use std::os::windows::process::CommandExt;
#[cfg(windows)]
use windows::core::PCWSTR;
#[cfg(windows)]
use windows::Win32::Foundation::{HWND, WAIT_OBJECT_0, WAIT_TIMEOUT};
#[cfg(windows)]
use windows::Win32::System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE};
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};

use serde::Deserialize;

const GITHUB_API_ACCEPT: &str = "application/vnd.github+json";
const GITHUB_API_VERSION: &str = "2022-11-28";
#[cfg(windows)]
const RELEASE_ASSET_NAME: &str = "ai-usage.exe";
#[cfg(not(windows))]
const RELEASE_ASSET_NAME: &str = "ai-usage";
#[cfg(windows)]
const HELPER_EXE_NAME: &str = "updater-helper.exe";
#[cfg(not(windows))]
const HELPER_EXE_NAME: &str = "updater-helper";
#[cfg(windows)]
const DOWNLOAD_EXE_NAME: &str = "update-download.exe";
#[cfg(not(windows))]
const DOWNLOAD_EXE_NAME: &str = "update-download";

/// How often background update checks run. Windows arms `TIMER_UPDATE_CHECK`
/// with it; macOS sleeps it between checks.
pub const AUTO_UPDATE_CHECK_INTERVAL_SECS: u64 = 24 * 60 * 60;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

trait CommandExtHelper {
    fn no_window(&mut self) -> &mut Self;
}

impl CommandExtHelper for Command {
    fn no_window(&mut self) -> &mut Self {
        #[cfg(windows)]
        {
            self.creation_flags(CREATE_NO_WINDOW);
        }
        self
    }
}

#[derive(Clone, Debug)]
pub struct ReleaseDescriptor {
    pub latest_version: String,
    asset_url: String,
    /// The sibling `<asset>.sha256` release asset. Both release workflows
    /// publish one, so its absence means the release is malformed and the
    /// download is refused rather than installed unverified.
    checksum_url: Option<String>,
}

#[derive(Debug)]
pub enum UpdateCheckResult {
    UpToDate,
    Available(ReleaseDescriptor),
}

#[derive(Deserialize)]
struct GitHubRelease {
    tag_name: String,
    assets: Vec<GitHubAsset>,
}

#[derive(Deserialize)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
}

pub fn handle_cli_mode(args: &[String]) -> Option<i32> {
    if args.len() == 5 && args[1] == "--apply-update" {
        let target = PathBuf::from(&args[2]);
        let source = PathBuf::from(&args[3]);
        let pid = args[4].parse::<u32>().unwrap_or(0);

        return Some(match apply_update(target, source, pid) {
            Ok(()) => 0,
            Err(error) => {
                show_error_message("Update failed", &error);
                1
            }
        });
    }

    None
}

pub fn check_for_updates() -> Result<UpdateCheckResult, String> {
    match fetch_latest_release()? {
        Some(release) => Ok(UpdateCheckResult::Available(release)),
        None => Ok(UpdateCheckResult::UpToDate),
    }
}

pub fn begin_self_update(release: &ReleaseDescriptor) -> Result<(), String> {
    let current_exe =
        std::env::current_exe().map_err(|e| format!("Unable to locate current executable: {e}"))?;
    ensure_target_location_writable(&current_exe)?;

    let stage_dir = updates_dir()?;
    std::fs::create_dir_all(&stage_dir)
        .map_err(|e| format!("Unable to create updater working directory: {e}"))?;

    let helper_path = stage_dir.join(HELPER_EXE_NAME);
    let download_path = stage_dir.join(DOWNLOAD_EXE_NAME);
    let partial_download_path = stage_dir.join(format!("{DOWNLOAD_EXE_NAME}.part"));

    if helper_path.exists() {
        let _ = std::fs::remove_file(&helper_path);
    }
    if download_path.exists() {
        let _ = std::fs::remove_file(&download_path);
    }
    if partial_download_path.exists() {
        let _ = std::fs::remove_file(&partial_download_path);
    }

    download_release_asset(
        &release.asset_url,
        release.checksum_url.as_deref(),
        &partial_download_path,
        &download_path,
    )?;
    std::fs::copy(&current_exe, &helper_path)
        .map_err(|e| format!("Unable to prepare updater helper: {e}"))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(metadata) = std::fs::metadata(&helper_path) {
            let mut perms = metadata.permissions();
            perms.set_mode(0o755);
            let _ = std::fs::set_permissions(&helper_path, perms);
        }
    }

    let pid = std::process::id().to_string();
    let target = current_exe.to_string_lossy().to_string();
    let source = download_path.to_string_lossy().to_string();

    Command::new(&helper_path)
        .arg("--apply-update")
        .arg(target)
        .arg(source)
        .arg(pid)
        .no_window()
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("Unable to launch updater helper: {e}"))?;

    Ok(())
}

/// Whether a zip entry looks like the executable to install: `*.exe` on
/// Windows, a file named `ai-usage` elsewhere (e.g. the
/// `Contents/MacOS/ai-usage` inside the macOS app bundle; suffixed renames
/// like `ai-usage-aarch64` also match). Anything else in the archive —
/// READMEs, checksum sidecars, disk images — is never installable.
fn is_installable_zip_entry(name: &str) -> bool {
    if name.ends_with('/') {
        return false;
    }
    #[cfg(windows)]
    {
        // Keep the historical case-sensitive match: release assets are
        // lowercase `ai-usage.exe` and nothing else ends in `.exe`.
        name.ends_with(".exe")
    }
    #[cfg(not(windows))]
    {
        let lower = name.to_ascii_lowercase();
        if lower.ends_with(".sha256") || lower.ends_with(".dmg") {
            return false;
        }
        let file_name = lower.rsplit('/').next().unwrap_or(&lower);
        file_name == "ai-usage" || file_name.starts_with("ai-usage-")
    }
}

fn unpack_zip_if_needed(source: &Path, stage_dir: &Path) -> Result<PathBuf, String> {
    let mut file = match File::open(source) {
        Ok(f) => f,
        Err(e) => return Err(format!("Failed to open downloaded file {}: {e}", source.display())),
    };
    use std::io::{Read, Seek, SeekFrom};
    let mut magic = [0u8; 4];
    if file.read_exact(&mut magic).is_ok() && &magic == b"PK\x03\x04" {
        file.seek(SeekFrom::Start(0))
            .map_err(|e| format!("Seek failed in zip archive: {e}"))?;
        let mut archive =
            zip::ZipArchive::new(file).map_err(|e| format!("Invalid zip archive: {e}"))?;

        let mut exe_index = None;
        for i in 0..archive.len() {
            let item = archive.by_index(i).map_err(|e| format!("Corrupt zip entry: {e}"))?;
            if is_installable_zip_entry(item.name()) {
                exe_index = Some(i);
                break;
            }
        }

        let index = exe_index
            .ok_or_else(|| "Could not find executable binary in update archive".to_string())?;
        let mut item = archive
            .by_index(index)
            .map_err(|e| format!("Zip entry read failed: {e}"))?;

        let extracted_path = stage_dir.join(if cfg!(windows) {
            "unpacked-binary.exe"
        } else {
            "unpacked-binary"
        });
        let mut dest = File::create(&extracted_path)
            .map_err(|e| format!("Failed to create extracted file: {e}"))?;
        io::copy(&mut item, &mut dest)
            .map_err(|e| format!("Failed to extract update binary: {e}"))?;
        dest.flush()
            .map_err(|e| format!("Failed to finalize extracted file: {e}"))?;

        return Ok(extracted_path);
    }

    Ok(source.to_path_buf())
}

fn apply_update(target: PathBuf, source: PathBuf, pid: u32) -> Result<(), String> {
    if !source.exists() {
        return Err(format!(
            "Downloaded update not found at {}",
            source.display()
        ));
    }

    let _ = wait_for_process_exit(pid, Duration::from_secs(30));

    let stage_dir = source
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let binary_source = unpack_zip_if_needed(&source, &stage_dir)?;

    replace_target_binary(&target, &binary_source)?;
    relaunch_target(&target)?;
    let _ = std::fs::remove_file(&source);
    if binary_source != source {
        let _ = std::fs::remove_file(&binary_source);
    }

    Ok(())
}

fn fetch_latest_release() -> Result<Option<ReleaseDescriptor>, String> {
    let (owner, repo) = github_repo()?;
    let url = format!("https://api.github.com/repos/{owner}/{repo}/releases/latest");
    let agent = build_agent()?;

    let mut response = agent
        .get(&url)
        .header("Accept", GITHUB_API_ACCEPT)
        .header("User-Agent", user_agent())
        .header("X-GitHub-Api-Version", GITHUB_API_VERSION)
        .call()
        .map_err(|e| format!("Unable to check GitHub releases: {e}"))?;

    let release: GitHubRelease = response
        .body_mut()
        .read_json()
        .map_err(|e| format!("Unable to parse GitHub release data: {e}"))?;

    let latest_version = release.tag_name.trim_start_matches('v').to_string();
    if !is_version_newer(&latest_version, env!("CARGO_PKG_VERSION")) {
        return Ok(None);
    }

    let asset = release
        .assets
        .iter()
        .find(|asset| asset.name.eq_ignore_ascii_case(RELEASE_ASSET_NAME))
        .or_else(|| {
            #[cfg(windows)]
            {
                release
                    .assets
                    .iter()
                    .find(|asset| asset.name.to_ascii_lowercase().ends_with(".exe"))
            }
            #[cfg(target_os = "macos")]
            {
                let arch = if cfg!(target_arch = "aarch64") { "arm" } else { "x86" };
                release
                    .assets
                    .iter()
                    .find(|asset| {
                        let name = asset.name.to_ascii_lowercase();
                        is_installable_release_asset(&name)
                            && (name.contains("macos") || name.contains("darwin"))
                            && (name.contains(arch) || name.contains("aarch64"))
                    })
                    .or_else(|| {
                        release.assets.iter().find(|asset| {
                            let name = asset.name.to_ascii_lowercase();
                            is_installable_release_asset(&name)
                                && (name.contains("macos") || name.contains("darwin"))
                                && name.contains("universal")
                        })
                    })
                    .or_else(|| {
                        release.assets.iter().find(|asset| {
                            let name = asset.name.to_ascii_lowercase();
                            is_installable_release_asset(&name)
                                && (name.contains("macos") || name.contains("darwin"))
                        })
                    })
                    .or_else(|| {
                        release.assets.iter().find(|asset| {
                            let name = asset.name.to_ascii_lowercase();
                            is_installable_release_asset(&name)
                                && !name.ends_with(".exe")
                                && (name.ends_with(".dmg")
                                    || name.ends_with(".zip")
                                    || name.starts_with("ai-usage"))
                        })
                    })
            }
            #[cfg(all(not(windows), not(target_os = "macos")))]
            {
                release
                    .assets
                    .iter()
                    .find(|asset| !asset.name.ends_with(".exe") && !asset.name.ends_with(".zip") && !asset.name.ends_with(".sha256"))
            }
        })
        .ok_or_else(|| {
            "No compatible executable asset was found in the latest release.".to_string()
        })?;

    let checksum_name = format!("{}.sha256", asset.name).to_ascii_lowercase();
    let checksum_url = release
        .assets
        .iter()
        .find(|candidate| candidate.name.to_ascii_lowercase() == checksum_name)
        .map(|candidate| candidate.browser_download_url.clone());

    Ok(Some(ReleaseDescriptor {
        latest_version,
        asset_url: asset.browser_download_url.clone(),
        checksum_url,
    }))
}

/// Release assets that can actually be installed. Checksum sidecars share the
/// binary's name stem (`ai-usage-macos-arm64.zip.sha256`), so name matching
/// must exclude them or an update would download a digest as its payload.
#[cfg(target_os = "macos")]
fn is_installable_release_asset(lower_name: &str) -> bool {
    !lower_name.ends_with(".sha256")
}

fn build_agent() -> Result<ureq::Agent, String> {
    let tls = ureq::tls::TlsConfig::builder()
        .provider(ureq::tls::TlsProvider::NativeTls)
        .root_certs(ureq::tls::RootCerts::PlatformVerifier)
        .build();
    Ok(ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(30)))
        .tls_config(tls)
        .build()
        .into())
}

fn download_release_asset(
    url: &str,
    checksum_url: Option<&str>,
    partial_path: &Path,
    final_path: &Path,
) -> Result<(), String> {
    // Refuse before spending any bandwidth: an unverifiable payload is never
    // going to be installed, so downloading it first only risks filling the disk.
    let Some(checksum_url) = checksum_url else {
        return Err(
            "This release does not publish a checksum, so the download cannot be verified. \
             Update refused; install the new version manually instead."
                .into(),
        );
    };

    let agent = build_agent()?;
    let response = agent
        .get(url)
        .header("User-Agent", user_agent())
        .call()
        .map_err(|e| format!("Unable to download the latest release: {e}"))?;

    let mut reader = response.into_body().into_reader();
    let mut file = File::create(partial_path)
        .map_err(|e| format!("Unable to create temporary download file: {e}"))?;

    io::copy(&mut reader, &mut file)
        .map_err(|e| format!("Unable to write the downloaded update: {e}"))?;
    file.flush()
        .map_err(|e| format!("Unable to finalize the downloaded update: {e}"))?;
    drop(file);

    verify_checksum(&agent, checksum_url, partial_path)?;

    std::fs::rename(partial_path, final_path)
        .map_err(|e| format!("Unable to finalize the downloaded update file: {e}"))?;

    Ok(())
}

/// Guards against a truncated or corrupted download, not a compromised
/// release: the checksum comes from the same GitHub release as the binary
/// itself, so this cannot catch a release that was tampered with at the
/// source. A release that ships no checksum is refused by the caller rather
/// than installed unverified.
fn verify_checksum(agent: &ureq::Agent, checksum_url: &str, downloaded: &Path) -> Result<(), String> {
    let mut response = agent
        .get(checksum_url)
        .header("User-Agent", user_agent())
        .call()
        .map_err(|e| format!("Unable to download the release checksum: {e}"))?;
    let body = response
        .body_mut()
        .read_to_string()
        .map_err(|e| format!("Unable to read the release checksum: {e}"))?;
    let expected = parse_sha256_digest(&body)
        .ok_or_else(|| "The published release checksum is malformed.".to_string())?;

    let mut file = File::open(downloaded)
        .map_err(|e| format!("Unable to reopen the downloaded update for verification: {e}"))?;
    let actual = sha256_hex(&mut file)
        .map_err(|e| format!("Unable to hash the downloaded update: {e}"))?;

    if actual != expected {
        let _ = std::fs::remove_file(downloaded);
        return Err(format!(
            "Downloaded update failed checksum verification (expected {expected}, got {actual}). The download may be corrupted or incomplete; try again."
        ));
    }

    Ok(())
}

/// A `.sha256` asset is either a bare hex digest or the standard
/// `sha256sum`/`shasum` line format (`"<hex>  <filename>"`); either way, the
/// digest is the first whitespace-separated token.
fn parse_sha256_digest(body: &str) -> Option<String> {
    let token = body.split_whitespace().next()?;
    (token.len() == 64 && token.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| token.to_ascii_lowercase())
}

fn sha256_hex(reader: &mut impl Read) -> io::Result<String> {
    let mut hasher = <sha2::Sha256 as sha2::Digest>::new();
    io::copy(reader, &mut hasher)?;
    Ok(format!("{:x}", sha2::Digest::finalize(hasher)))
}

fn replace_target_binary(target: &Path, source: &Path) -> Result<(), String> {
    let backup_path = backup_path_for(target);
    let mut last_error = None;

    for _ in 0..60 {
        let _ = std::fs::remove_file(&backup_path);

        let renamed_existing = match std::fs::rename(target, &backup_path) {
            Ok(()) => true,
            Err(error) if error.kind() == io::ErrorKind::NotFound => false,
            Err(error) => {
                last_error = Some(error);
                std::thread::sleep(Duration::from_millis(500));
                continue;
            }
        };

        match std::fs::copy(source, target) {
            Ok(_) => {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if let Ok(metadata) = std::fs::metadata(target) {
                        let mut perms = metadata.permissions();
                        perms.set_mode(0o755);
                        let _ = std::fs::set_permissions(target, perms);
                    }
                }
                let _ = std::fs::remove_file(&backup_path);
                return Ok(());
            }
            Err(error) => {
                last_error = Some(error);
                let _ = std::fs::remove_file(target);
                if renamed_existing {
                    let _ = std::fs::rename(&backup_path, target);
                }
            }
        }

        std::thread::sleep(Duration::from_millis(500));
    }

    Err(format!(
        "Unable to replace {}. {}",
        target.display(),
        last_error
            .map(|error| error.to_string())
            .unwrap_or_else(|| {
                "The file may still be locked or the install directory may not be writable."
                    .to_string()
            })
    ))
}

fn relaunch_target(target: &Path) -> Result<(), String> {
    let mut command = Command::new(target);
    if let Some(parent) = target.parent() {
        command.current_dir(parent);
    }

    command
        .no_window()
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| {
            format!(
                "The update was installed, but the app could not be restarted automatically: {e}"
            )
        })?;

    Ok(())
}

#[cfg(windows)]
fn wait_for_process_exit(pid: u32, timeout: Duration) -> Result<(), String> {
    if pid == 0 {
        return Ok(());
    }

    unsafe {
        let handle = OpenProcess(PROCESS_SYNCHRONIZE, false, pid)
            .map_err(|e| format!("Unable to monitor the running app process: {e}"))?;

        let result = WaitForSingleObject(handle, timeout.as_millis().min(u32::MAX as u128) as u32);
        let _ = windows::Win32::Foundation::CloseHandle(handle);

        if result == WAIT_OBJECT_0 {
            Ok(())
        } else if result == WAIT_TIMEOUT {
            Err("Timed out waiting for the running app to exit.".to_string())
        } else {
            Err("Unable to confirm that the running app has exited.".to_string())
        }
    }
}

#[cfg(not(windows))]
fn wait_for_process_exit(pid: u32, timeout: Duration) -> Result<(), String> {
    if pid == 0 {
        return Ok(());
    }
    let start = std::time::Instant::now();
    while start.elapsed() < timeout {
        let running = Command::new("kill")
            .arg("-0")
            .arg(pid.to_string())
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !running {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    Err("Timed out waiting for the running app to exit.".to_string())
}

fn updates_dir() -> Result<PathBuf, String> {
    dirs::data_local_dir()
        .map(|dir| dir.join("ai-usage").join("updates"))
        .or_else(|| {
            Some(
                std::env::temp_dir()
                    .join("ai-usage")
                    .join("updates"),
            )
        })
        .ok_or_else(|| "Unable to resolve a writable local updates directory.".to_string())
}


fn backup_path_for(target: &Path) -> PathBuf {
    let file_name = target
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("app.exe");
    target.with_file_name(format!("{file_name}.old"))
}

fn ensure_target_location_writable(target: &Path) -> Result<(), String> {
    let parent = target.parent().ok_or_else(|| {
        "Unable to determine the install directory for the current executable.".to_string()
    })?;

    let probe_path = parent.join(".__ccum_update_probe");
    match File::create(&probe_path) {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe_path);
            Ok(())
        }
        Err(error) => Err(format!(
            "The current install location is not writable. Move the app to a user-writable folder or install it somewhere outside Program Files. {error}"
        )),
    }
}

fn github_repo() -> Result<(&'static str, &'static str), String> {
    let repository = env!("CARGO_PKG_REPOSITORY").trim_end_matches('/');
    let parts: Vec<&str> = repository.split('/').collect();
    if parts.len() < 2 {
        return Err("Package repository URL is not configured for GitHub releases.".to_string());
    }

    let owner = parts[parts.len() - 2];
    let repo = parts[parts.len() - 1];
    if owner.is_empty() || repo.is_empty() {
        return Err("Package repository URL is not configured for GitHub releases.".to_string());
    }

    Ok((owner, repo))
}

fn user_agent() -> &'static str {
    concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"))
}


fn is_version_newer(candidate: &str, current: &str) -> bool {
    parse_version(candidate) > parse_version(current)
}

fn parse_version(version: &str) -> (u32, u32, u32) {
    let core = version.split('-').next().unwrap_or(version);
    let mut parts = core.split('.').map(|part| part.parse::<u32>().unwrap_or(0));

    (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    )
}

#[cfg(windows)]
fn show_error_message(title: &str, message: &str) {
    unsafe {
        let title_wide = wide_str(title);
        let message_wide = wide_str(message);
        let _ = MessageBoxW(
            Some(HWND::default()),
            PCWSTR::from_raw(message_wide.as_ptr()),
            PCWSTR::from_raw(title_wide.as_ptr()),
            MB_OK | MB_ICONERROR,
        );
    }
}

#[cfg(not(windows))]
fn show_error_message(title: &str, message: &str) {
    eprintln!("{title}: {message}");
    let script = format!(
        "display alert \"{}\" message \"{}\" as critical",
        title.replace('"', "\\\""),
        message.replace('"', "\\\"")
    );
    let _ = Command::new("osascript").arg("-e").arg(script).spawn();
}

#[cfg(windows)]
fn wide_str(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksum_digest_accepts_bare_hex_and_sha256sum_format() {
        let digest = "a".repeat(64);
        assert_eq!(parse_sha256_digest(&digest), Some(digest.clone()));
        assert_eq!(
            parse_sha256_digest(&format!("{digest}  ai-usage.exe\n")),
            Some(digest.clone())
        );
        assert_eq!(
            parse_sha256_digest(&digest.to_ascii_uppercase()),
            Some(digest)
        );
    }

    #[test]
    fn checksum_digest_rejects_malformed_input() {
        assert_eq!(parse_sha256_digest(""), None);
        assert_eq!(parse_sha256_digest("not-hex"), None);
        assert_eq!(parse_sha256_digest(&"a".repeat(63)), None);
        assert_eq!(parse_sha256_digest(&"a".repeat(65)), None);
        // A stray HTML error page (e.g. a 404 that still returned status 200
        // from a CDN) must not be mistaken for a digest.
        assert_eq!(parse_sha256_digest("<html>Not Found</html>"), None);
    }

    #[test]
    fn sha256_hex_matches_known_vectors() {
        assert_eq!(
            sha256_hex(&mut "".as_bytes()).unwrap(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(&mut "abc".as_bytes()).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn installable_zip_entries_reject_directories_readmes_and_sidecars() {
        assert!(is_installable_zip_entry(
            "AI Usage Monitor.app/Contents/MacOS/ai-usage"
        ));
        assert!(is_installable_zip_entry("ai-usage"));
        assert!(is_installable_zip_entry("ai-usage-aarch64"));
        assert!(!is_installable_zip_entry("AI Usage Monitor.app/"));
        assert!(!is_installable_zip_entry("README.md"));
        assert!(!is_installable_zip_entry("ai-usage.sha256"));
        assert!(!is_installable_zip_entry(
            "AI Usage Monitor.app/Contents/MacOS/ai-usage.sha256"
        ));
    }

    #[cfg(windows)]
    #[test]
    fn installable_zip_entries_accept_only_executables() {
        assert!(is_installable_zip_entry("ai-usage.exe"));
        assert!(!is_installable_zip_entry("README.md"));
        assert!(!is_installable_zip_entry("ai-usage.exe.sha256"));
    }

    #[test]
    fn newer_versions_win_and_prerelease_suffixes_are_ignored() {
        assert!(is_version_newer("2.12.42", "2.12.41"));
        assert!(is_version_newer("2.13.0", "2.12.41"));
        assert!(!is_version_newer("2.12.41", "2.12.41"));
        assert!(!is_version_newer("2.12.40", "2.12.41"));
        assert!(!is_version_newer("2.12.41-beta", "2.12.41"));
    }

    #[test]
    fn test_github_repo_matches_sungback_ai_usage() {
        let (owner, repo) = github_repo().expect("github_repo should parse successfully");
        assert_eq!(owner, "sungback");
        assert_eq!(repo, "ai-usage");
    }

    #[test]
    fn a_release_without_a_checksum_is_refused_before_anything_is_downloaded() {
        let root = std::env::temp_dir().join(format!("updater_no_checksum_{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let partial = root.join("download.part");
        let final_path = root.join("download.bin");

        // Port 1 is never listening, so reaching the network would surface a
        // connection error instead of the refusal below.
        let error = download_release_asset("http://127.0.0.1:1/never-fetched", None, &partial, &final_path)
            .expect_err("an unverifiable payload must never be installed");

        assert!(
            error.contains("does not publish a checksum"),
            "unexpected error: {error}"
        );
        assert!(!partial.exists(), "nothing may be downloaded when it cannot be verified");
        assert!(!final_path.exists());

        std::fs::remove_dir_all(&root).unwrap();
    }
}
