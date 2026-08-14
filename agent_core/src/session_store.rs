use crate::MemGuard;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::collections::VecDeque;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

const DEFAULT_HISTORY_PAGE_LIMIT: usize = 200;

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
    RuntimeNotice,
    Stats,
    Attachment,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatHistoryPage {
    pub records: Vec<ChatHistoryRecord>,
    pub before_cursor: Option<String>,
    pub has_more: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionResumeNotice {
    pub history_path: PathBuf,
    pub current_dir: PathBuf,
}

impl SessionResumeNotice {
    pub fn render(&self) -> String {
        format!(
            "## SYSTEM\n\nThis session was restored and may not include the full previous context.\n\n{}\n\nDo not assume the whole previous context is loaded. Read this file only when needed for the current task.\nTry to use efficient tools such as tail, rg, jq, or short scripts instead of a huge cat.\n\nCurrent cwd: {}",
            chat_history_prompt_format_hint(&self.history_path),
            self.current_dir.display()
        )
    }
}

#[derive(Debug, Clone)]
pub struct SessionStore {
    root: PathBuf,
    history_indexes: Arc<Mutex<BTreeMap<PathBuf, HistoryIndex>>>,
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

impl SessionStore {
    pub fn new(root: impl AsRef<Path>) -> Self {
        let root = root.as_ref().to_path_buf();
        Self {
            guard: MemGuard::for_memory_dir(&root),
            root,
            history_indexes: Arc::new(Mutex::new(BTreeMap::new())),
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

    pub fn history_path_for_session(&self, session_id: &str) -> PathBuf {
        self.sessions_dir()
            .join(sanitize_session_path_component(session_id))
            .join("raw_chat_history.jsonl")
    }

    pub fn upsert_session(&self, session: &StoredSession) -> Result<(), String> {
        let _index_lock = self
            .index_lock
            .lock()
            .map_err(|_| "session_index_lock_poisoned".to_string())?;
        self.guard.with_write(|| {
            fs::create_dir_all(self.sessions_dir()).map_err(|_| "session_dir_create_failed")?;
            restrict_session_path_permissions(&self.sessions_dir(), true)?;
            let mut sessions = self.list_sessions_unlocked()?;
            if let Some(existing) = sessions
                .iter_mut()
                .find(|existing| existing.session_id == session.session_id)
            {
                *existing = session.clone();
            } else {
                sessions.push(session.clone());
            }
            sessions.sort_by_key(|session| (session.updated_at_ms, session.session_id.clone()));
            self.write_sessions_unlocked(&sessions)
        })?
    }

    pub fn list_sessions(&self) -> Result<Vec<StoredSession>, String> {
        let _index_lock = self
            .index_lock
            .lock()
            .map_err(|_| "session_index_lock_poisoned".to_string())?;
        self.guard.with_read(|| self.list_sessions_unlocked())?
    }

    fn list_sessions_unlocked(&self) -> Result<Vec<StoredSession>, String> {
        let path = self.index_path();
        if !path.exists() {
            return Ok(Vec::new());
        }
        let file = fs::File::open(path).map_err(|_| "session_index_open_failed")?;
        let mut sessions = Vec::new();
        for line in BufReader::new(file).lines() {
            let line = line.map_err(|_| "session_index_read_failed")?;
            if line.trim().is_empty() {
                continue;
            }
            sessions.push(
                serde_json::from_str::<StoredSession>(&line)
                    .map_err(|_| "session_record_parse_failed")?,
            );
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
        Ok(self
            .list_sessions()?
            .into_iter()
            .find(|session| session.session_id == session_id))
    }

    pub fn delete_session(&self, session_id: &str) -> Result<(), String> {
        let _index_lock = self
            .index_lock
            .lock()
            .map_err(|_| "session_index_lock_poisoned".to_string())?;
        self.guard.with_write(|| {
            let mut sessions = self.list_sessions_unlocked()?;
            let original_len = sessions.len();
            sessions.retain(|session| session.session_id != session_id);
            if sessions.len() == original_len {
                return Err("session_not_found".to_string());
            }
            self.write_sessions_unlocked(&sessions)?;
            let history_path = self.history_path_for_session(session_id);
            self.history_indexes
                .lock()
                .map_err(|_| "chat_history_index_poisoned")?
                .remove(&history_path);
            let session_dir = history_path
                .parent()
                .ok_or_else(|| "session_data_path_invalid".to_string())?;
            if session_dir.exists() {
                fs::remove_dir_all(session_dir).map_err(|_| "session_data_remove_failed")?;
            }
            Ok(())
        })?
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
        let mut file = options
            .open(&temporary)
            .map_err(|_| "session_index_open_failed")?;
        for session in sessions {
            let line =
                serde_json::to_string(session).map_err(|_| "session_record_serialize_failed")?;
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
    }

    pub fn append_history_record(
        &self,
        session_id: &str,
        record: &ChatHistoryRecord,
    ) -> Result<(), String> {
        let path = self.history_path_for_session(session_id);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|_| "chat_history_dir_create_failed")?;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|_| "chat_history_open_failed")?;
        let line =
            serde_json::to_string(record).map_err(|_| "chat_history_record_serialize_failed")?;
        writeln!(file, "{line}").map_err(|_| "chat_history_write_failed".to_string())?;
        self.history_indexes
            .lock()
            .map_err(|_| "chat_history_index_poisoned")?
            .remove(&path);
        Ok(())
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
    }
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
