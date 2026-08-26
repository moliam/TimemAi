use agent_core::MemGuard;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

pub const MAX_SESSION_GROUPS: usize = 64;
pub const MAX_SESSION_GROUP_NAME_CHARS: usize = 80;
const MAX_SESSION_GROUP_ID_BYTES: usize = 160;
const MAX_SESSION_GROUP_FILE_BYTES: u64 = 512 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionGroup {
    pub id: String,
    pub name: String,
}

pub fn session_groups_path(memory_dir: &Path) -> PathBuf {
    memory_dir.join("session_groups.json")
}

pub fn normalize_session_group_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("session_group_name_required".into());
    }
    if name.chars().count() > MAX_SESSION_GROUP_NAME_CHARS {
        return Err("session_group_name_too_long".into());
    }
    if name.chars().any(char::is_control) {
        return Err("session_group_name_contains_control_character".into());
    }
    Ok(name.to_string())
}

pub fn validate_session_group_id(id: &str) -> Result<(), String> {
    let valid = !id.is_empty()
        && id.len() <= MAX_SESSION_GROUP_ID_BYTES
        && id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'));
    valid
        .then_some(())
        .ok_or_else(|| "session_group_id_invalid".into())
}

pub fn validate_session_groups(groups: &[SessionGroup]) -> Result<(), String> {
    if groups.len() > MAX_SESSION_GROUPS {
        return Err("session_group_limit_reached".into());
    }
    let mut ids = BTreeSet::new();
    let mut names = BTreeSet::new();
    for group in groups {
        validate_session_group_id(&group.id)?;
        normalize_session_group_name(&group.name)?;
        if !ids.insert(group.id.as_str()) {
            return Err("session_group_id_duplicate".into());
        }
        if !names.insert(group.name.to_lowercase()) {
            return Err("session_group_name_duplicate".into());
        }
    }
    Ok(())
}

pub fn load_session_groups(memory_dir: &Path) -> Result<Vec<SessionGroup>, String> {
    let path = session_groups_path(memory_dir);
    let metadata = match fs::metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(_) => return Err("session_group_store_metadata_failed".into()),
    };
    if !metadata.is_file() || metadata.len() > MAX_SESSION_GROUP_FILE_BYTES {
        return Err("session_group_store_invalid".into());
    }
    let bytes = fs::read(path).map_err(|_| "session_group_store_read_failed".to_string())?;
    let groups = serde_json::from_slice::<Vec<SessionGroup>>(&bytes)
        .map_err(|_| "session_group_store_parse_failed".to_string())?;
    validate_session_groups(&groups)?;
    Ok(groups)
}

pub fn save_session_groups(memory_dir: &Path, groups: &[SessionGroup]) -> Result<(), String> {
    validate_session_groups(groups)?;
    let path = session_groups_path(memory_dir);
    let guard = MemGuard::for_memory_domain(memory_dir, "session-groups");
    guard.with_write(|| {
        let mut payload = serde_json::to_vec_pretty(groups)
            .map_err(|_| "session_group_store_serialize_failed".to_string())?;
        payload.push(b'\n');
        agent_core::atomic_write_file(&path, &payload)
            .map_err(|_| "session_group_store_write_failed".to_string())
    })?
}
