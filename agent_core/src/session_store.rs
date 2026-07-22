use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::collections::VecDeque;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
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
    pub state: StoredSessionState,
    pub last_turn_id: Option<String>,
    pub raw_chat_history_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct StoredSessionProfile {
    pub provider: String,
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
}

impl SessionStore {
    pub fn new(root: impl AsRef<Path>) -> Self {
        Self {
            root: root.as_ref().to_path_buf(),
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
        fs::create_dir_all(self.sessions_dir()).map_err(|_| "session_dir_create_failed")?;
        let mut sessions = self.list_sessions()?;
        if let Some(existing) = sessions
            .iter_mut()
            .find(|existing| existing.session_id == session.session_id)
        {
            *existing = session.clone();
        } else {
            sessions.push(session.clone());
        }
        sessions.sort_by_key(|session| (session.updated_at_ms, session.session_id.clone()));
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(self.index_path())
            .map_err(|_| "session_index_open_failed")?;
        for session in sessions {
            let line =
                serde_json::to_string(&session).map_err(|_| "session_record_serialize_failed")?;
            writeln!(file, "{line}").map_err(|_| "session_index_write_failed")?;
        }
        Ok(())
    }

    pub fn list_sessions(&self) -> Result<Vec<StoredSession>, String> {
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
            .open(path)
            .map_err(|_| "chat_history_open_failed")?;
        let line =
            serde_json::to_string(record).map_err(|_| "chat_history_record_serialize_failed")?;
        writeln!(file, "{line}").map_err(|_| "chat_history_write_failed".to_string())
    }

    pub fn read_history_page(
        &self,
        session_id: &str,
        before_cursor: Option<&str>,
        limit: usize,
    ) -> Result<ChatHistoryPage, String> {
        read_history_page_from_path(
            &self.history_path_for_session(session_id),
            before_cursor,
            limit,
        )
    }
}

pub fn read_history_page_from_path(
    path: &Path,
    before_cursor: Option<&str>,
    limit: usize,
) -> Result<ChatHistoryPage, String> {
    let limit = if limit == 0 {
        DEFAULT_HISTORY_PAGE_LIMIT
    } else {
        limit
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
    let mut page_len = 0usize;
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
            page_len = page_len.saturating_add(1);
            // A restored page must never begin in the middle of a turn. Keep
            // the newest complete turns near the requested record budget; a
            // single very large turn is intentionally allowed to exceed it.
            while page_len > limit && page.len() > 1 {
                if let Some((_, _, removed)) = page.pop_front() {
                    page_len = page_len.saturating_sub(removed.len());
                }
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
