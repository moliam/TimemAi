//! In-app self upgrade: download the prebuilt release archive for this
//! platform, verify its sha256 sidecar, stage and validate the new binary,
//! then replace the running executable. MEM workspaces, sessions,
//! credentials, and user configuration are never touched.
//!
//! Flow: check -> apply (download -> verify -> stage -> validate -> replace)
//! -> pending_restart. The restart spawns the replaced executable and
//! gracefully stops the old process (see the restart wiring in `server::run`).

use serde::Serialize;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const DEFAULT_REPOSITORY: &str = "moliam/TimemAi";
const CHECK_TIMEOUT: Duration = Duration::from_secs(20);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(600);
/// 256 MiB hard cap for release archives. A release binary is tens of MiB.
const MAX_ARCHIVE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_SIDECAR_BYTES: u64 = 4 * 1024;
const STAGED_VERIFY_TIMEOUT: Duration = Duration::from_secs(30);
const DOWNLOAD_PROGRESS_STEP_PERCENT: u64 = 1;
const BACKUP_RETENTION: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "phase")]
pub(crate) enum UpgradePhase {
    Idle,
    Checking,
    NoUpdate {
        current_version: String,
    },
    UpdateAvailable {
        current_version: String,
        remote_version: String,
    },
    Downloading {
        remote_version: String,
        percent: u8,
    },
    Verifying {
        remote_version: String,
    },
    Staging {
        remote_version: String,
    },
    Applying {
        remote_version: String,
    },
    PendingRestart {
        current_version: String,
        pending_version: String,
    },
    Failed {
        error: String,
    },
}

impl UpgradePhase {
    fn is_busy(&self) -> bool {
        matches!(
            self,
            Self::Checking
                | Self::Downloading { .. }
                | Self::Verifying { .. }
                | Self::Staging { .. }
                | Self::Applying { .. }
                | Self::PendingRestart { .. }
        )
    }
}

/// Shared upgrade runtime state plus the captured executable path. The
/// executable path is captured at process start because `current_exe`
/// becomes unreliable after the running file is renamed or replaced.
#[derive(Debug)]
pub(crate) struct UpgradeRuntime {
    shared: Mutex<UpgradeShared>,
    pub(crate) executable_path: Option<PathBuf>,
}

#[derive(Debug)]
struct UpgradeShared {
    phase: UpgradePhase,
}

impl Default for UpgradeShared {
    fn default() -> Self {
        Self {
            phase: UpgradePhase::Idle,
        }
    }
}

pub(crate) type SharedUpgradeRuntime = Arc<UpgradeRuntime>;

pub(crate) fn new_upgrade_runtime(executable_path: Option<PathBuf>) -> SharedUpgradeRuntime {
    Arc::new(UpgradeRuntime {
        shared: Mutex::new(UpgradeShared {
            phase: UpgradePhase::Idle,
        }),
        executable_path,
    })
}

fn lock_shared(runtime: &UpgradeRuntime) -> std::sync::MutexGuard<'_, UpgradeShared> {
    runtime
        .shared
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub(crate) fn snapshot_phase(runtime: &UpgradeRuntime) -> UpgradePhase {
    lock_shared(runtime).phase.clone()
}

pub(crate) fn set_phase(runtime: &UpgradeRuntime, phase: UpgradePhase) {
    lock_shared(runtime).phase = phase;
}

/// Claims the busy transition if the current phase is idle-ish. Returns the
/// phase to install before the background work starts, or `None` when an
/// operation is already running.
pub(crate) fn try_begin(runtime: &UpgradeRuntime, busy: UpgradePhase) -> Option<UpgradePhase> {
    let mut shared = lock_shared(runtime);
    if shared.phase.is_busy() {
        return None;
    }
    shared.phase = busy.clone();
    Some(busy)
}

pub(crate) fn current_workspace_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

pub(crate) fn upgrade_repository() -> String {
    std::env::var("TIMEM_UPGRADE_REPOSITORY")
        .ok()
        .filter(|value| valid_repository(value))
        .unwrap_or_else(|| DEFAULT_REPOSITORY.to_string())
}

pub(crate) fn valid_repository(value: &str) -> bool {
    !value.is_empty()
        && !value.contains("//")
        && !value.starts_with('/')
        && !value.ends_with('/')
        && value.split('/').count() == 2
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/'))
}

pub(crate) fn upgrade_base_url() -> String {
    std::env::var("TIMEM_UPGRADE_BASE_URL")
        .ok()
        .map(|custom| custom.trim().trim_end_matches('/').to_string())
        .filter(|custom| !custom.is_empty())
        .unwrap_or_else(|| format!("https://github.com/{}", upgrade_repository()))
}

pub(crate) fn platform_target() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => "x86_64-pc-windows-msvc",
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
        _ => "unsupported-target",
    }
}

pub(crate) fn staged_binary_name() -> &'static str {
    if cfg!(windows) {
        "timem.exe"
    } else {
        "timem"
    }
}

pub(crate) fn asset_names(release_tag: &str) -> (String, String) {
    let archive = format!("timem-{release_tag}-{}.tar.gz", platform_target());
    let sidecar = format!("{archive}.sha256");
    (archive, sidecar)
}

pub(crate) fn asset_download_url(base_url: &str, release_tag: &str, asset: &str) -> String {
    format!("{base_url}/releases/download/{release_tag}/{asset}")
}

pub(crate) fn latest_redirect_url(base_url: &str) -> String {
    format!("{base_url}/releases/latest")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Version3 {
    major: u64,
    minor: u64,
    patch: u64,
}

pub(crate) fn parse_release_version(tag: &str) -> Option<Version3> {
    let trimmed = tag.strip_prefix('v').unwrap_or(tag);
    let mut parts = trimmed.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next().unwrap_or("0").parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some(Version3 {
        major,
        minor,
        patch,
    })
}

pub(crate) fn remote_is_newer(remote: &str, current: &str) -> bool {
    match (
        parse_release_version(remote),
        parse_release_version(current),
    ) {
        (Some(remote), Some(current)) => remote > current,
        _ => false,
    }
}

fn http_client(timeout: Duration) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::limited(10))
        .user_agent(format!(
            "timem-self-upgrade/{}",
            current_workspace_version()
        ))
        .build()
        .map_err(|error| format!("http_client_build_failed:{error}"))
}

/// Resolves the latest release tag by following the `/releases/latest`
/// redirect, exactly like the shell upgrade script.
pub(crate) async fn resolve_latest_tag(base_url: &str) -> Result<String, String> {
    let client = http_client(CHECK_TIMEOUT)?;
    let response = client
        .get(latest_redirect_url(base_url))
        .send()
        .await
        .map_err(|error| format!("latest_release_request_failed:{error}"))?;
    let final_url = response.url().as_str().trim_end_matches('/');
    let tag = final_url.rsplit('/').next().unwrap_or("");
    if tag.starts_with('v') && parse_release_version(tag).is_some() {
        Ok(tag.to_string())
    } else {
        Err(format!("latest_release_tag_unresolved:{final_url}"))
    }
}

pub(crate) async fn run_check(runtime: &UpgradeRuntime) {
    if try_begin(runtime, UpgradePhase::Checking).is_none() {
        return;
    }
    let current = current_workspace_version().to_string();
    match resolve_latest_tag(&upgrade_base_url()).await {
        Ok(tag) if remote_is_newer(&tag, &current) => set_phase(
            runtime,
            UpgradePhase::UpdateAvailable {
                current_version: current,
                remote_version: tag,
            },
        ),
        Ok(_) => set_phase(
            runtime,
            UpgradePhase::NoUpdate {
                current_version: current,
            },
        ),
        Err(error) => set_phase(runtime, UpgradePhase::Failed { error }),
    }
}

pub(crate) async fn run_apply(shared: &SharedUpgradeRuntime) {
    let current = current_workspace_version().to_string();
    let initial = UpgradePhase::Downloading {
        remote_version: String::new(),
        percent: 0,
    };
    if try_begin(shared, initial).is_none() {
        return;
    }
    if let Err(error) = apply_flow(shared, &current).await {
        set_phase(shared, UpgradePhase::Failed { error });
    }
}

async fn apply_flow(shared: &SharedUpgradeRuntime, current_version: &str) -> Result<(), String> {
    let Some(executable) = shared.executable_path.clone() else {
        return Err("executable_path_unavailable".to_string());
    };
    if platform_target() == "unsupported-target" {
        return Err("unsupported_upgrade_platform".to_string());
    }
    let base = upgrade_base_url();
    let tag = resolve_latest_tag(&base).await?;
    if !remote_is_newer(&tag, current_version) {
        return Err(format!("remote_not_newer:{tag}"));
    }
    set_phase(
        shared,
        UpgradePhase::Downloading {
            remote_version: tag.clone(),
            percent: 0,
        },
    );
    let (archive_name, sidecar_name) = asset_names(&tag);
    let stage_dir = stage_directory(&executable);
    std::fs::create_dir_all(&stage_dir)
        .map_err(|error| format!("stage_dir_create_failed:{error}"))?;
    let result = staged_apply(
        shared,
        &base,
        &tag,
        &stage_dir,
        &archive_name,
        &sidecar_name,
        &executable,
        current_version,
    )
    .await;
    let _ = std::fs::remove_dir_all(&stage_dir);
    result
}

#[allow(clippy::too_many_arguments)]
async fn staged_apply(
    shared: &SharedUpgradeRuntime,
    base: &str,
    tag: &str,
    stage_dir: &Path,
    archive_name: &str,
    sidecar_name: &str,
    executable: &Path,
    current_version: &str,
) -> Result<(), String> {
    let sidecar_url = asset_download_url(base, tag, sidecar_name);
    let sidecar_text = download_text(&sidecar_url, MAX_SIDECAR_BYTES).await?;
    let expected = parse_sha256_sidecar(&sidecar_text)
        .ok_or_else(|| "malformed_checksum_sidecar".to_string())?;
    let archive_path = stage_dir.join(archive_name);
    let archive_url = asset_download_url(base, tag, archive_name);
    let progress_runtime = shared.clone();
    let progress_tag = tag.to_string();
    download_to_file(&archive_url, &archive_path, &move |percent| {
        set_phase(
            &progress_runtime,
            UpgradePhase::Downloading {
                remote_version: progress_tag.clone(),
                percent,
            },
        );
    })
    .await?;
    set_phase(
        shared,
        UpgradePhase::Verifying {
            remote_version: tag.to_string(),
        },
    );
    let actual = sha256_file(&archive_path)?;
    if actual != expected {
        return Err(format!(
            "checksum_mismatch:expected={expected}:actual={actual}"
        ));
    }
    set_phase(
        shared,
        UpgradePhase::Staging {
            remote_version: tag.to_string(),
        },
    );
    let staged = extract_staged_binary(&archive_path, stage_dir)?;
    verify_staged_binary(&staged)?;
    set_phase(
        shared,
        UpgradePhase::Applying {
            remote_version: tag.to_string(),
        },
    );
    replace_executable(&staged, executable)?;
    set_phase(
        shared,
        UpgradePhase::PendingRestart {
            current_version: current_version.to_string(),
            pending_version: tag.to_string(),
        },
    );
    Ok(())
}

pub(crate) fn stage_directory(executable: &Path) -> PathBuf {
    let parent = executable.parent().unwrap_or_else(|| Path::new("."));
    parent.join(format!(".timem-upgrade-stage-{}", std::process::id()))
}

pub(crate) fn parse_sha256_sidecar(sidecar: &str) -> Option<String> {
    let first = sidecar.split_whitespace().next()?;
    let valid = first.len() == 64 && first.chars().all(|c| c.is_ascii_hexdigit());
    if valid {
        Some(first.to_ascii_lowercase())
    } else {
        None
    }
}

#[cfg(test)]
pub(crate) fn sha256_bytes(bytes: &[u8]) -> String {
    use ring::digest::{digest, SHA256};
    let sum = digest(&SHA256, bytes);
    hex_lower(sum.as_ref())
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

pub(crate) fn sha256_file(path: &Path) -> Result<String, String> {
    use ring::digest::{Context, SHA256};
    let mut file =
        std::fs::File::open(path).map_err(|error| format!("open_for_checksum_failed:{error}"))?;
    let mut context = Context::new(&SHA256);
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("checksum_read_failed:{error}"))?;
        if read == 0 {
            break;
        }
        context.update(&buffer[..read]);
    }
    Ok(hex_lower(context.finish().as_ref()))
}

/// Downloads `url` into `destination` while reporting progress as a percent
/// of the announced content length.
pub(crate) async fn download_to_file(
    url: &str,
    destination: &Path,
    on_progress: &(dyn Fn(u8) + Send + Sync),
) -> Result<(), String> {
    let client = http_client(DOWNLOAD_TIMEOUT)?;
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|error| format!("download_request_failed:{error}"))?
        .error_for_status()
        .map_err(|error| format!("download_http_status:{error}"))?;
    let total = response.content_length();
    if let Some(total) = total {
        if total > MAX_ARCHIVE_BYTES {
            return Err(format!("archive_too_large:{total}>{}", MAX_ARCHIVE_BYTES));
        }
    }
    let parent = destination
        .parent()
        .ok_or_else(|| "download_destination_missing_parent".to_string())?;
    std::fs::create_dir_all(parent).map_err(|error| format!("stage_dir_create_failed:{error}"))?;
    let file = std::fs::File::create(destination)
        .map_err(|error| format!("download_file_create_failed:{error}"))?;
    let mut writer = std::io::BufWriter::new(file);
    let mut downloaded: u64 = 0;
    let mut last_percent = 0_u64;
    let mut response = response;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| format!("download_read_failed:{error}"))?
    {
        downloaded += chunk.len() as u64;
        if downloaded > MAX_ARCHIVE_BYTES {
            return Err(format!(
                "archive_too_large:{downloaded}>{}",
                MAX_ARCHIVE_BYTES
            ));
        }
        writer
            .write_all(&chunk)
            .map_err(|error| format!("download_write_failed:{error}"))?;
        if let Some(total) = total.filter(|total| *total > 0) {
            let percent = (downloaded * 100 / total).min(100);
            if percent >= last_percent + DOWNLOAD_PROGRESS_STEP_PERCENT {
                last_percent = percent;
                on_progress(percent as u8);
            }
        }
    }
    writer
        .flush()
        .map_err(|error| format!("download_flush_failed:{error}"))?;
    writer
        .get_ref()
        .sync_all()
        .map_err(|error| format!("download_sync_failed:{error}"))?;
    if downloaded == 0 {
        return Err("download_empty_body".to_string());
    }
    Ok(())
}

pub(crate) async fn download_text(url: &str, max_bytes: u64) -> Result<String, String> {
    let client = http_client(CHECK_TIMEOUT)?;
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|error| format!("request_failed:{error}"))?
        .error_for_status()
        .map_err(|error| format!("http_status:{error}"))?;
    let bytes = response
        .bytes()
        .await
        .map_err(|error| format!("read_failed:{error}"))?;
    if bytes.len() as u64 > max_bytes {
        return Err(format!("response_too_large:{}>{max_bytes}", bytes.len()));
    }
    String::from_utf8(bytes.to_vec()).map_err(|_| "response_not_utf8".to_string())
}

/// Extracts only the expected binary member from the release tar.gz into
/// `destination_dir`. Only a regular file whose member name is exactly the
/// expected binary name is unpacked, so hostile member names (`../`,
/// absolute paths, symlinks) can never escape or replace anything.
pub(crate) fn extract_staged_binary(
    archive: &Path,
    destination_dir: &Path,
) -> Result<PathBuf, String> {
    let file =
        std::fs::File::open(archive).map_err(|error| format!("archive_open_failed:{error}"))?;
    let gz = flate2::read::GzDecoder::new(file);
    let mut tar = tar::Archive::new(gz);
    let expected = staged_binary_name();
    let mut found: Option<PathBuf> = None;
    let entries = tar
        .entries()
        .map_err(|error| format!("archive_read_failed:{error}"))?;
    for entry in entries {
        let mut entry = entry.map_err(|error| format!("archive_entry_read_failed:{error}"))?;
        let path = entry
            .path()
            .map_err(|error| format!("archive_entry_path_failed:{error}"))?
            .to_path_buf();
        let Some(name) = path.to_str() else {
            continue;
        };
        if name != expected {
            continue;
        }
        if !entry.header().entry_type().is_file() {
            return Err("archive_binary_not_regular_file".to_string());
        }
        if entry.size() > MAX_ARCHIVE_BYTES {
            return Err(format!(
                "staged_binary_too_large:{}>{MAX_ARCHIVE_BYTES}",
                entry.size()
            ));
        }
        if found.is_some() {
            return Err("archive_duplicate_binary".to_string());
        }
        std::fs::create_dir_all(destination_dir)
            .map_err(|error| format!("stage_dir_create_failed:{error}"))?;
        let destination = destination_dir.join(expected);
        entry
            .unpack(&destination)
            .map_err(|error| format!("staged_unpack_failed:{error}"))?;
        found = Some(destination);
    }
    let staged = found.ok_or_else(|| format!("archive_missing_binary:{expected}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let metadata = std::fs::metadata(&staged)
            .map_err(|error| format!("staged_metadata_failed:{error}"))?;
        let mut permissions = metadata.permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&staged, permissions)
            .map_err(|error| format!("staged_chmod_failed:{error}"))?;
    }
    Ok(staged)
}

/// Runs the staged binary with `--help` and requires exit code 0 so a
/// truncated or wrong-platform archive is never swapped in. The child is
/// bounded by a timeout so a hostile staged binary cannot hang the server.
pub(crate) fn verify_staged_binary(staged: &Path) -> Result<(), String> {
    let staged = staged.to_path_buf();
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let output = std::process::Command::new(&staged)
            .arg("--help")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output();
        let _ = sender.send(match output {
            Ok(output) if output.status.success() => Ok(()),
            Ok(output) => Err(format!(
                "staged_binary_verify_failed:exit_code:{}",
                output
                    .status
                    .code()
                    .map(|code| code.to_string())
                    .unwrap_or_else(|| "signal".to_string())
            )),
            Err(error) => Err(format!("staged_binary_spawn_failed:{error}")),
        });
    });
    match receiver.recv_timeout(STAGED_VERIFY_TIMEOUT) {
        Ok(result) => result,
        Err(_) => Err("staged_binary_verify_timeout".to_string()),
    }
}

fn timestamp_label() -> String {
    chrono::Local::now().format("%Y%m%d-%H%M%S").to_string()
}

fn backup_path(executable: &Path, kind: &str) -> PathBuf {
    let file_name = executable
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("timem");
    let parent = executable.parent().unwrap_or_else(|| Path::new("."));
    parent.join(format!("{file_name}.{kind}.{}", timestamp_label()))
}

/// Swaps `staged` in for the currently running executable.
///
/// Unix: rename replaces the running binary atomically; the previous binary
/// is preserved as `<name>.bak.<timestamp>` for manual rollback.
/// Windows: the running `.exe` cannot be replaced but can be renamed; the
/// old file is renamed to `<name>.old.<timestamp>` first and restored if
/// moving the staged file in fails.
pub(crate) fn replace_executable(staged: &Path, executable: &Path) -> Result<(), String> {
    let backup = backup_path(executable, "bak");
    std::fs::copy(executable, &backup).map_err(|error| format!("backup_failed:{error}"))?;
    #[cfg(unix)]
    let move_result = std::fs::rename(staged, executable);
    #[cfg(windows)]
    let move_result = {
        let old = backup_path(executable, "old");
        match std::fs::rename(executable, &old) {
            Ok(()) => match std::fs::rename(staged, executable) {
                Ok(()) => Ok(()),
                Err(error) => {
                    let _ = std::fs::rename(&old, executable);
                    Err(error)
                }
            },
            Err(error) => Err(error),
        }
    };
    match move_result {
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = std::fs::remove_file(&backup);
            Err(format!("replace_failed:{error}"))
        }
    }
}

/// Best-effort removal of upgrade leftovers (`.bak.*` / `.old.*`) next to the
/// running executable. Backups newer than the retention window are kept for
/// manual rollback.
pub(crate) fn cleanup_upgrade_leftovers(executable: &Path) {
    let Some(parent) = executable.parent() else {
        return;
    };
    let Some(stem) = executable.file_name().and_then(|name| name.to_str()) else {
        return;
    };
    let cutoff = std::time::SystemTime::now()
        .checked_sub(BACKUP_RETENTION)
        .unwrap_or(std::time::UNIX_EPOCH);
    let Ok(entries) = std::fs::read_dir(parent) else {
        return;
    };
    for entry in entries.flatten() {
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if !name.starts_with(stem) || (!name.contains(".bak.") && !name.contains(".old.")) {
            continue;
        }
        if let Ok(metadata) = entry.metadata() {
            if let Ok(modified) = metadata.modified() {
                if modified >= cutoff {
                    continue;
                }
            }
        }
        let _ = std::fs::remove_file(entry.path());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "timem-self-upgrade-test-{}-{label}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn parses_release_versions_numerically() {
        assert_eq!(
            parse_release_version("v2.10.0"),
            Some(Version3 {
                major: 2,
                minor: 10,
                patch: 0
            })
        );
        assert_eq!(
            parse_release_version("2.6"),
            Some(Version3 {
                major: 2,
                minor: 6,
                patch: 0
            })
        );
        assert_eq!(parse_release_version("v2.6.0.1"), None);
        assert_eq!(parse_release_version("latest"), None);
    }

    #[test]
    fn numeric_version_order_beats_lexicographic() {
        assert!(remote_is_newer("v2.10.0", "v2.9.0"));
        assert!(!remote_is_newer("v2.9.0", "v2.10.0"));
        assert!(remote_is_newer("v3.0.0", "v2.99.99"));
        assert!(!remote_is_newer("v2.6.0", "v2.6.0"));
        assert!(!remote_is_newer("garbage", "v2.6.0"));
    }

    #[test]
    fn sidecar_parsing_accepts_only_hex_digests() {
        assert_eq!(
            parse_sha256_sidecar(
                "ABCDEF0123456789abcdef0123456789abcdef0123456789abcdef0123456789  timem.tar.gz\n"
            )
            .as_deref(),
            Some("abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789")
        );
        assert_eq!(parse_sha256_sidecar("short  file"), None);
        assert_eq!(parse_sha256_sidecar(""), None);
        assert_eq!(
            parse_sha256_sidecar(
                "zfffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"
            ),
            None
        );
    }

    #[test]
    fn asset_names_follow_the_release_layout() {
        let (archive, sidecar) = asset_names("v9.9.9");
        assert_eq!(
            archive,
            format!("timem-v9.9.9-{}.tar.gz", platform_target())
        );
        assert_eq!(sidecar, format!("{archive}.sha256"));
    }

    #[test]
    fn repository_validation_rejects_injection() {
        assert!(valid_repository("moliam/TimemAi"));
        assert!(!valid_repository("moliam/TimemAi/extra"));
        assert!(!valid_repository("/moliam/TimemAi"));
        assert!(!valid_repository("moliam//TimemAi"));
        assert!(!valid_repository(""));
        assert!(!valid_repository("evil@example.com/TimemAi"));
    }

    #[test]
    fn sha256_file_matches_bytes_hash() {
        let dir = temp_dir("sha");
        let file = dir.join("payload.bin");
        std::fs::write(&file, b"timem self upgrade checksum").unwrap();
        assert_eq!(
            sha256_file(&file).unwrap(),
            sha256_bytes(b"timem self upgrade checksum")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn write_archive(dir: &Path, members: &[(&str, &[u8])]) -> PathBuf {
        let archive = dir.join("asset.tar.gz");
        let file = std::fs::File::create(&archive).unwrap();
        let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        let mut builder = tar::Builder::new(encoder);
        for (name, bytes) in members {
            let mut header = tar::Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder
                .append_data(&mut header, name, std::io::Cursor::new(bytes))
                .unwrap();
        }
        builder.into_inner().unwrap().finish().unwrap();
        archive
    }

    #[cfg(unix)]
    fn executable_script(dir: &Path, name: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        let mut permissions = std::fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&path, permissions).unwrap();
        path
    }

    #[cfg(unix)]
    #[test]
    fn extract_only_the_expected_binary_member() {
        let dir = temp_dir("extract");
        let archive = write_archive(
            &dir,
            &[
                ("README-ASSETS.txt", b"docs" as &[u8]),
                ("timem", b"#!/bin/sh\nexit 0\n"),
                ("nested/timem", b"#!/bin/sh\nexit 1\n"),
            ],
        );
        let staged_dir = dir.join("staged");
        let staged = extract_staged_binary(&archive, &staged_dir).unwrap();
        assert_eq!(staged, staged_dir.join("timem"));
        assert_eq!(
            std::fs::read(&staged).unwrap(),
            b"#!/bin/sh\nexit 0\n".to_vec()
        );
        assert!(!staged_dir.join("README-ASSETS.txt").exists());
        assert!(!staged_dir.join("nested").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn extract_rejects_missing_and_duplicate_binaries() {
        let dir = temp_dir("extract-missing");
        let archive = write_archive(&dir, &[("other.txt", b"no binary")]);
        let err = extract_staged_binary(&archive, &dir.join("staged")).unwrap_err();
        assert!(err.starts_with("archive_missing_binary:"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn staged_binary_verification_requires_help_success() {
        let dir = temp_dir("verify");
        let good = executable_script(&dir, "good", "#!/bin/sh\nexit 0\n");
        let bad = executable_script(&dir, "bad", "#!/bin/sh\nexit 3\n");
        assert_eq!(verify_staged_binary(&good), Ok(()));
        assert!(verify_staged_binary(&bad)
            .unwrap_err()
            .starts_with("staged_binary_verify_failed"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn replace_swaps_executable_and_keeps_backup() {
        let dir = temp_dir("replace");
        let executable = executable_script(&dir, "timem", "#!/bin/sh\necho old\n");
        let staged_dir = dir.join("stage");
        std::fs::create_dir_all(&staged_dir).unwrap();
        let staged = executable_script(&staged_dir, "timem", "#!/bin/sh\necho new\n");
        replace_executable(&staged, &executable).unwrap();
        assert_eq!(
            std::fs::read(&executable).unwrap(),
            b"#!/bin/sh\necho new\n".to_vec()
        );
        assert!(!staged.exists());
        let backups: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|entry| {
                entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| name.starts_with("timem.bak."))
            })
            .collect();
        assert_eq!(backups.len(), 1);
        assert_eq!(
            std::fs::read(backups[0].path()).unwrap(),
            b"#!/bin/sh\necho old\n".to_vec()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn phase_busy_tracking_covers_active_operations_only() {
        assert!(!UpgradePhase::Idle.is_busy());
        assert!(!UpgradePhase::NoUpdate {
            current_version: "2.6.0".into()
        }
        .is_busy());
        assert!(!UpgradePhase::UpdateAvailable {
            current_version: "2.6.0".into(),
            remote_version: "v2.7.0".into()
        }
        .is_busy());
        assert!(!UpgradePhase::Failed { error: "x".into() }.is_busy());
        assert!(UpgradePhase::Checking.is_busy());
        assert!(UpgradePhase::Downloading {
            remote_version: "v2.7.0".into(),
            percent: 10
        }
        .is_busy());
        assert!(UpgradePhase::PendingRestart {
            current_version: "2.6.0".into(),
            pending_version: "v2.7.0".into()
        }
        .is_busy());
    }

    #[test]
    fn try_begin_rejects_concurrent_operations() {
        let runtime = new_upgrade_runtime(None);
        assert!(try_begin(&runtime, UpgradePhase::Checking).is_some());
        assert!(try_begin(&runtime, UpgradePhase::Checking).is_none());
        set_phase(
            &runtime,
            UpgradePhase::Failed {
                error: "done".into(),
            },
        );
        assert!(try_begin(&runtime, UpgradePhase::Checking).is_some());
    }
}
