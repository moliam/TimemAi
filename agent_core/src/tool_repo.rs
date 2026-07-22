use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::process::CommandExt;

const MANIFEST_FILE: &str = ".timem-tool.json";
const README_FILE: &str = "README.md";
const MAX_TOOL_FILES: usize = 64;
const MAX_TOOL_BYTES: u64 = 2 * 1024 * 1024;
const MAX_SEARCH_FILE_BYTES: u64 = 256 * 1024;
const MAX_SELF_TEST_MS: u64 = 60_000;
static NEXT_DRAFT_ID: AtomicU64 = AtomicU64::new(1);
static NEXT_TOOL_ID: AtomicU64 = AtomicU64::new(1);
static REPO_LOCKS: OnceLock<Mutex<BTreeMap<PathBuf, Weak<Mutex<()>>>>> = OnceLock::new();

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolManifest {
    #[serde(default)]
    pub tool_id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub tool_type: String,
    pub language: String,
    pub entrypoint: String,
    pub synopsis: String,
    pub self_test: ToolSelfTest,
    #[serde(default)]
    pub created_at_ms: i64,
    #[serde(default)]
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolSelfTest {
    #[serde(default)]
    pub entrypoint: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default = "default_self_test_timeout_ms")]
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolSummary {
    pub tool_id: String,
    pub name: String,
    pub tool_type: String,
    pub language: String,
    pub synopsis: String,
    pub entrypoint: String,
    pub path: String,
    pub updated_at_ms: i64,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolFileEntry {
    pub path: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolDetail {
    pub summary: ToolSummary,
    pub readme: String,
    pub files: Vec<ToolFileEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolPublishResult {
    pub summary: ToolSummary,
    pub validation_output: String,
    pub updated_existing: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionToolRepo {
    session_root: PathBuf,
}

impl SessionToolRepo {
    pub fn new(memory_dir: impl AsRef<Path>, session_id: &str) -> Self {
        Self {
            session_root: memory_dir
                .as_ref()
                .join("sessions")
                .join(sanitize_component(session_id)),
        }
    }

    pub fn root(&self) -> PathBuf {
        self.session_root.join("toolrepo")
    }

    pub fn drafts_dir(&self) -> PathBuf {
        self.root().join(".drafts")
    }

    pub fn create_draft(&self) -> Result<PathBuf, String> {
        let lock = repo_lock(&self.root());
        let _guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let draft = self.drafts_dir().join(format!(
            "draft_{}_{}_{}",
            now_ms(),
            std::process::id(),
            NEXT_DRAFT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&draft).map_err(|error| format!("tool_draft_create_failed:{error}"))?;
        Ok(draft)
    }

    pub fn discard_draft(&self, draft_path: impl AsRef<Path>) -> Result<(), String> {
        let lock = repo_lock(&self.root());
        let _guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let drafts = canonical_or_lexical(&self.drafts_dir());
        let draft = canonical_or_lexical(draft_path.as_ref());
        if !draft.starts_with(&drafts) || draft == drafts {
            return Err("tool_draft_outside_session_repo".to_string());
        }
        if draft.exists() {
            fs::remove_dir_all(draft)
                .map_err(|error| format!("tool_draft_remove_failed:{error}"))?;
        }
        Ok(())
    }

    pub fn list(&self) -> Result<Vec<ToolSummary>, String> {
        let root = self.root();
        if !root.exists() {
            return Ok(Vec::new());
        }
        let mut tools = Vec::new();
        for entry in fs::read_dir(&root).map_err(|error| format!("toolrepo_read_failed:{error}"))? {
            let entry = entry.map_err(|error| format!("toolrepo_read_failed:{error}"))?;
            let path = entry.path();
            if !path.is_dir() || entry.file_name() == ".drafts" {
                continue;
            }
            if let Ok(manifest) = read_manifest(&path) {
                tools.push(summary_from_manifest(&path, &manifest));
            }
        }
        tools.sort_by(|left, right| {
            right
                .updated_at_ms
                .cmp(&left.updated_at_ms)
                .then_with(|| left.name.cmp(&right.name))
        });
        Ok(tools)
    }

    pub fn detail(&self, tool_id: &str) -> Result<ToolDetail, String> {
        let path = self.find_tool_path(tool_id)?;
        let manifest = read_manifest(&path)?;
        let readme = fs::read_to_string(path.join(README_FILE))
            .map_err(|error| format!("tool_readme_read_failed:{error}"))?;
        Ok(ToolDetail {
            summary: summary_from_manifest(&path, &manifest),
            readme,
            files: collect_tool_files(&path)?,
        })
    }

    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<ToolSummary>, String> {
        let needle = query.trim().to_ascii_lowercase();
        let limit = limit.clamp(1, 200);
        let mut matches = Vec::new();
        for summary in self.list()? {
            let metadata_match = needle.is_empty()
                || format!(
                    "{} {} {} {} {}",
                    summary.name,
                    summary.tool_type,
                    summary.language,
                    summary.synopsis,
                    summary.entrypoint
                )
                .to_ascii_lowercase()
                .contains(&needle);
            let content_match = !metadata_match
                && tool_content_contains(Path::new(&summary.path), &needle).unwrap_or(false);
            if metadata_match || content_match {
                matches.push(summary);
                if matches.len() >= limit {
                    break;
                }
            }
        }
        Ok(matches)
    }

    pub fn rename(&self, tool_id: &str, new_name: &str) -> Result<ToolSummary, String> {
        let lock = repo_lock(&self.root());
        let _guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        validate_semantic_name(new_name)?;
        let source = self.find_tool_path(tool_id)?;
        let target = self.root().join(new_name);
        if target.exists() && target != source {
            return Err("tool_name_already_exists".to_string());
        }
        let mut manifest = read_manifest(&source)?;
        manifest.name = new_name.to_string();
        manifest.updated_at_ms = now_ms();
        write_manifest_atomic(&source, &manifest)?;
        if source != target {
            fs::rename(&source, &target).map_err(|error| format!("tool_rename_failed:{error}"))?;
        }
        Ok(summary_from_manifest(&target, &manifest))
    }

    pub fn publish(&self, draft_path: impl AsRef<Path>) -> Result<ToolPublishResult, String> {
        let lock = repo_lock(&self.root());
        let _guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let drafts = canonical_or_lexical(&self.drafts_dir());
        let draft = canonical_or_lexical(draft_path.as_ref());
        if !draft.starts_with(&drafts) || draft == drafts {
            return Err("tool_draft_outside_session_repo".to_string());
        }
        validate_tool_tree(&draft)?;
        let mut manifest = read_manifest(&draft)?;
        validate_manifest(&manifest)?;
        let validation_output = run_self_test(&draft, &manifest)?;

        let now = now_ms();
        let existing = if manifest.tool_id.trim().is_empty() {
            None
        } else {
            self.find_tool_path(&manifest.tool_id).ok()
        };
        let updated_existing = existing.is_some();
        if manifest.tool_id.trim().is_empty() {
            manifest.tool_id = format!(
                "tool_{}_{}_{}",
                now,
                std::process::id(),
                NEXT_TOOL_ID.fetch_add(1, Ordering::Relaxed)
            );
        }
        if manifest.created_at_ms <= 0 {
            manifest.created_at_ms = existing
                .as_ref()
                .and_then(|path| read_manifest(path).ok())
                .map(|old| old.created_at_ms)
                .filter(|value| *value > 0)
                .unwrap_or(now);
        }
        manifest.updated_at_ms = now;
        write_manifest_atomic(&draft, &manifest)?;

        fs::create_dir_all(self.root())
            .map_err(|error| format!("toolrepo_create_failed:{error}"))?;
        let target = self.root().join(&manifest.name);
        if target.exists() && existing.as_ref() != Some(&target) {
            return Err("tool_name_already_exists".to_string());
        }
        let backup = existing.as_ref().map(|_| {
            self.root()
                .join(format!(".backup_{}_{}", manifest.tool_id, now))
        });
        if let (Some(source), Some(backup)) = (existing.as_ref(), backup.as_ref()) {
            fs::rename(source, backup)
                .map_err(|error| format!("tool_update_backup_failed:{error}"))?;
        }
        if let Err(error) = fs::rename(&draft, &target) {
            if let (Some(source), Some(backup)) = (existing.as_ref(), backup.as_ref()) {
                let _ = fs::rename(backup, source);
            }
            return Err(format!("tool_publish_failed:{error}"));
        }
        if let Some(backup) = backup {
            let _ = fs::remove_dir_all(backup);
        }
        Ok(ToolPublishResult {
            summary: summary_from_manifest(&target, &manifest),
            validation_output,
            updated_existing,
        })
    }

    fn find_tool_path(&self, tool_id: &str) -> Result<PathBuf, String> {
        self.list()?
            .into_iter()
            .find(|tool| tool.tool_id == tool_id)
            .map(|tool| PathBuf::from(tool.path))
            .ok_or_else(|| "tool_not_found".to_string())
    }
}

fn repo_lock(root: &Path) -> Arc<Mutex<()>> {
    let key = canonical_or_lexical(root);
    let locks = REPO_LOCKS.get_or_init(|| Mutex::new(BTreeMap::new()));
    let mut locks = locks
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(lock) = locks.get(&key).and_then(Weak::upgrade) {
        return lock;
    }
    let lock = Arc::new(Mutex::new(()));
    locks.insert(key, Arc::downgrade(&lock));
    lock
}

fn default_self_test_timeout_ms() -> u64 {
    10_000
}

fn validate_manifest(manifest: &ToolManifest) -> Result<(), String> {
    validate_semantic_name(&manifest.name)?;
    for (field, value) in [
        ("type", manifest.tool_type.as_str()),
        ("language", manifest.language.as_str()),
        ("synopsis", manifest.synopsis.as_str()),
        ("entrypoint", manifest.entrypoint.as_str()),
    ] {
        if value.trim().is_empty() {
            return Err(format!("tool_manifest_{field}_required"));
        }
    }
    validate_relative_path(&manifest.entrypoint)?;
    if !manifest.self_test.entrypoint.trim().is_empty() {
        validate_relative_path(&manifest.self_test.entrypoint)?;
    }
    if manifest.self_test.timeout_ms == 0 || manifest.self_test.timeout_ms > MAX_SELF_TEST_MS {
        return Err("tool_self_test_timeout_out_of_range".to_string());
    }
    Ok(())
}

fn validate_semantic_name(name: &str) -> Result<(), String> {
    let name = name.trim();
    if name.len() < 3 || name.len() > 80 {
        return Err("tool_name_length_invalid".to_string());
    }
    if !name
        .chars()
        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
        || name.starts_with('-')
        || name.ends_with('-')
        || name.contains("--")
    {
        return Err("tool_name_must_be_semantic_kebab_case".to_string());
    }
    Ok(())
}

fn validate_relative_path(value: &str) -> Result<(), String> {
    let path = Path::new(value);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err("tool_path_must_be_relative".to_string());
    }
    Ok(())
}

fn validate_tool_tree(root: &Path) -> Result<(), String> {
    if !root.is_dir() {
        return Err("tool_draft_not_found".to_string());
    }
    if !root.join(README_FILE).is_file() {
        return Err("tool_readme_required".to_string());
    }
    let manifest = read_manifest(root)?;
    let entrypoint = root.join(&manifest.entrypoint);
    if !entrypoint.is_file() {
        return Err("tool_entrypoint_not_found".to_string());
    }
    if !manifest.self_test.entrypoint.trim().is_empty()
        && !root.join(&manifest.self_test.entrypoint).is_file()
    {
        return Err("tool_self_test_entrypoint_not_found".to_string());
    }
    let mut pending = vec![root.to_path_buf()];
    let mut count = 0usize;
    let mut bytes = 0u64;
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(&dir).map_err(|error| format!("tool_tree_read_failed:{error}"))? {
            let entry = entry.map_err(|error| format!("tool_tree_read_failed:{error}"))?;
            let metadata = fs::symlink_metadata(entry.path())
                .map_err(|error| format!("tool_tree_metadata_failed:{error}"))?;
            if metadata.file_type().is_symlink() {
                return Err("tool_symlink_not_allowed".to_string());
            }
            if metadata.is_dir() {
                pending.push(entry.path());
            } else if metadata.is_file() {
                count = count.saturating_add(1);
                bytes = bytes.saturating_add(metadata.len());
                if count > MAX_TOOL_FILES {
                    return Err("tool_file_count_limit_exceeded".to_string());
                }
                if bytes > MAX_TOOL_BYTES {
                    return Err("tool_size_limit_exceeded".to_string());
                }
            }
        }
    }
    Ok(())
}

fn run_self_test(root: &Path, manifest: &ToolManifest) -> Result<String, String> {
    let self_test_entrypoint = if manifest.self_test.entrypoint.trim().is_empty() {
        &manifest.entrypoint
    } else {
        &manifest.self_test.entrypoint
    };
    let entrypoint = root.join(self_test_entrypoint);
    let mut command = match manifest.language.trim().to_ascii_lowercase().as_str() {
        "python" | "python3" => {
            let mut command = Command::new("python3");
            command.arg(&entrypoint);
            command
        }
        "bash" | "shell" | "sh" => {
            let mut command = Command::new("/bin/bash");
            command.arg(&entrypoint);
            command
        }
        _ => Command::new(&entrypoint),
    };
    command
        .args(&manifest.self_test.args)
        .current_dir(root)
        .env_clear()
        .env(
            "PATH",
            std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".to_string()),
        )
        .env("TMPDIR", std::env::temp_dir())
        .env("TIMEM_TOOLGEN_SELF_TEST", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    unsafe {
        command.pre_exec(|| {
            if libc::setpgid(0, 0) == 0 {
                Ok(())
            } else {
                Err(std::io::Error::last_os_error())
            }
        });
    }
    let mut child = command
        .spawn()
        .map_err(|error| format!("tool_self_test_spawn_failed:{error}"))?;
    let stdout_reader = child
        .stdout
        .take()
        .map(|stdout| thread::spawn(move || read_bounded_stream(stdout, 16 * 1024)));
    let stderr_reader = child
        .stderr
        .take()
        .map(|stderr| thread::spawn(move || read_bounded_stream(stderr, 16 * 1024)));
    let deadline = Instant::now() + Duration::from_millis(manifest.self_test.timeout_ms);
    let status = loop {
        match child
            .try_wait()
            .map_err(|error| format!("tool_self_test_wait_failed:{error}"))?
        {
            Some(status) => break status,
            None if Instant::now() >= deadline => {
                #[cfg(unix)]
                unsafe {
                    libc::kill(-(child.id() as i32), libc::SIGKILL);
                }
                let _ = child.kill();
                let _ = child.wait();
                let _ = join_bounded_reader(stdout_reader);
                let _ = join_bounded_reader(stderr_reader);
                return Err("tool_self_test_timeout".to_string());
            }
            None => thread::sleep(Duration::from_millis(20)),
        }
    };
    let mut output = join_bounded_reader(stdout_reader);
    let stderr = join_bounded_reader(stderr_reader);
    if !stderr.is_empty() {
        if !output.is_empty() {
            output.push('\n');
        }
        output.push_str(&stderr);
    }
    if !status.success() {
        return Err(format!(
            "tool_self_test_failed:exit_code={}:{}",
            status.code().unwrap_or(-1),
            compact(&output, 2_000)
        ));
    }
    Ok(compact(&output, 2_000))
}

fn read_bounded_stream(mut stream: impl Read, max_bytes: usize) -> String {
    let mut retained = Vec::with_capacity(max_bytes.min(8 * 1024));
    let mut chunk = [0u8; 8 * 1024];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(read) => {
                let keep = max_bytes.saturating_sub(retained.len()).min(read);
                retained.extend_from_slice(&chunk[..keep]);
            }
        }
    }
    String::from_utf8_lossy(&retained).into_owned()
}

fn join_bounded_reader(reader: Option<thread::JoinHandle<String>>) -> String {
    reader
        .and_then(|reader| reader.join().ok())
        .unwrap_or_default()
}

fn read_manifest(root: &Path) -> Result<ToolManifest, String> {
    let raw = fs::read_to_string(root.join(MANIFEST_FILE))
        .map_err(|error| format!("tool_manifest_read_failed:{error}"))?;
    serde_json::from_str(&raw).map_err(|error| format!("tool_manifest_invalid:{error}"))
}

fn write_manifest_atomic(root: &Path, manifest: &ToolManifest) -> Result<(), String> {
    let target = root.join(MANIFEST_FILE);
    let temp = root.join(format!("{MANIFEST_FILE}.tmp"));
    let raw = serde_json::to_string_pretty(manifest)
        .map_err(|error| format!("tool_manifest_serialize_failed:{error}"))?;
    fs::write(&temp, format!("{raw}\n"))
        .map_err(|error| format!("tool_manifest_write_failed:{error}"))?;
    fs::rename(&temp, &target).map_err(|error| format!("tool_manifest_commit_failed:{error}"))
}

fn summary_from_manifest(path: &Path, manifest: &ToolManifest) -> ToolSummary {
    ToolSummary {
        tool_id: manifest.tool_id.clone(),
        name: manifest.name.clone(),
        tool_type: manifest.tool_type.clone(),
        language: manifest.language.clone(),
        synopsis: manifest.synopsis.clone(),
        entrypoint: manifest.entrypoint.clone(),
        path: path.display().to_string(),
        updated_at_ms: manifest.updated_at_ms,
        status: "ready".to_string(),
    }
}

fn collect_tool_files(root: &Path) -> Result<Vec<ToolFileEntry>, String> {
    let mut files = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(&dir).map_err(|error| format!("tool_tree_read_failed:{error}"))? {
            let entry = entry.map_err(|error| format!("tool_tree_read_failed:{error}"))?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)
                .map_err(|error| format!("tool_tree_metadata_failed:{error}"))?;
            if metadata.is_dir() {
                pending.push(path);
            } else if metadata.is_file() {
                let relative = path
                    .strip_prefix(root)
                    .unwrap_or(&path)
                    .display()
                    .to_string();
                files.push(ToolFileEntry {
                    path: relative,
                    bytes: metadata.len(),
                });
            }
        }
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(files)
}

fn tool_content_contains(root: &Path, needle: &str) -> Result<bool, String> {
    let mut visited = BTreeSet::new();
    for file in collect_tool_files(root)? {
        if file.bytes > MAX_SEARCH_FILE_BYTES || !visited.insert(file.path.clone()) {
            continue;
        }
        let path = root.join(&file.path);
        let Ok(bytes) = fs::read(path) else {
            continue;
        };
        let Ok(text) = String::from_utf8(bytes) else {
            continue;
        };
        if text.to_ascii_lowercase().contains(needle) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn canonical_or_lexical(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn sanitize_component(value: &str) -> String {
    let value = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    if value.is_empty() {
        "session".to_string()
    } else {
        value
    }
}

fn compact(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let compact = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        format!("{compact}...")
    } else {
        compact
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

#[cfg(test)]
#[path = "../tests/unit/tool_repo_tests.rs"]
mod tests;
