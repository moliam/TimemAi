use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub const MAX_WORKER_ROLES: usize = 32;
pub const MAX_ROLE_NAME_CHARS: usize = 80;
pub const MAX_ROLE_DESCRIPTION_BYTES: usize = 16 * 1024;
const MAX_ROLE_FILE_BYTES: u64 = 1024 * 1024;
static NEXT_TEMP_FILE: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkerRole {
    pub id: String,
    pub name: String,
    pub description: String,
}

pub fn roles_path_for_history(history_path: &Path) -> Result<PathBuf, String> {
    history_path
        .parent()
        .map(|directory| directory.join("worker_roles.json"))
        .ok_or_else(|| "worker_role_path_invalid".to_string())
}

pub fn normalize_role_fields(name: &str, description: &str) -> Result<(String, String), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("worker_role_name_required".to_string());
    }
    if name.chars().count() > MAX_ROLE_NAME_CHARS {
        return Err("worker_role_name_too_long".to_string());
    }
    if name.chars().any(char::is_control) {
        return Err("worker_role_name_contains_control_character".to_string());
    }
    let description = description.trim();
    if description.is_empty() {
        return Err("worker_role_description_required".to_string());
    }
    if description.len() > MAX_ROLE_DESCRIPTION_BYTES {
        return Err("worker_role_description_too_long".to_string());
    }
    if description.contains('\0') {
        return Err("worker_role_description_contains_nul".to_string());
    }
    Ok((name.to_string(), description.to_string()))
}

pub fn validate_role_id(id: &str) -> Result<(), String> {
    let valid = !id.is_empty()
        && id.len() <= 160
        && id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'));
    valid
        .then_some(())
        .ok_or_else(|| "worker_role_id_invalid".to_string())
}

pub fn validate_role_collection(roles: &[WorkerRole]) -> Result<(), String> {
    if roles.len() > MAX_WORKER_ROLES {
        return Err("worker_role_limit_reached".to_string());
    }
    let mut ids = std::collections::BTreeSet::new();
    let mut names = std::collections::BTreeSet::new();
    for role in roles {
        validate_role_id(&role.id)?;
        normalize_role_fields(&role.name, &role.description)?;
        if !ids.insert(role.id.as_str()) {
            return Err("worker_role_id_duplicate".to_string());
        }
        if !names.insert(role.name.to_lowercase()) {
            return Err("worker_role_name_duplicate".to_string());
        }
    }
    Ok(())
}

pub fn load_roles(path: &Path) -> Result<Vec<WorkerRole>, String> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(_) => return Err("worker_role_store_metadata_failed".to_string()),
    };
    if !metadata.is_file() || metadata.len() > MAX_ROLE_FILE_BYTES {
        return Err("worker_role_store_invalid".to_string());
    }
    let bytes = fs::read(path).map_err(|_| "worker_role_store_read_failed".to_string())?;
    let roles = serde_json::from_slice::<Vec<WorkerRole>>(&bytes)
        .map_err(|_| "worker_role_store_parse_failed".to_string())?;
    validate_role_collection(&roles)?;
    Ok(roles)
}

pub fn save_roles(path: &Path, roles: &[WorkerRole]) -> Result<(), String> {
    validate_role_collection(roles)?;
    let parent = path
        .parent()
        .ok_or_else(|| "worker_role_path_invalid".to_string())?;
    fs::create_dir_all(parent).map_err(|_| "worker_role_store_create_failed".to_string())?;
    let payload = serde_json::to_vec_pretty(roles)
        .map_err(|_| "worker_role_store_serialize_failed".to_string())?;
    if payload.len() as u64 > MAX_ROLE_FILE_BYTES {
        return Err("worker_role_store_too_large".to_string());
    }
    let temp_id = NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed);
    let temp_path = parent.join(format!(
        ".worker_roles.{}.{}.tmp",
        std::process::id(),
        temp_id
    ));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
            .map_err(|_| "worker_role_store_temp_create_failed".to_string())?;
        file.write_all(&payload)
            .and_then(|()| file.write_all(b"\n"))
            .map_err(|_| "worker_role_store_write_failed".to_string())?;
        file.sync_all()
            .map_err(|_| "worker_role_store_sync_failed".to_string())?;
        fs::rename(&temp_path, path).map_err(|_| "worker_role_store_replace_failed".to_string())?;
        if let Ok(directory) = fs::File::open(parent) {
            let _ = directory.sync_all();
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "timem_worker_roles_{label}_{}_{}.json",
            std::process::id(),
            NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn role_store_round_trips_and_replaces_atomically() {
        let path = temp_path("roundtrip");
        let first = vec![WorkerRole {
            id: "role_1".to_string(),
            name: "Reviewer".to_string(),
            description: "Inspect evidence before changing code.".to_string(),
        }];
        save_roles(&path, &first).unwrap();
        assert_eq!(load_roles(&path).unwrap(), first);
        let second = vec![WorkerRole {
            id: "role_2".to_string(),
            name: "Builder".to_string(),
            description: "Implement and test the requested behavior.".to_string(),
        }];
        save_roles(&path, &second).unwrap();
        assert_eq!(load_roles(&path).unwrap(), second);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn role_validation_rejects_unsafe_and_duplicate_values() {
        assert_eq!(
            normalize_role_fields("", "description").unwrap_err(),
            "worker_role_name_required"
        );
        assert_eq!(
            normalize_role_fields("bad\nname", "description").unwrap_err(),
            "worker_role_name_contains_control_character"
        );
        let duplicate = vec![
            WorkerRole {
                id: "role_1".to_string(),
                name: "Review".to_string(),
                description: "One".to_string(),
            },
            WorkerRole {
                id: "role_2".to_string(),
                name: "review".to_string(),
                description: "Two".to_string(),
            },
        ];
        assert_eq!(
            validate_role_collection(&duplicate).unwrap_err(),
            "worker_role_name_duplicate"
        );
    }
}
