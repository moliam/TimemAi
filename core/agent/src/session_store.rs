use crate::atomic_write_file;
use crate::MemGuard;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

const DEFAULT_HISTORY_PAGE_LIMIT: usize = 200;
const MAX_SESSION_INDEX_RECORD_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredSession {
    pub session_id: String,
    pub display_name: String,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub current_dir: String,
    pub profile: StoredSessionProfile,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env_overrides: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub mcp_server_ids: Vec<String>,
    pub state: StoredSessionState,
    pub last_turn_id: Option<String>,
    pub raw_chat_history_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct StoredSessionProfile {
    pub model: String,
    pub api_protocol: String,
    pub response_protocol: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StoredSessionState {
    Ready,
    Interrupted,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChatHistoryRecord {
    Message {
        role: ChatHistoryRole,
        turn_id: String,
        created_at_ms: i64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kind: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        command_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        delivery_state: Option<ChatCommandDeliveryState>,
        content: String,
    },
    Event {
        role: ChatHistoryRole,
        turn_id: String,
        created_at_ms: i64,
        kind: ChatHistoryEventKind,
        content: String,
        #[serde(flatten)]
        extra: BTreeMap<String, Value>,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChatCommandDeliveryState {
    Recorded,
    CoreAccepted,
}

impl ChatHistoryRecord {
    pub fn created_at_ms(&self) -> i64 {
        match self {
            ChatHistoryRecord::Message { created_at_ms, .. }
            | ChatHistoryRecord::Event { created_at_ms, .. } => *created_at_ms,
        }
    }

    pub fn turn_id(&self) -> &str {
        match self {
            ChatHistoryRecord::Message { turn_id, .. }
            | ChatHistoryRecord::Event { turn_id, .. } => turn_id,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChatHistoryRole {
    User,
    Assistant,
    System,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChatHistoryEventKind {
    FreeTalk,
    Progress,
    Action,
    ActionResult,
    ContextCompact,
    Repair,
    SubAnswer,
    RuntimeNotice,
    Stats,
    Attachment,
}

fn temporary_event_time(record: &ChatHistoryRecord) -> Option<i64> {
    match record {
        ChatHistoryRecord::Event {
            created_at_ms,
            kind:
                ChatHistoryEventKind::Action
                | ChatHistoryEventKind::ActionResult
                | ChatHistoryEventKind::ContextCompact
                | ChatHistoryEventKind::Repair,
            ..
        } => Some(*created_at_ms),
        _ => None,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatHistoryPage {
    pub records: Vec<ChatHistoryRecord>,
    pub before_cursor: Option<String>,
    pub has_more: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionIndexRecovery {
    pub sessions: Vec<StoredSession>,
    pub invalid_records: usize,
    /// First retained corrupt artifact, kept for source compatibility.
    pub backup_path: Option<PathBuf>,
    /// Every corrupt artifact retained during this recovery pass.
    pub backup_paths: Vec<PathBuf>,
}

impl SessionIndexRecovery {
    pub fn repaired(&self) -> bool {
        !self.backup_paths.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionResumeNotice {
    pub history_path: PathBuf,
    pub current_dir: PathBuf,
}

impl SessionResumeNotice {
    pub fn render(&self) -> String {
        format!(
            "Runtime just restarted. Previous chat history's runtime info/tasks are invalid/outdated unless user asks to retrieve them.\n\n{}\n\nCurrent cwd: {}",
            chat_history_prompt_format_hint(&self.history_path),
            self.current_dir.display()
        )
    }
}

#[derive(Debug, Clone)]
pub struct SessionStore {
    root: PathBuf,
    history_indexes: Arc<Mutex<BTreeMap<PathBuf, HistoryIndex>>>,
    temporary_event_summaries: Arc<Mutex<BTreeMap<PathBuf, TemporaryEventSummary>>>,
    index_lock: Arc<Mutex<()>>,
    guard: MemGuard,
}

#[derive(Debug, Clone)]
struct HistoryIndex {
    file_len: u64,
    modified_at_ms: Option<u128>,
    entries: Vec<HistoryIndexEntry>,
}

#[derive(Debug, Clone)]
struct HistoryIndexEntry {
    byte_offset: u64,
    byte_len: u64,
    turn_id: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
struct TemporaryEventSummary {
    file_len: u64,
    min_created_at_ms: Option<i64>,
}

impl SessionStore {
    pub fn new(root: impl AsRef<Path>) -> Self {
        let root = root.as_ref().to_path_buf();
        Self {
            guard: MemGuard::for_memory_domain(&root, "session-index"),
            root,
            history_indexes: Arc::new(Mutex::new(BTreeMap::new())),
            temporary_event_summaries: Arc::new(Mutex::new(BTreeMap::new())),
            index_lock: Arc::new(Mutex::new(())),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn sessions_dir(&self) -> PathBuf {
        self.root.join("sessions")
    }

    pub fn index_path(&self) -> PathBuf {
        self.sessions_dir().join("index.jsonl")
    }

    pub fn layout_marker_path(&self) -> PathBuf {
        self.sessions_dir().join(".metadata-v2")
    }

    pub fn metadata_path_for_session(&self, session_id: &str) -> PathBuf {
        self.sessions_dir()
            .join(sanitize_session_path_component(session_id))
            .join("session.json")
    }

    pub fn history_path_for_session(&self, session_id: &str) -> PathBuf {
        self.sessions_dir()
            .join(sanitize_session_path_component(session_id))
            .join("raw_chat_history.jsonl")
    }

    pub fn upsert_session(&self, session: &StoredSession) -> Result<(), String> {
        validate_session_id(&session.session_id)?;
        self.ensure_metadata_v2()?;
        let path = self.metadata_path_for_session(&session.session_id);
        let guard = MemGuard::for_memory_domain(
            &self.root,
            format!(
                "session-data-{}",
                sanitize_session_path_component(&session.session_id)
            ),
        );
        guard.with_write(|| {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(|_| "session_dir_create_failed".to_string())?;
                restrict_session_path_permissions(parent, true)?;
            }
            let mut payload = serde_json::to_vec_pretty(session)
                .map_err(|_| "session_record_serialize_failed".to_string())?;
            payload.push(b'\n');
            crate::atomic_write_file(&path, &payload)
                .map_err(|_| "session_metadata_write_failed".to_string())?;
            restrict_session_path_permissions(&path, false)
        })?
    }

    fn ensure_metadata_v2(&self) -> Result<(), String> {
        if self.layout_marker_path().exists() {
            return Ok(());
        }
        let _index_lock = self
            .index_lock
            .lock()
            .map_err(|_| "session_index_lock_poisoned".to_string())?;
        self.guard.with_write(|| {
            if self.layout_marker_path().exists() {
                return Ok(());
            }
            fs::create_dir_all(self.sessions_dir())
                .map_err(|_| "session_dir_create_failed".to_string())?;
            restrict_session_path_permissions(&self.sessions_dir(), true)?;
            let sessions = self.list_legacy_sessions_unlocked()?;
            for session in sessions {
                let path = self.metadata_path_for_session(&session.session_id);
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)
                        .map_err(|_| "session_dir_create_failed".to_string())?;
                    restrict_session_path_permissions(parent, true)?;
                }
                let mut payload = serde_json::to_vec_pretty(&session)
                    .map_err(|_| "session_record_serialize_failed".to_string())?;
                payload.push(b'\n');
                crate::atomic_write_file(&path, &payload)
                    .map_err(|_| "session_metadata_write_failed".to_string())?;
                restrict_session_path_permissions(&path, false)?;
            }
            crate::atomic_write_file(&self.layout_marker_path(), b"2\n")
                .map_err(|_| "session_layout_marker_write_failed".to_string())?;
            restrict_session_path_permissions(&self.layout_marker_path(), false)?;
            if self.index_path().exists() {
                let archived = self.sessions_dir().join("index.v1.jsonl");
                if !archived.exists() {
                    fs::rename(self.index_path(), archived)
                        .map_err(|_| "session_index_archive_failed".to_string())?;
                } else {
                    fs::remove_file(self.index_path())
                        .map_err(|_| "session_index_archive_failed".to_string())?;
                }
            }
            Ok(())
        })?
    }
    pub fn list_sessions(&self) -> Result<Vec<StoredSession>, String> {
        if self.layout_marker_path().exists() {
            self.list_metadata_sessions_unlocked()
        } else {
            self.list_legacy_sessions_unlocked()
        }
    }

    /// Loads the restore index and repairs malformed JSONL records without
    /// discarding valid sessions. The exact original file is retained beside
    /// the index before the repaired replacement is installed.
    pub fn list_sessions_resilient(&self) -> Result<SessionIndexRecovery, String> {
        if self.layout_marker_path().exists() {
            let _index_lock = self
                .index_lock
                .lock()
                .map_err(|_| "session_index_lock_poisoned".to_string())?;
            return self
                .guard
                .with_write(|| self.recover_metadata_sessions_unlocked())?;
        }
        let recovery = match self.list_legacy_sessions_unlocked() {
            Ok(sessions) => SessionIndexRecovery {
                sessions,
                invalid_records: 0,
                backup_path: None,
                backup_paths: Vec::new(),
            },
            Err(error) if error == "session_record_parse_failed" => {
                let _index_lock = self
                    .index_lock
                    .lock()
                    .map_err(|_| "session_index_lock_poisoned".to_string())?;
                self.guard
                    .with_write(|| self.repair_session_index_unlocked())??
            }
            Err(error) => return Err(error),
        };
        self.ensure_metadata_v2()?;
        Ok(recovery)
    }

    fn repair_session_index_unlocked(&self) -> Result<SessionIndexRecovery, String> {
        // Another process may have repaired the index while this caller waited
        // for the cross-process memory guard.
        if let Ok(sessions) = self.list_legacy_sessions_unlocked() {
            return Ok(SessionIndexRecovery {
                sessions,
                invalid_records: 0,
                backup_path: None,
                backup_paths: Vec::new(),
            });
        }
        let path = self.index_path();
        let file =
            fs::File::open(&path).map_err(|_| "session_index_recovery_read_failed".to_string())?;
        let mut reader = BufReader::new(file);
        let mut sessions = Vec::new();
        let mut invalid_records = 0usize;
        while let Some(line) = read_bounded_jsonl_record(
            &mut reader,
            MAX_SESSION_INDEX_RECORD_BYTES,
            "session_index_recovery_read_failed",
        )? {
            if line.oversized {
                invalid_records = invalid_records.saturating_add(1);
                continue;
            }
            if line.bytes.iter().all(|byte| byte.is_ascii_whitespace()) {
                continue;
            }
            match serde_json::from_slice::<StoredSession>(&line.bytes) {
                Ok(session) => sessions.push(session),
                Err(_) => invalid_records = invalid_records.saturating_add(1),
            }
        }
        sessions.sort_by(|left, right| {
            right
                .updated_at_ms
                .cmp(&left.updated_at_ms)
                .then_with(|| left.session_id.cmp(&right.session_id))
        });
        let mut seen_session_ids = BTreeSet::new();
        sessions.retain(|session| {
            if seen_session_ids.insert(session.session_id.clone()) {
                true
            } else {
                invalid_records = invalid_records.saturating_add(1);
                false
            }
        });
        if invalid_records == 0 {
            return Err("session_index_recovery_not_needed".to_string());
        }
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let backup_path = self
            .sessions_dir()
            .join(format!("index.jsonl.session-index-corrupt-backup-{suffix}"));
        copy_file_synced(&path, &backup_path, "session_index_backup")?;
        self.write_sessions_unlocked(&sessions)?;
        Ok(SessionIndexRecovery {
            sessions,
            invalid_records,
            backup_path: Some(backup_path.clone()),
            backup_paths: vec![backup_path],
        })
    }

    fn recover_metadata_sessions_unlocked(&self) -> Result<SessionIndexRecovery, String> {
        let mut sessions = Vec::new();
        let mut backup_paths = Vec::new();
        let entries = match fs::read_dir(self.sessions_dir()) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(SessionIndexRecovery {
                    sessions,
                    invalid_records: 0,
                    backup_path: None,
                    backup_paths,
                });
            }
            Err(_) => return Err("session_dir_read_failed".to_string()),
        };
        for entry in entries {
            let entry = entry.map_err(|_| "session_dir_read_failed".to_string())?;
            let file_type = entry
                .file_type()
                .map_err(|_| "session_dir_read_failed".to_string())?;
            if !file_type.is_dir() || entry.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            let path = entry.path().join("session.json");
            if !path.exists() {
                continue;
            }
            let parsed = fs::read(&path)
                .map_err(|_| "session_metadata_read_failed".to_string())
                .and_then(|bytes| {
                    serde_json::from_slice::<StoredSession>(&bytes)
                        .map_err(|_| "session_metadata_parse_failed".to_string())
                })
                .and_then(|session| {
                    if self.metadata_path_for_session(&session.session_id) == path {
                        Ok(session)
                    } else {
                        Err("session_metadata_id_mismatch".to_string())
                    }
                });
            match parsed {
                Ok(session) => sessions.push(session),
                Err(error)
                    if matches!(
                        error.as_str(),
                        "session_metadata_parse_failed" | "session_metadata_id_mismatch"
                    ) =>
                {
                    let suffix = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_nanos();
                    let backup = path.with_file_name(format!(
                        "session.json.session-metadata-corrupt-backup-{suffix}"
                    ));
                    fs::rename(&path, &backup)
                        .map_err(|_| "session_metadata_quarantine_failed".to_string())?;
                    restrict_session_path_permissions(&backup, false)?;
                    backup_paths.push(backup);
                }
                Err(error) => return Err(error),
            }
        }
        sessions.sort_by(|left, right| {
            right
                .updated_at_ms
                .cmp(&left.updated_at_ms)
                .then_with(|| left.session_id.cmp(&right.session_id))
        });
        Ok(SessionIndexRecovery {
            sessions,
            invalid_records: backup_paths.len(),
            backup_path: backup_paths.first().cloned(),
            backup_paths,
        })
    }

    fn list_metadata_sessions_unlocked(&self) -> Result<Vec<StoredSession>, String> {
        let mut sessions = Vec::new();
        let entries = match fs::read_dir(self.sessions_dir()) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(sessions),
            Err(_) => return Err("session_dir_read_failed".to_string()),
        };
        for entry in entries {
            let entry = entry.map_err(|_| "session_dir_read_failed".to_string())?;
            let file_type = entry
                .file_type()
                .map_err(|_| "session_dir_read_failed".to_string())?;
            if !file_type.is_dir() || entry.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            let path = entry.path().join("session.json");
            if !path.exists() {
                continue;
            }
            let bytes = fs::read(&path).map_err(|_| "session_metadata_read_failed".to_string())?;
            let session = serde_json::from_slice::<StoredSession>(&bytes)
                .map_err(|_| "session_metadata_parse_failed".to_string())?;
            if self.metadata_path_for_session(&session.session_id) != path {
                return Err("session_metadata_id_mismatch".to_string());
            }
            sessions.push(session);
        }
        sessions.sort_by(|left, right| {
            right
                .updated_at_ms
                .cmp(&left.updated_at_ms)
                .then_with(|| left.session_id.cmp(&right.session_id))
        });
        Ok(sessions)
    }

    fn list_legacy_sessions_unlocked(&self) -> Result<Vec<StoredSession>, String> {
        let path = self.index_path();
        if !path.exists() {
            return Ok(Vec::new());
        }
        let file = fs::File::open(path).map_err(|_| "session_index_open_failed")?;
        let mut reader = BufReader::new(file);
        let mut sessions = Vec::new();
        let mut session_ids = BTreeSet::new();
        while let Some(line) = read_bounded_jsonl_record(
            &mut reader,
            MAX_SESSION_INDEX_RECORD_BYTES,
            "session_index_read_failed",
        )? {
            if line.oversized || line.bytes.iter().all(|byte| byte.is_ascii_whitespace()) {
                if line.oversized {
                    return Err("session_record_parse_failed".to_string());
                }
                continue;
            }
            let session = serde_json::from_slice::<StoredSession>(&line.bytes)
                .map_err(|_| "session_record_parse_failed")?;
            if !session_ids.insert(session.session_id.clone()) {
                return Err("session_record_parse_failed".to_string());
            }
            sessions.push(session);
        }
        sessions.sort_by(|left, right| {
            right
                .updated_at_ms
                .cmp(&left.updated_at_ms)
                .then_with(|| left.session_id.cmp(&right.session_id))
        });
        Ok(sessions)
    }

    pub fn load_session(&self, session_id: &str) -> Result<Option<StoredSession>, String> {
        if self.layout_marker_path().exists() {
            validate_session_id(session_id)?;
            let path = self.metadata_path_for_session(session_id);
            return match fs::read(path) {
                Ok(bytes) => serde_json::from_slice(&bytes)
                    .map(Some)
                    .map_err(|_| "session_metadata_parse_failed".to_string()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(_) => Err("session_metadata_read_failed".to_string()),
            };
        }
        Ok(self
            .list_legacy_sessions_unlocked()?
            .into_iter()
            .find(|session| session.session_id == session_id))
    }

    pub fn delete_session(&self, session_id: &str) -> Result<(), String> {
        self.ensure_metadata_v2()?;
        validate_session_id(session_id)?;
        let history_path = self.history_path_for_session(session_id);
        let session_dir = history_path
            .parent()
            .ok_or_else(|| "session_data_path_invalid".to_string())?;
        let deleted_dir = self.sessions_dir().join(format!(
            ".deleted-{}-{}-{}",
            sanitize_session_path_component(session_id),
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let session_guard = MemGuard::for_memory_domain(
            &self.root,
            format!(
                "session-data-{}",
                sanitize_session_path_component(session_id)
            ),
        );
        let renamed = session_guard.with_write(|| {
            if !self.metadata_path_for_session(session_id).exists() {
                return Err("session_not_found".to_string());
            }
            self.history_indexes
                .lock()
                .map_err(|_| "chat_history_index_poisoned".to_string())?
                .remove(&history_path);
            fs::rename(session_dir, &deleted_dir)
                .map_err(|_| "session_data_remove_failed".to_string())?;
            Ok::<_, String>(true)
        })??;
        if renamed {
            fs::remove_dir_all(deleted_dir)
                .map_err(|_| "session_data_remove_failed".to_string())?;
        }
        Ok(())
    }

    fn write_sessions_unlocked(&self, sessions: &[StoredSession]) -> Result<(), String> {
        let index_path = self.index_path();
        let temporary = index_path.with_extension("jsonl.tmp");
        let mut options = OpenOptions::new();
        options.create(true).write(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let result = (|| {
            let mut file = options
                .open(&temporary)
                .map_err(|_| "session_index_open_failed")?;
            for session in sessions {
                let line = serde_json::to_string(session)
                    .map_err(|_| "session_record_serialize_failed")?;
                writeln!(file, "{line}").map_err(|_| "session_index_write_failed")?;
            }
            file.sync_all().map_err(|_| "session_index_sync_failed")?;
            fs::rename(&temporary, &index_path).map_err(|_| "session_index_replace_failed")?;
            restrict_session_path_permissions(&index_path, false)?;
            #[cfg(unix)]
            fs::File::open(self.sessions_dir())
                .and_then(|directory| directory.sync_all())
                .map_err(|_| "session_index_dir_sync_failed")?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }

    pub fn append_history_record(
        &self,
        session_id: &str,
        record: &ChatHistoryRecord,
    ) -> Result<(), String> {
        let path = self.history_path_for_session(session_id);
        let summary_path = temporary_event_summary_path(&path);
        let mut bytes = serde_json::to_vec(record)
            .map_err(|_| "chat_history_record_serialize_failed".to_string())?;
        bytes.push(b'\n');
        MemGuard::for_memory_domain(
            &self.root,
            format!(
                "session-data-{}",
                sanitize_session_path_component(session_id)
            ),
        )
        .with_write(|| {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(|_| "chat_history_dir_create_failed")?;
            }
            let before_len = fs::metadata(&path)
                .map(|metadata| metadata.len())
                .unwrap_or(0);
            let mut options = OpenOptions::new();
            options.create(true).append(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options
                .open(&path)
                .map_err(|_| "chat_history_open_failed")?;
            file.write_all(&bytes)
                .map_err(|_| "chat_history_write_failed".to_string())?;
            let metadata = file
                .metadata()
                .map_err(|_| "chat_history_open_failed".to_string())?;
            let file_len = metadata.len();
            let modified_at_ms = metadata
                .modified()
                .ok()
                .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
                .map(|value| value.as_millis());
            let mut indexes = self
                .history_indexes
                .lock()
                .map_err(|_| "chat_history_index_poisoned".to_string())?;
            if let Some(index) = indexes
                .get_mut(&path)
                .filter(|index| index.file_len == before_len)
            {
                index.entries.push(HistoryIndexEntry {
                    byte_offset: before_len,
                    byte_len: bytes.len() as u64,
                    turn_id: record.turn_id().to_string(),
                });
                index.file_len = file_len;
                index.modified_at_ms = modified_at_ms;
            } else {
                indexes.remove(&path);
            }
            drop(indexes);

            let temporary_time = temporary_event_time(record);
            let mut summaries = self
                .temporary_event_summaries
                .lock()
                .map_err(|_| "chat_history_summary_poisoned".to_string())?;
            let existing = summaries
                .get(&path)
                .copied()
                .or_else(|| read_temporary_event_summary(&summary_path));
            let summary =
                if let Some(mut summary) = existing.filter(|value| value.file_len == before_len) {
                    summary.file_len = file_len;
                    if let Some(created_at_ms) = temporary_time {
                        summary.min_created_at_ms = Some(
                            summary
                                .min_created_at_ms
                                .map_or(created_at_ms, |current| current.min(created_at_ms)),
                        );
                    }
                    summary
                } else if before_len == 0 {
                    TemporaryEventSummary {
                        file_len,
                        min_created_at_ms: temporary_time,
                    }
                } else {
                    // Unknown pre-existing history is repaired by the low-frequency
                    // retention pass; do not scan it on the append path.
                    summaries.remove(&path);
                    let _ = fs::remove_file(&summary_path);
                    return Ok::<(), String>(());
                };
            write_temporary_event_summary(&summary_path, summary)?;
            summaries.insert(path.clone(), summary);
            Ok(())
        })
        .map_err(|error| error.to_string())??;
        Ok(())
    }

    pub fn delete_history_message(
        &self,
        session_id: &str,
        turn_id: &str,
        role: ChatHistoryRole,
        role_index: usize,
    ) -> Result<ChatHistoryRecord, String> {
        let turn_id = turn_id.trim();
        if turn_id.is_empty() {
            return Err("turn_id_required".to_string());
        }
        MemGuard::for_memory_domain(
            &self.root,
            format!(
                "session-data-{}",
                sanitize_session_path_component(session_id)
            ),
        )
        .with_write(|| {
            self.delete_history_message_unlocked(session_id, turn_id, role, role_index)
        })?
    }

    fn delete_history_message_unlocked(
        &self,
        session_id: &str,
        turn_id: &str,
        role: ChatHistoryRole,
        role_index: usize,
    ) -> Result<ChatHistoryRecord, String> {
        let path = self.history_path_for_session(session_id);
        if !path.exists() {
            return Err("chat_message_not_found".to_string());
        }
        let parent = path
            .parent()
            .ok_or_else(|| "chat_history_path_invalid".to_string())?;
        let temporary = parent.join("raw_chat_history.jsonl.tmp");
        let source = fs::File::open(&path).map_err(|_| "chat_history_open_failed")?;
        let mut options = OpenOptions::new();
        options.create(true).write(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut target = options
            .open(&temporary)
            .map_err(|_| "chat_history_open_failed")?;
        let mut matching_index = 0usize;
        let mut deleted = None;
        for line in BufReader::new(source).lines() {
            let line = line.map_err(|_| "chat_history_read_failed")?;
            let parsed = parse_chat_history_record_line(&line);
            let matches_target = parsed.as_ref().is_some_and(|record| {
                matches!(
                    record,
                    ChatHistoryRecord::Message {
                        role: record_role,
                        turn_id: record_turn_id,
                        ..
                    } if *record_role == role && record_turn_id == turn_id
                )
            });
            if matches_target {
                if matching_index == role_index && deleted.is_none() {
                    deleted = parsed;
                    matching_index = matching_index.saturating_add(1);
                    continue;
                }
                matching_index = matching_index.saturating_add(1);
            }
            writeln!(target, "{line}").map_err(|_| "chat_history_write_failed".to_string())?;
        }
        let Some(deleted) = deleted else {
            let _ = fs::remove_file(&temporary);
            return Err("chat_message_not_found".to_string());
        };
        target
            .sync_all()
            .map_err(|_| "chat_history_sync_failed".to_string())?;
        fs::rename(&temporary, &path).map_err(|_| "chat_history_replace_failed".to_string())?;
        restrict_session_path_permissions(&path, false)?;
        #[cfg(unix)]
        fs::File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|_| "chat_history_dir_sync_failed".to_string())?;
        self.history_indexes
            .lock()
            .map_err(|_| "chat_history_index_poisoned")?
            .remove(&path);
        self.temporary_event_summaries
            .lock()
            .map_err(|_| "chat_history_summary_poisoned")?
            .remove(&path);
        let _ = fs::remove_file(temporary_event_summary_path(&path));
        Ok(deleted)
    }

    /// Removes temporary chat-history events older than `cutoff_ms` while preserving
    /// every user/assistant message, durable event kinds, and malformed lines. The
    /// replacement is installed atomically.
    pub fn prune_temporary_history_events_before(
        &self,
        session_id: &str,
        cutoff_ms: i64,
    ) -> Result<usize, String> {
        validate_session_id(session_id)?;
        let path = self.history_path_for_session(session_id);
        let summary_path = temporary_event_summary_path(&path);
        MemGuard::for_memory_domain(
            &self.root,
            format!(
                "session-data-{}",
                sanitize_session_path_component(session_id)
            ),
        )
        .with_write(|| {
            let file_len = fs::metadata(&path)
                .map(|metadata| metadata.len())
                .unwrap_or(0);
            let cached_summary = self
                .temporary_event_summaries
                .lock()
                .map_err(|_| "chat_history_summary_poisoned".to_string())?
                .get(&path)
                .copied();
            let summary = cached_summary.or_else(|| read_temporary_event_summary(&summary_path));
            if let Some(summary) = summary.filter(|summary| {
                summary.file_len == file_len
                    && summary
                        .min_created_at_ms
                        .is_none_or(|created_at_ms| created_at_ms >= cutoff_ms)
            }) {
                self.temporary_event_summaries
                    .lock()
                    .map_err(|_| "chat_history_summary_poisoned".to_string())?
                    .insert(path.clone(), summary);
                return Ok(0);
            }
            if !path.exists() {
                let summary = TemporaryEventSummary {
                    file_len: 0,
                    min_created_at_ms: None,
                };
                write_temporary_event_summary(&summary_path, summary)?;
                self.temporary_event_summaries
                    .lock()
                    .map_err(|_| "chat_history_summary_poisoned".to_string())?
                    .insert(path.clone(), summary);
                return Ok(0);
            }
            let parent = path
                .parent()
                .ok_or_else(|| "chat_history_path_invalid".to_string())?;
            let temporary = parent.join(format!(
                "raw_chat_history.jsonl.retention.tmp-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos()
            ));
            let source = fs::File::open(&path).map_err(|_| "chat_history_open_failed")?;
            let mut options = OpenOptions::new();
            options.create_new(true).write(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut target = options
                .open(&temporary)
                .map_err(|_| "chat_history_open_failed")?;
            let result: Result<(usize, Option<i64>, u64), String> = (|| {
                let mut removed = 0usize;
                let mut min_created_at_ms = None;
                for line in BufReader::new(source).lines() {
                    let line = line.map_err(|_| "chat_history_read_failed".to_string())?;
                    if let Some(created_at_ms) = parse_chat_history_record_line(&line)
                        .as_ref()
                        .and_then(temporary_event_time)
                    {
                        if created_at_ms < cutoff_ms {
                            removed = removed.saturating_add(1);
                            continue;
                        }
                        min_created_at_ms = Some(
                            min_created_at_ms
                                .map_or(created_at_ms, |current: i64| current.min(created_at_ms)),
                        );
                    }
                    writeln!(target, "{line}")
                        .map_err(|_| "chat_history_write_failed".to_string())?;
                }
                if removed == 0 {
                    drop(target);
                    fs::remove_file(&temporary)
                        .map_err(|_| "chat_history_temp_remove_failed".to_string())?;
                    return Ok((0, min_created_at_ms, file_len));
                }
                target
                    .sync_all()
                    .map_err(|_| "chat_history_sync_failed".to_string())?;
                fs::rename(&temporary, &path)
                    .map_err(|_| "chat_history_replace_failed".to_string())?;
                restrict_session_path_permissions(&path, false)?;
                #[cfg(unix)]
                fs::File::open(parent)
                    .and_then(|directory| directory.sync_all())
                    .map_err(|_| "chat_history_dir_sync_failed".to_string())?;
                let retained_len = fs::metadata(&path)
                    .map(|metadata| metadata.len())
                    .map_err(|_| "chat_history_open_failed".to_string())?;
                Ok((removed, min_created_at_ms, retained_len))
            })();
            if result.is_err() {
                let _ = fs::remove_file(&temporary);
            }
            let (removed, min_created_at_ms, retained_len) = result?;
            let summary = TemporaryEventSummary {
                file_len: retained_len,
                min_created_at_ms,
            };
            write_temporary_event_summary(&summary_path, summary)?;
            self.temporary_event_summaries
                .lock()
                .map_err(|_| "chat_history_summary_poisoned".to_string())?
                .insert(path.clone(), summary);
            if removed > 0 {
                self.history_indexes
                    .lock()
                    .map_err(|_| "chat_history_index_poisoned".to_string())?
                    .remove(&path);
            }
            Ok(removed)
        })
        .map_err(|error| error.to_string())?
    }

    /// Removes oldest complete contiguous Turn groups until at least
    /// `bytes_to_remove` bytes have been reclaimed. Malformed lines are retained.
    pub fn prune_oldest_history_turns(
        &self,
        session_id: &str,
        bytes_to_remove: u64,
    ) -> Result<u64, String> {
        validate_session_id(session_id)?;
        if bytes_to_remove == 0 {
            return Ok(0);
        }
        let path = self.history_path_for_session(session_id);
        let removed = MemGuard::for_memory_domain(
            &self.root,
            format!(
                "session-data-{}",
                sanitize_session_path_component(session_id)
            ),
        )
        .with_write(|| {
            if !path.exists() {
                return Ok::<u64, String>(0);
            }
            let bytes = fs::read(&path).map_err(|_| "chat_history_read_failed".to_string())?;
            let mut records: Vec<(Vec<u8>, Option<String>)> = Vec::new();
            let mut start = 0usize;
            for (index, byte) in bytes.iter().enumerate() {
                if *byte != b'\n' {
                    continue;
                }
                let raw = bytes[start..=index].to_vec();
                let text = std::str::from_utf8(&raw).ok().map(str::trim_end);
                let turn = text
                    .and_then(parse_chat_history_record_line)
                    .map(|record| record.turn_id().to_string());
                records.push((raw, turn));
                start = index + 1;
            }
            if start < bytes.len() {
                let raw = bytes[start..].to_vec();
                let turn = std::str::from_utf8(&raw)
                    .ok()
                    .and_then(parse_chat_history_record_line)
                    .map(|record| record.turn_id().to_string());
                records.push((raw, turn));
            }
            let mut groups: Vec<(usize, usize, u64, bool)> = Vec::new();
            let mut index = 0usize;
            while index < records.len() {
                let group_start = index;
                let turn = records[index].1.clone();
                index += 1;
                while index < records.len() && turn.is_some() && records[index].1 == turn {
                    index += 1;
                }
                let size = records[group_start..index]
                    .iter()
                    .map(|(raw, _)| raw.len() as u64)
                    .sum();
                groups.push((group_start, index, size, turn.is_some()));
            }
            let mut reclaimed = 0u64;
            let mut remove = BTreeSet::new();
            for (group_start, group_end, size, removable) in groups {
                if reclaimed >= bytes_to_remove {
                    break;
                }
                if !removable {
                    continue;
                }
                reclaimed = reclaimed.saturating_add(size);
                remove.extend(group_start..group_end);
            }
            if remove.is_empty() {
                return Ok(0);
            }
            let retained = records
                .into_iter()
                .enumerate()
                .filter_map(|(index, (raw, _))| (!remove.contains(&index)).then_some(raw))
                .collect::<Vec<_>>();
            let retained_bytes = retained.into_iter().flatten().collect::<Vec<_>>();
            atomic_write_file(&path, &retained_bytes)
                .map_err(|_| "chat_history_replace_failed".to_string())?;
            Ok(reclaimed)
        })??;
        self.history_indexes
            .lock()
            .map_err(|_| "chat_history_index_poisoned".to_string())?
            .remove(&path);
        self.temporary_event_summaries
            .lock()
            .map_err(|_| "chat_history_summary_poisoned".to_string())?
            .remove(&path);
        let _ = fs::remove_file(temporary_event_summary_path(&path));
        Ok(removed)
    }

    pub fn read_history_page(
        &self,
        session_id: &str,
        before_cursor: Option<&str>,
        turn_limit: usize,
    ) -> Result<ChatHistoryPage, String> {
        let path = self.history_path_for_session(session_id);
        let index = self.history_index_for_path(&path)?;
        read_history_page_from_index(&path, &index, before_cursor, turn_limit)
    }

    fn history_index_for_path(&self, path: &Path) -> Result<HistoryIndex, String> {
        if !path.exists() {
            return Ok(HistoryIndex {
                file_len: 0,
                modified_at_ms: None,
                entries: Vec::new(),
            });
        }
        let metadata = fs::metadata(path).map_err(|_| "chat_history_open_failed")?;
        let file_len = metadata.len();
        let modified_at_ms = metadata
            .modified()
            .ok()
            .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
            .map(|value| value.as_millis());
        if let Some(existing) = self
            .history_indexes
            .lock()
            .map_err(|_| "chat_history_index_poisoned")?
            .get(path)
            .filter(|index| index.file_len == file_len && index.modified_at_ms == modified_at_ms)
            .cloned()
        {
            return Ok(existing);
        }

        let index = build_history_index(path, file_len, modified_at_ms)?;
        self.history_indexes
            .lock()
            .map_err(|_| "chat_history_index_poisoned")?
            .insert(path.to_path_buf(), index.clone());
        Ok(index)
    }
}

fn temporary_event_summary_path(history_path: &Path) -> PathBuf {
    history_path.with_file_name("raw_chat_history.retention.json")
}

fn read_temporary_event_summary(path: &Path) -> Option<TemporaryEventSummary> {
    let bytes = fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn write_temporary_event_summary(
    path: &Path,
    summary: TemporaryEventSummary,
) -> Result<(), String> {
    let mut bytes = serde_json::to_vec(&summary)
        .map_err(|_| "chat_history_summary_serialize_failed".to_string())?;
    bytes.push(b'\n');
    atomic_write_file(path, &bytes).map_err(|_| "chat_history_summary_write_failed".to_string())
}

struct BoundedJsonlRecord {
    bytes: Vec<u8>,
    oversized: bool,
}

fn read_bounded_jsonl_record(
    reader: &mut impl BufRead,
    max_bytes: usize,
    read_error: &str,
) -> Result<Option<BoundedJsonlRecord>, String> {
    let mut bytes = Vec::new();
    let mut oversized = false;
    let mut saw_data = false;
    loop {
        let buffer = reader.fill_buf().map_err(|_| read_error.to_string())?;
        if buffer.is_empty() {
            return if saw_data {
                Ok(Some(BoundedJsonlRecord { bytes, oversized }))
            } else {
                Ok(None)
            };
        }
        saw_data = true;
        let consumed = buffer
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|index| index + 1)
            .unwrap_or(buffer.len());
        let content_len = if buffer.get(consumed.saturating_sub(1)) == Some(&b'\n') {
            consumed - 1
        } else {
            consumed
        };
        if !oversized {
            let remaining = max_bytes.saturating_sub(bytes.len());
            let keep = content_len.min(remaining);
            bytes.extend_from_slice(&buffer[..keep]);
            if keep < content_len {
                oversized = true;
            }
        }
        let finished = buffer.get(consumed.saturating_sub(1)) == Some(&b'\n');
        reader.consume(consumed);
        if finished {
            return Ok(Some(BoundedJsonlRecord { bytes, oversized }));
        }
    }
}

fn copy_file_synced(source: &Path, target: &Path, label: &str) -> Result<(), String> {
    let mut source_file = fs::File::open(source).map_err(|_| format!("{label}_read_failed"))?;
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut target_file = options
        .open(target)
        .map_err(|_| format!("{label}_open_failed"))?;
    std::io::copy(&mut source_file, &mut target_file)
        .and_then(|_| target_file.sync_all())
        .map_err(|_| format!("{label}_write_failed"))?;
    #[cfg(unix)]
    if let Some(parent) = target.parent() {
        fs::File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|_| format!("{label}_dir_sync_failed"))?;
    }
    Ok(())
}

#[cfg(unix)]
fn restrict_session_path_permissions(path: &Path, directory: bool) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;

    let mode = if directory { 0o700 } else { 0o600 };
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .map_err(|_| "session_permissions_update_failed".to_string())
}

#[cfg(not(unix))]
fn restrict_session_path_permissions(_path: &Path, _directory: bool) -> Result<(), String> {
    Ok(())
}

fn build_history_index(
    path: &Path,
    file_len: u64,
    modified_at_ms: Option<u128>,
) -> Result<HistoryIndex, String> {
    let file = fs::File::open(path).map_err(|_| "chat_history_open_failed")?;
    let mut reader = BufReader::new(file);
    let mut byte_offset = 0u64;
    let mut entries = Vec::new();
    let mut line = String::new();
    loop {
        line.clear();
        let byte_len = reader
            .read_line(&mut line)
            .map_err(|_| "chat_history_read_failed")?;
        if byte_len == 0 {
            break;
        }
        if let Some(record) = (!line.trim().is_empty())
            .then(|| parse_chat_history_record_line(&line))
            .flatten()
        {
            entries.push(HistoryIndexEntry {
                byte_offset,
                byte_len: byte_len as u64,
                turn_id: record.turn_id().to_string(),
            });
        }
        byte_offset = byte_offset.saturating_add(byte_len as u64);
    }
    Ok(HistoryIndex {
        file_len,
        modified_at_ms,
        entries,
    })
}

fn read_history_page_from_index(
    path: &Path,
    index: &HistoryIndex,
    before_cursor: Option<&str>,
    turn_limit: usize,
) -> Result<ChatHistoryPage, String> {
    let turn_limit = if turn_limit == 0 {
        DEFAULT_HISTORY_PAGE_LIMIT
    } else {
        turn_limit
    };
    let requested_end = before_cursor
        .map(|cursor| {
            cursor
                .parse::<usize>()
                .map_err(|_| "invalid_history_cursor")
        })
        .transpose()?;
    let end = requested_end
        .unwrap_or(index.entries.len())
        .min(index.entries.len());
    let start = page_start_index(&index.entries, end, turn_limit);
    if index.entries.is_empty() && !path.exists() {
        return Ok(ChatHistoryPage {
            records: Vec::new(),
            before_cursor: None,
            has_more: false,
        });
    }
    let mut file = fs::File::open(path).map_err(|_| "chat_history_open_failed")?;
    let mut records = Vec::with_capacity(end.saturating_sub(start));
    for entry in &index.entries[start..end] {
        file.seek(SeekFrom::Start(entry.byte_offset))
            .map_err(|_| "chat_history_read_failed")?;
        let mut bytes = vec![0; entry.byte_len as usize];
        file.read_exact(&mut bytes)
            .map_err(|_| "chat_history_read_failed")?;
        let line = String::from_utf8(bytes).map_err(|_| "chat_history_read_failed")?;
        if let Some(record) = parse_chat_history_record_line(&line) {
            records.push(record);
        }
    }
    Ok(ChatHistoryPage {
        records,
        before_cursor: (start > 0).then(|| start.to_string()),
        has_more: start > 0,
    })
}

fn page_start_index(entries: &[HistoryIndexEntry], end: usize, limit: usize) -> usize {
    let mut start = end;
    let mut turn_count = 0usize;
    while start > 0 && turn_count < limit {
        let turn_id = &entries[start - 1].turn_id;
        let mut turn_start = start - 1;
        while turn_start > 0 && entries[turn_start - 1].turn_id == *turn_id {
            turn_start -= 1;
        }
        start = turn_start;
        turn_count = turn_count.saturating_add(1);
    }
    start
}

pub fn read_history_page_from_path(
    path: &Path,
    before_cursor: Option<&str>,
    turn_limit: usize,
) -> Result<ChatHistoryPage, String> {
    let turn_limit = if turn_limit == 0 {
        DEFAULT_HISTORY_PAGE_LIMIT
    } else {
        turn_limit
    };
    if !path.exists() {
        return Ok(ChatHistoryPage {
            records: Vec::new(),
            before_cursor: None,
            has_more: false,
        });
    }
    let requested_end = before_cursor
        .map(|cursor| {
            cursor
                .parse::<usize>()
                .map_err(|_| "invalid_history_cursor")
        })
        .transpose()?;
    let file = fs::File::open(path).map_err(|_| "chat_history_open_failed")?;
    let mut page = VecDeque::<(usize, String, Vec<ChatHistoryRecord>)>::new();
    let mut logical_index = 0usize;
    for line in BufReader::new(file).lines() {
        let line = line.map_err(|_| "chat_history_read_failed")?;
        if line.trim().is_empty() {
            continue;
        }
        let Some(record) = parse_chat_history_record_line(&line) else {
            continue;
        };
        let within_window = requested_end.is_none_or(|end| logical_index < end);
        if within_window {
            let turn_id = record.turn_id().to_string();
            let extends_active_turn = page
                .back()
                .is_some_and(|(_, active_turn_id, _)| *active_turn_id == turn_id);
            if extends_active_turn {
                page.back_mut().unwrap().2.push(record);
            } else {
                page.push_back((logical_index, turn_id, vec![record]));
            }
            // The page limit counts complete turns, not JSONL records. A
            // complex turn may contain many action and result records but is
            // still one user-visible task in the history UI.
            while page.len() > turn_limit {
                page.pop_front();
            }
        }
        logical_index = logical_index.saturating_add(1);
    }
    let end = requested_end.unwrap_or(logical_index).min(logical_index);
    while page.front().is_some_and(|(index, _, _)| *index >= end) {
        page.pop_front();
    }
    let start = page.front().map(|(index, _, _)| *index).unwrap_or(end);
    Ok(ChatHistoryPage {
        records: page
            .into_iter()
            .flat_map(|(_, _, records)| records)
            .collect(),
        before_cursor: (start > 0).then(|| start.to_string()),
        has_more: start > 0,
    })
}

pub fn read_all_history_records(path: &Path) -> Result<Vec<ChatHistoryRecord>, String> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let file = fs::File::open(path).map_err(|_| "chat_history_open_failed")?;
    let mut records = Vec::new();
    for line in BufReader::new(file).lines() {
        let line = line.map_err(|_| "chat_history_read_failed")?;
        if line.trim().is_empty() {
            continue;
        }
        if let Some(record) = parse_chat_history_record_line(&line) {
            records.push(record);
        }
    }
    Ok(records)
}

fn parse_chat_history_record_line(line: &str) -> Option<ChatHistoryRecord> {
    serde_json::from_str::<ChatHistoryRecord>(line).ok()
}

pub fn chat_history_prompt_format_hint(path: &Path) -> String {
    let message = ChatHistoryRecord::Message {
        role: ChatHistoryRole::User,
        turn_id: "...".to_string(),
        created_at_ms: 123,
        kind: None,
        command_id: None,
        delivery_state: None,
        content: "...".to_string(),
    };
    let event = ChatHistoryRecord::Event {
        role: ChatHistoryRole::System,
        turn_id: "...".to_string(),
        created_at_ms: 123,
        kind: ChatHistoryEventKind::ActionResult,
        content: "...".to_string(),
        extra: BTreeMap::new(),
    };
    format!(
        "Refer to chat history when necessary:\npath: {}\nformat: JSONL, one record per line.\nrecord types:\n- {}\n- {}\nMessage records may include optional kind for user entries: task, supplement, or approval.\nAdditional event fields may appear depending on kind.",
        path.display(),
        serde_json::to_string(&message).expect("chat history message example serializes"),
        serde_json::to_string(&event).expect("chat history event example serializes")
    )
}

pub fn new_stored_session(
    session_id: impl Into<String>,
    display_name: impl Into<String>,
    current_dir: impl AsRef<Path>,
    profile: StoredSessionProfile,
    history_path: impl AsRef<Path>,
) -> StoredSession {
    let now = now_ms();
    StoredSession {
        session_id: session_id.into(),
        display_name: display_name.into(),
        created_at_ms: now,
        updated_at_ms: now,
        current_dir: current_dir.as_ref().display().to_string(),
        profile,
        env: BTreeMap::new(),
        env_overrides: Some(BTreeMap::new()),
        mcp_server_ids: Vec::new(),
        state: StoredSessionState::Ready,
        last_turn_id: None,
        raw_chat_history_path: history_path.as_ref().display().to_string(),
        group_id: None,
    }
}

fn validate_session_id(value: &str) -> Result<(), String> {
    let valid = !value.is_empty()
        && value.len() <= 160
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'));
    valid
        .then_some(())
        .ok_or_else(|| "session_id_invalid".to_string())
}

fn sanitize_session_path_component(value: &str) -> String {
    let sanitized = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    if sanitized.is_empty() {
        "session".to_string()
    } else {
        sanitized
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}
