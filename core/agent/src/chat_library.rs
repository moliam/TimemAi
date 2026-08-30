use crate::rolling_file_store::{
    read_segmented_records, rewrite_segmented_records, segmented_directory, RollingCapacity,
    DEFAULT_ROLLING_SLICE_BYTES,
};
use crate::session_store::{
    read_all_history_records, ChatHistoryRecord, ChatHistoryRole, SessionStore,
};
use crate::{atomic_write_file, MemGuard};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_QUERY_CHARS: usize = 256;
const MAX_RESULTS: usize = 200;
pub const FAVORITES_LIMIT_256_MB: u64 = 256 * 1024 * 1024;
pub const FAVORITES_LIMIT_1_GB: u64 = 1024 * 1024 * 1024;
const CAPACITY_WARNING_PERCENT: u8 = 90;
static NEXT_LIBRARY_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatSearchHit {
    pub source_key: String,
    pub session_id: String,
    pub session_display_name: String,
    pub turn_id: String,
    pub role: ChatHistoryRole,
    pub content: String,
    pub created_at_ms: i64,
    pub favorite_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatFavorite {
    pub id: String,
    pub source_key: String,
    pub session_id: String,
    pub session_display_name: String,
    pub turn_id: String,
    pub content_snapshot: String,
    pub title: String,
    pub source_created_at_ms: i64,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub version: u64,
    #[serde(default)]
    pub deleted: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatLibraryCapacity {
    pub used_bytes: u64,
    pub limit_bytes: Option<u64>,
    pub used_percent: Option<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatLibrarySettings {
    pub max_bytes: Option<u64>,
}

impl Default for ChatLibrarySettings {
    fn default() -> Self {
        Self {
            max_bytes: Some(FAVORITES_LIMIT_256_MB),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CreateFavoriteOutcome {
    Created {
        favorite: Box<ChatFavorite>,
        capacity: ChatLibraryCapacity,
        nearing_limit: bool,
    },
    CapacityReached(ChatLibraryCapacity),
}

#[derive(Debug, Clone)]
pub struct ChatLibrary {
    root: PathBuf,
    guard: MemGuard,
}

impl ChatLibrary {
    pub fn new(memory_dir: impl AsRef<Path>) -> Self {
        let memory_dir = memory_dir.as_ref();
        Self {
            root: memory_dir.join("chat_library"),
            guard: MemGuard::for_memory_domain(memory_dir, "chat-library"),
        }
    }

    pub fn favorites_path(&self) -> PathBuf {
        self.root.join("favorites.jsonl")
    }

    pub fn settings_path(&self) -> PathBuf {
        self.root.join("settings.json")
    }

    pub fn capacity(&self) -> Result<ChatLibraryCapacity, String> {
        capacity_for(
            &self.favorites_path(),
            &load_settings(&self.settings_path())?,
        )
    }

    pub fn update_capacity_limit(
        &self,
        max_bytes: Option<u64>,
    ) -> Result<ChatLibraryCapacity, String> {
        validate_capacity_limit(max_bytes)?;
        self.guard.with_write(|| {
            let settings = ChatLibrarySettings { max_bytes };
            if max_bytes.is_some() {
                let current = latest_favorites(&self.favorites_path())?;
                rewrite_active_favorites(
                    &self.favorites_path(),
                    current.into_values().filter(|item| !item.deleted).collect(),
                    &settings,
                )?;
            }
            save_settings(&self.settings_path(), &settings)?;
            capacity_for(&self.favorites_path(), &settings)
        })?
    }

    pub fn list_favorites(&self) -> Result<Vec<ChatFavorite>, String> {
        let mut values = latest_favorites(&self.favorites_path())?
            .into_values()
            .filter(|item| !item.deleted)
            .collect::<Vec<_>>();
        values.sort_by(|a, b| {
            b.updated_at_ms
                .cmp(&a.updated_at_ms)
                .then_with(|| b.id.cmp(&a.id))
        });
        Ok(values)
    }

    pub fn create_favorite(
        &self,
        store: &SessionStore,
        session_id: &str,
        turn_id: &str,
    ) -> Result<CreateFavoriteOutcome, String> {
        let session = store
            .load_session(session_id)?
            .ok_or_else(|| "session_not_found".to_string())?;
        let records = read_all_history_records(&store.history_path_for_session(session_id))?;
        let (content, source_created_at_ms) = records
            .iter()
            .find_map(|record| match record {
                ChatHistoryRecord::Message {
                    role: ChatHistoryRole::Assistant,
                    turn_id: record_turn_id,
                    created_at_ms,
                    content,
                    ..
                } if record_turn_id == turn_id => Some((content.clone(), *created_at_ms)),
                _ => None,
            })
            .ok_or_else(|| "assistant_answer_not_found".to_string())?;
        let source_key = source_key(session_id, turn_id, ChatHistoryRole::Assistant, 0);
        self.guard.with_write(|| {
            let current = latest_favorites(&self.favorites_path())?;
            if let Some(existing) = current
                .values()
                .find(|item| item.source_key == source_key && !item.deleted)
            {
                let capacity = capacity_for(
                    &self.favorites_path(),
                    &load_settings(&self.settings_path())?,
                )?;
                return Ok(CreateFavoriteOutcome::Created {
                    favorite: Box::new(existing.clone()),
                    capacity,
                    nearing_limit: capacity_is_nearing_limit(capacity),
                });
            }
            let settings = load_settings(&self.settings_path())?;
            let now = now_ms();
            let favorite = ChatFavorite {
                id: unique_id("favorite"),
                source_key,
                session_id: session_id.to_string(),
                session_display_name: session.display_name,
                turn_id: turn_id.to_string(),
                title: default_title(&content, session_id),
                content_snapshot: content,
                source_created_at_ms,
                created_at_ms: now,
                updated_at_ms: now,
                version: 1,
                deleted: false,
            };
            let record_bytes = serialized_favorite_bytes(&favorite)?.len() as u64;
            let current_capacity = capacity_for(&self.favorites_path(), &settings)?;
            if settings.max_bytes.is_some_and(|total| {
                RollingCapacity::from_total_bytes(total)
                    .is_ok_and(|capacity| record_bytes > capacity.stable_bytes)
            }) {
                return Ok(CreateFavoriteOutcome::CapacityReached(current_capacity));
            }
            let mut active = current
                .into_values()
                .filter(|item| !item.deleted)
                .collect::<Vec<_>>();
            active.push(favorite.clone());
            rewrite_active_favorites(&self.favorites_path(), active, &settings)?;
            let capacity = capacity_for(&self.favorites_path(), &settings)?;
            Ok(CreateFavoriteOutcome::Created {
                favorite: Box::new(favorite),
                capacity,
                nearing_limit: capacity_is_nearing_limit(capacity),
            })
        })?
    }

    pub fn delete_favorite(&self, favorite_id: &str) -> Result<ChatFavorite, String> {
        self.guard.with_write(|| {
            let current = latest_favorites(&self.favorites_path())?;
            let mut favorite = current
                .get(favorite_id)
                .filter(|item| !item.deleted)
                .cloned()
                .ok_or_else(|| "favorite_not_found".to_string())?;
            favorite.deleted = true;
            favorite.version = favorite.version.saturating_add(1);
            favorite.updated_at_ms = now_ms();
            let mut retained = current
                .into_values()
                .filter(|item| !item.deleted && item.id != favorite_id)
                .collect::<Vec<_>>();
            retained.sort_by(|left, right| left.id.cmp(&right.id));
            rewrite_favorites(&self.favorites_path(), &retained)?;
            Ok(favorite)
        })?
    }

    pub fn search(
        &self,
        store: &SessionStore,
        query: &str,
        session_scope: Option<&str>,
        limit: usize,
    ) -> Result<Vec<ChatSearchHit>, String> {
        let query = query.trim();
        if query.is_empty() {
            return Ok(Vec::new());
        }
        if query.chars().count() > MAX_QUERY_CHARS {
            return Err("chat_search_query_too_long".to_string());
        }
        let terms = query
            .split_whitespace()
            .map(|term| term.to_lowercase())
            .collect::<Vec<_>>();
        let favorites = latest_favorites(&self.favorites_path())?;
        let favorite_by_source = favorites
            .values()
            .filter(|item| !item.deleted)
            .map(|item| (item.source_key.as_str(), item.id.as_str()))
            .collect::<BTreeMap<_, _>>();
        let mut hits = Vec::new();
        for session in store.list_sessions()? {
            if session_scope.is_some_and(|scope| scope != session.session_id) {
                continue;
            }
            let records =
                read_all_history_records(&store.history_path_for_session(&session.session_id))?;
            let mut indexes = BTreeMap::<(String, &str), usize>::new();
            for record in records {
                let ChatHistoryRecord::Message {
                    role,
                    turn_id,
                    created_at_ms,
                    content,
                    ..
                } = record
                else {
                    continue;
                };
                if !matches!(role, ChatHistoryRole::User | ChatHistoryRole::Assistant) {
                    continue;
                }
                let role_key = match role {
                    ChatHistoryRole::User => "user",
                    ChatHistoryRole::Assistant => "assistant",
                    ChatHistoryRole::System => "system",
                };
                let index = indexes.entry((turn_id.clone(), role_key)).or_default();
                let current_index = *index;
                *index = index.saturating_add(1);
                let normalized = content.to_lowercase();
                if !terms.iter().all(|term| normalized.contains(term)) {
                    continue;
                }
                let key = source_key(&session.session_id, &turn_id, role, current_index);
                hits.push(ChatSearchHit {
                    favorite_id: favorite_by_source
                        .get(key.as_str())
                        .map(|id| (*id).to_string()),
                    source_key: key,
                    session_id: session.session_id.clone(),
                    session_display_name: session.display_name.clone(),
                    turn_id,
                    role,
                    content,
                    created_at_ms,
                });
            }
        }
        hits.sort_by_key(|hit| std::cmp::Reverse(hit.created_at_ms));
        hits.truncate(limit.clamp(1, MAX_RESULTS));
        Ok(hits)
    }
}

pub fn source_key(
    session_id: &str,
    turn_id: &str,
    role: ChatHistoryRole,
    role_index: usize,
) -> String {
    let role = match role {
        ChatHistoryRole::User => "user",
        ChatHistoryRole::Assistant => "assistant",
        ChatHistoryRole::System => "system",
    };
    format!("legacy:{session_id}:{turn_id}:{role}:{role_index}")
}

fn latest_favorites(path: &Path) -> Result<BTreeMap<String, ChatFavorite>, String> {
    let records =
        read_segmented_records(path).map_err(|_| "favorite_store_read_failed".to_string())?;
    let mut latest = BTreeMap::new();
    for record in records {
        if record.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let Ok(item) = serde_json::from_slice::<ChatFavorite>(&record) else {
            continue;
        };
        if latest
            .get(&item.id)
            .is_none_or(|old: &ChatFavorite| item.version > old.version)
        {
            latest.insert(item.id.clone(), item);
        }
    }
    Ok(latest)
}

fn serialized_favorite_bytes(favorite: &ChatFavorite) -> Result<Vec<u8>, String> {
    let mut line =
        serde_json::to_vec(favorite).map_err(|_| "favorite_store_serialize_failed".to_string())?;
    line.push(b'\n');
    Ok(line)
}

fn capacity_for(
    path: &Path,
    settings: &ChatLibrarySettings,
) -> Result<ChatLibraryCapacity, String> {
    let directory = segmented_directory(path);
    let used_bytes = if directory.exists() {
        fs::read_dir(directory)
            .map_err(|_| "favorite_store_read_failed".to_string())?
            .filter_map(Result::ok)
            .filter_map(|entry| entry.metadata().ok())
            .filter(|metadata| metadata.is_file())
            .map(|metadata| metadata.len())
            .sum()
    } else if path.exists() {
        fs::metadata(path)
            .map_err(|_| "favorite_store_read_failed".to_string())?
            .len()
    } else {
        0
    };
    Ok(capacity_for_used(used_bytes, settings.max_bytes))
}

fn capacity_for_used(used_bytes: u64, limit_bytes: Option<u64>) -> ChatLibraryCapacity {
    let used_percent = limit_bytes.map(|limit| {
        let percent = (u128::from(used_bytes) * 100).div_ceil(u128::from(limit));
        percent.min(100) as u8
    });
    ChatLibraryCapacity {
        used_bytes,
        limit_bytes,
        used_percent,
    }
}

fn capacity_is_nearing_limit(capacity: ChatLibraryCapacity) -> bool {
    capacity.limit_bytes.is_some_and(|limit| {
        u128::from(capacity.used_bytes) * 100
            > u128::from(limit) * u128::from(CAPACITY_WARNING_PERCENT)
    })
}

#[cfg(test)]
fn capacity_exceeds_limit(capacity: ChatLibraryCapacity) -> bool {
    capacity
        .limit_bytes
        .is_some_and(|limit| capacity.used_bytes > limit)
}

fn validate_capacity_limit(max_bytes: Option<u64>) -> Result<(), String> {
    if matches!(
        max_bytes,
        Some(FAVORITES_LIMIT_256_MB | FAVORITES_LIMIT_1_GB)
    ) || max_bytes.is_none()
    {
        Ok(())
    } else {
        Err("favorite_capacity_limit_invalid".to_string())
    }
}

fn load_settings(path: &Path) -> Result<ChatLibrarySettings, String> {
    if !path.exists() {
        return Ok(ChatLibrarySettings::default());
    }
    let raw = fs::read(path).map_err(|_| "favorite_settings_read_failed".to_string())?;
    let settings = serde_json::from_slice::<ChatLibrarySettings>(&raw)
        .map_err(|_| "favorite_settings_invalid".to_string())?;
    validate_capacity_limit(settings.max_bytes)?;
    Ok(settings)
}

fn save_settings(path: &Path, settings: &ChatLibrarySettings) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|_| "favorite_store_dir_failed".to_string())?;
    }
    let payload = serde_json::to_vec_pretty(settings)
        .map_err(|_| "favorite_settings_serialize_failed".to_string())?;
    atomic_write_file(path, &payload).map_err(|_| "favorite_settings_write_failed".to_string())
}

fn rewrite_favorites(path: &Path, favorites: &[ChatFavorite]) -> Result<(), String> {
    let mut payload = Vec::new();
    for favorite in favorites {
        serde_json::to_writer(&mut payload, favorite)
            .map_err(|_| "favorite_store_serialize_failed".to_string())?;
        payload.push(b'\n');
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|_| "favorite_store_dir_failed".to_string())?;
    }
    atomic_write_file(path, &payload).map_err(|_| "favorite_store_write_failed".to_string())?;
    let segmented = segmented_directory(path);
    if segmented.exists() {
        fs::remove_dir_all(segmented).map_err(|_| "favorite_store_write_failed".to_string())?;
    }
    Ok(())
}

fn rewrite_active_favorites(
    path: &Path,
    mut favorites: Vec<ChatFavorite>,
    settings: &ChatLibrarySettings,
) -> Result<(), String> {
    favorites.sort_by(|left, right| {
        left.created_at_ms
            .cmp(&right.created_at_ms)
            .then_with(|| left.id.cmp(&right.id))
    });
    if let Some(total_bytes) = settings.max_bytes {
        let capacity = RollingCapacity::from_total_bytes(total_bytes)
            .map_err(|_| "favorite_capacity_limit_invalid".to_string())?;
        let records = favorites
            .iter()
            .map(serialized_favorite_bytes)
            .collect::<Result<Vec<_>, _>>()?;
        rewrite_segmented_records(path, &records, capacity, DEFAULT_ROLLING_SLICE_BYTES).map_err(
            |error| match error.to_string().as_str() {
                "rolling_record_exceeds_capacity" | "rolling_record_exceeds_slice" => {
                    "favorite_capacity_reached".to_string()
                }
                _ => "favorite_store_write_failed".to_string(),
            },
        )?;
        return Ok(());
    }
    rewrite_favorites(path, &favorites)
}

fn default_title(content: &str, session_id: &str) -> String {
    let first = content
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("");
    let cleaned = first.trim_start_matches(['#', '*', '-', '>', '`', ' ']);
    let title = cleaned.chars().take(80).collect::<String>();
    if title.is_empty() {
        format!("来自 {session_id} 的回复")
    } else {
        title
    }
}
fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}
fn unique_id(prefix: &str) -> String {
    format!(
        "{prefix}-{}-{}",
        now_ms(),
        NEXT_LIBRARY_ID.fetch_add(1, Ordering::Relaxed)
    )
}

#[cfg(test)]
#[path = "../tests/unit/chat_library_tests.rs"]
mod tests;
