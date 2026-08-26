use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub const MAX_WORKER_ROLES: usize = 128;
pub const MAX_WORKER_ROLE_GROUPS: usize = 64;
pub const MAX_ROLE_NAME_CHARS: usize = 80;
pub const MAX_ROLE_GROUP_NAME_CHARS: usize = 80;
pub const MAX_ROLE_DESCRIPTION_BYTES: usize = 16 * 1024;
pub const MAX_ROLE_FILE_BYTES: u64 = 4 * 1024 * 1024;
static NEXT_TEMP_FILE: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkerRole {
    pub id: String,
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkerRoleGroup {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub role_ids: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkerRoleLibrary {
    #[serde(default)]
    pub roles: Vec<WorkerRole>,
    #[serde(default)]
    pub groups: Vec<WorkerRoleGroup>,
}

pub fn role_library_path(memory_dir: &Path) -> PathBuf {
    memory_dir.join("worker_roles.json")
}

/// Legacy per-Session location, retained only for one-time migration.
pub fn roles_path_for_history(history_path: &Path) -> Result<PathBuf, String> {
    history_path
        .parent()
        .map(|directory| directory.join("worker_roles.json"))
        .ok_or_else(|| "worker_role_path_invalid".to_string())
}

pub fn normalize_role_fields(name: &str, description: &str) -> Result<(String, String), String> {
    let name = normalize_short_name(
        name,
        MAX_ROLE_NAME_CHARS,
        "worker_role_name_required",
        "worker_role_name_too_long",
        "worker_role_name_contains_control_character",
    )?;
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
    Ok((name, description.to_string()))
}

pub fn normalize_group_name(name: &str) -> Result<String, String> {
    normalize_short_name(
        name,
        MAX_ROLE_GROUP_NAME_CHARS,
        "worker_role_group_name_required",
        "worker_role_group_name_too_long",
        "worker_role_group_name_contains_control_character",
    )
}

fn normalize_short_name(
    value: &str,
    max_chars: usize,
    required_error: &str,
    too_long_error: &str,
    control_error: &str,
) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(required_error.to_string());
    }
    if value.chars().count() > max_chars {
        return Err(too_long_error.to_string());
    }
    if value.chars().any(char::is_control) {
        return Err(control_error.to_string());
    }
    Ok(value.to_string())
}

pub fn validate_role_id(id: &str) -> Result<(), String> {
    validate_identifier(id, "worker_role_id_invalid")
}

pub fn validate_group_id(id: &str) -> Result<(), String> {
    validate_identifier(id, "worker_role_group_id_invalid")
}

fn validate_identifier(id: &str, error: &str) -> Result<(), String> {
    let valid = !id.is_empty()
        && id.len() <= 160
        && id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'));
    valid.then_some(()).ok_or_else(|| error.to_string())
}

pub fn validate_role_collection(roles: &[WorkerRole]) -> Result<(), String> {
    validate_role_library(&WorkerRoleLibrary {
        roles: roles.to_vec(),
        groups: Vec::new(),
    })
}

pub fn validate_role_library(library: &WorkerRoleLibrary) -> Result<(), String> {
    if library.roles.len() > MAX_WORKER_ROLES {
        return Err("worker_role_limit_reached".to_string());
    }
    if library.groups.len() > MAX_WORKER_ROLE_GROUPS {
        return Err("worker_role_group_limit_reached".to_string());
    }

    let mut role_ids = BTreeSet::new();
    let mut role_names = BTreeSet::new();
    for role in &library.roles {
        validate_role_id(&role.id)?;
        normalize_role_fields(&role.name, &role.description)?;
        if !role_ids.insert(role.id.as_str()) {
            return Err("worker_role_id_duplicate".to_string());
        }
        if !role_names.insert(role.name.to_lowercase()) {
            return Err("worker_role_name_duplicate".to_string());
        }
    }

    let mut group_ids = BTreeSet::new();
    let mut group_names = BTreeSet::new();
    let mut placed_role_ids = BTreeSet::new();
    for group in &library.groups {
        validate_group_id(&group.id)?;
        normalize_group_name(&group.name)?;
        if !group_ids.insert(group.id.as_str()) {
            return Err("worker_role_group_id_duplicate".to_string());
        }
        if !group_names.insert(group.name.to_lowercase()) {
            return Err("worker_role_group_name_duplicate".to_string());
        }
        for role_id in &group.role_ids {
            if !role_ids.contains(role_id.as_str()) {
                return Err("worker_role_group_role_not_found".to_string());
            }
            if !placed_role_ids.insert(role_id.as_str()) {
                return Err("worker_role_group_role_duplicate".to_string());
            }
        }
    }
    Ok(())
}

pub fn recover_role_library(bytes: &[u8]) -> WorkerRoleLibrary {
    let parsed = serde_json::from_slice::<WorkerRoleLibrary>(bytes).or_else(|_| {
        serde_json::from_slice::<Vec<WorkerRole>>(bytes).map(|roles| WorkerRoleLibrary {
            roles,
            groups: Vec::new(),
        })
    });
    let Ok(parsed) = parsed else {
        return WorkerRoleLibrary::default();
    };

    let mut recovered = WorkerRoleLibrary::default();
    let mut role_ids = BTreeSet::new();
    let mut role_names = BTreeSet::new();
    for role in parsed.roles.into_iter().take(MAX_WORKER_ROLES) {
        if validate_role_id(&role.id).is_err()
            || normalize_role_fields(&role.name, &role.description).is_err()
            || !role_ids.insert(role.id.clone())
            || !role_names.insert(role.name.to_lowercase())
        {
            continue;
        }
        recovered.roles.push(role);
    }

    let mut group_ids = BTreeSet::new();
    let mut group_names = BTreeSet::new();
    let mut placed_role_ids = BTreeSet::new();
    for mut group in parsed.groups.into_iter().take(MAX_WORKER_ROLE_GROUPS) {
        if validate_group_id(&group.id).is_err()
            || normalize_group_name(&group.name).is_err()
            || !group_ids.insert(group.id.clone())
            || !group_names.insert(group.name.to_lowercase())
        {
            continue;
        }
        group.role_ids.retain(|role_id| {
            role_ids.contains(role_id) && placed_role_ids.insert(role_id.clone())
        });
        recovered.groups.push(group);
    }
    recovered
}

pub fn load_role_library(path: &Path) -> Result<WorkerRoleLibrary, String> {
    let bytes = read_store(path)?;
    let Some(bytes) = bytes else {
        return Ok(WorkerRoleLibrary::default());
    };
    let library = match serde_json::from_slice::<WorkerRoleLibrary>(&bytes) {
        Ok(library) => library,
        Err(_) => {
            let roles = serde_json::from_slice::<Vec<WorkerRole>>(&bytes)
                .map_err(|_| "worker_role_store_parse_failed".to_string())?;
            WorkerRoleLibrary {
                roles,
                groups: Vec::new(),
            }
        }
    };
    validate_role_library(&library)?;
    Ok(library)
}

/// Loads the legacy array format used in individual Session directories.
pub fn load_roles(path: &Path) -> Result<Vec<WorkerRole>, String> {
    let bytes = read_store(path)?;
    let Some(bytes) = bytes else {
        return Ok(Vec::new());
    };
    let roles = serde_json::from_slice::<Vec<WorkerRole>>(&bytes)
        .map_err(|_| "worker_role_store_parse_failed".to_string())?;
    validate_role_collection(&roles)?;
    Ok(roles)
}

fn read_store(path: &Path) -> Result<Option<Vec<u8>>, String> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("worker_role_store_metadata_failed".to_string()),
    };
    if !metadata.is_file() || metadata.len() > MAX_ROLE_FILE_BYTES {
        return Err("worker_role_store_invalid".to_string());
    }
    fs::read(path)
        .map(Some)
        .map_err(|_| "worker_role_store_read_failed".to_string())
}

pub fn save_role_library(path: &Path, library: &WorkerRoleLibrary) -> Result<(), String> {
    validate_role_library(library)?;
    save_payload(path, library)
}

#[cfg(test)]
pub fn save_roles(path: &Path, roles: &[WorkerRole]) -> Result<(), String> {
    validate_role_collection(roles)?;
    save_payload(path, roles)
}

fn save_payload(path: &Path, value: &(impl Serialize + ?Sized)) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "worker_role_path_invalid".to_string())?;
    fs::create_dir_all(parent).map_err(|_| "worker_role_store_create_failed".to_string())?;
    let payload = serde_json::to_vec_pretty(value)
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

    fn role(id: &str, name: &str) -> WorkerRole {
        WorkerRole {
            id: id.to_string(),
            name: name.to_string(),
            description: format!("{name} instructions"),
        }
    }

    #[test]
    fn role_library_round_trips_groups_and_order() {
        let path = temp_path("library_roundtrip");
        let library = WorkerRoleLibrary {
            roles: vec![role("role_1", "Reviewer"), role("role_2", "Builder")],
            groups: vec![WorkerRoleGroup {
                id: "group_quality".to_string(),
                name: "Quality".to_string(),
                role_ids: vec!["role_2".to_string(), "role_1".to_string()],
            }],
        };
        save_role_library(&path, &library).unwrap();
        assert_eq!(load_role_library(&path).unwrap(), library);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn legacy_role_array_loads_as_ungrouped_library() {
        let path = temp_path("legacy");
        save_roles(&path, &[role("role_1", "Reviewer")]).unwrap();
        let library = load_role_library(&path).unwrap();
        assert_eq!(library.roles.len(), 1);
        assert!(library.groups.is_empty());
        let _ = fs::remove_file(path);
    }

    #[test]
    fn recovery_drops_invalid_duplicates_and_repairs_group_references_deterministically() {
        let library = WorkerRoleLibrary {
            roles: vec![
                role("role_keep", "Reviewer"),
                role("role_duplicate_name", "reviewer"),
                role("bad id", "Broken"),
                role("role_second", "Builder"),
            ],
            groups: vec![
                WorkerRoleGroup {
                    id: "group_keep".to_string(),
                    name: "Quality".to_string(),
                    role_ids: vec![
                        "missing".to_string(),
                        "role_keep".to_string(),
                        "role_keep".to_string(),
                    ],
                },
                WorkerRoleGroup {
                    id: "group_duplicate_name".to_string(),
                    name: "quality".to_string(),
                    role_ids: vec!["role_second".to_string()],
                },
                WorkerRoleGroup {
                    id: "group_second".to_string(),
                    name: "Delivery".to_string(),
                    role_ids: vec!["role_keep".to_string(), "role_second".to_string()],
                },
            ],
        };
        let bytes = serde_json::to_vec(&library).unwrap();

        let recovered = recover_role_library(&bytes);

        assert_eq!(
            recovered
                .roles
                .iter()
                .map(|role| role.id.as_str())
                .collect::<Vec<_>>(),
            vec!["role_keep", "role_second"]
        );
        assert_eq!(recovered.groups.len(), 2);
        assert_eq!(recovered.groups[0].role_ids, vec!["role_keep"]);
        assert_eq!(recovered.groups[1].role_ids, vec!["role_second"]);
        validate_role_library(&recovered).unwrap();
    }

    #[test]
    fn failed_role_library_replace_removes_temporary_file() {
        let root = std::env::temp_dir().join(format!(
            "timem_worker_roles_replace_failure_{}_{}",
            std::process::id(),
            NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("worker_roles.json");
        fs::create_dir(&path).unwrap();
        let result = save_role_library(
            &path,
            &WorkerRoleLibrary {
                roles: vec![role("role_1", "Reviewer")],
                groups: Vec::new(),
            },
        );

        assert_eq!(result.unwrap_err(), "worker_role_store_replace_failed");
        let leftovers = fs::read_dir(&root)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
            .collect::<Vec<_>>();
        assert!(leftovers.is_empty(), "failed save left temporary files");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn validation_rejects_duplicate_placement_and_names() {
        let duplicate_names = WorkerRoleLibrary {
            roles: vec![role("role_1", "Review"), role("role_2", "review")],
            groups: Vec::new(),
        };
        assert_eq!(
            validate_role_library(&duplicate_names).unwrap_err(),
            "worker_role_name_duplicate"
        );

        let duplicate_placement = WorkerRoleLibrary {
            roles: vec![role("role_1", "Review")],
            groups: vec![
                WorkerRoleGroup {
                    id: "group_a".to_string(),
                    name: "A".to_string(),
                    role_ids: vec!["role_1".to_string()],
                },
                WorkerRoleGroup {
                    id: "group_b".to_string(),
                    name: "B".to_string(),
                    role_ids: vec!["role_1".to_string()],
                },
            ],
        };
        assert_eq!(
            validate_role_library(&duplicate_placement).unwrap_err(),
            "worker_role_group_role_duplicate"
        );
    }
}
