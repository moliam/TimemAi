use crate::status_view::HostStatusLevel;
use serde_json::json;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceState {
    dirs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceChange {
    Added(String),
    Removed(String),
    Unchanged(WorkspaceUnchangedReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceUnchangedReason {
    EmptyInput,
    Duplicate,
    IndexOutOfRange,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceMenuReport {
    pub dirs: Vec<String>,
    pub is_empty: bool,
    pub add_index: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceCommand {
    AddDir { value: String, home_dir: PathBuf },
    RemoveIndex(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceCommandOutcome {
    Added(String),
    Removed(String),
    EmptyInput,
    Duplicate,
    IndexOutOfRange,
    SaveFailed {
        attempted_change: WorkspaceChange,
        error: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceCommandReport {
    pub outcome: WorkspaceCommandOutcome,
    pub dirs: Vec<String>,
    pub changed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceCommandMessageKind {
    Added,
    Removed,
    Cancelled,
    Duplicate,
    SelectionInvalid,
    SaveFailed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceCommandMessage {
    pub kind: WorkspaceCommandMessageKind,
    pub level: HostStatusLevel,
    pub subject: Option<String>,
    pub error: Option<String>,
}

impl WorkspaceCommandReport {
    pub fn message(&self) -> WorkspaceCommandMessage {
        match &self.outcome {
            WorkspaceCommandOutcome::Added(normalized) => WorkspaceCommandMessage {
                kind: WorkspaceCommandMessageKind::Added,
                level: HostStatusLevel::Info,
                subject: Some(normalized.clone()),
                error: None,
            },
            WorkspaceCommandOutcome::Removed(removed) => WorkspaceCommandMessage {
                kind: WorkspaceCommandMessageKind::Removed,
                level: HostStatusLevel::Info,
                subject: Some(removed.clone()),
                error: None,
            },
            WorkspaceCommandOutcome::EmptyInput => WorkspaceCommandMessage {
                kind: WorkspaceCommandMessageKind::Cancelled,
                level: HostStatusLevel::Info,
                subject: None,
                error: None,
            },
            WorkspaceCommandOutcome::Duplicate => WorkspaceCommandMessage {
                kind: WorkspaceCommandMessageKind::Duplicate,
                level: HostStatusLevel::Warning,
                subject: None,
                error: None,
            },
            WorkspaceCommandOutcome::IndexOutOfRange => WorkspaceCommandMessage {
                kind: WorkspaceCommandMessageKind::SelectionInvalid,
                level: HostStatusLevel::Warning,
                subject: None,
                error: None,
            },
            WorkspaceCommandOutcome::SaveFailed { error, .. } => WorkspaceCommandMessage {
                kind: WorkspaceCommandMessageKind::SaveFailed,
                level: HostStatusLevel::Error,
                subject: None,
                error: Some(error.clone()),
            },
        }
    }
}

impl WorkspaceState {
    pub fn new(dirs: Vec<String>) -> Self {
        let mut state = Self { dirs };
        state.normalize_order();
        state
    }

    pub fn dirs(&self) -> &[String] {
        &self.dirs
    }

    pub fn into_dirs(self) -> Vec<String> {
        self.dirs
    }

    pub fn menu_report(&self) -> WorkspaceMenuReport {
        WorkspaceMenuReport {
            dirs: self.dirs.clone(),
            is_empty: self.dirs.is_empty(),
            add_index: self.dirs.len(),
        }
    }

    pub fn add_dir(&mut self, value: &str, home_dir: &Path) -> WorkspaceChange {
        if value.trim().is_empty() {
            return WorkspaceChange::Unchanged(WorkspaceUnchangedReason::EmptyInput);
        }
        let normalized = normalize_workspace_dir(value, home_dir);
        if self.dirs.iter().any(|dir| dir == &normalized) {
            return WorkspaceChange::Unchanged(WorkspaceUnchangedReason::Duplicate);
        }
        self.dirs.push(normalized.clone());
        self.normalize_order();
        WorkspaceChange::Added(normalized)
    }

    pub fn remove_index(&mut self, index: usize) -> WorkspaceChange {
        if index >= self.dirs.len() {
            return WorkspaceChange::Unchanged(WorkspaceUnchangedReason::IndexOutOfRange);
        }
        WorkspaceChange::Removed(self.dirs.remove(index))
    }

    fn normalize_order(&mut self) {
        self.dirs.retain(|dir| !dir.trim().is_empty());
        self.dirs.sort();
        self.dirs.dedup();
    }
}

pub fn normalize_workspace_dir(value: &str, home_dir: &Path) -> String {
    let expanded = expand_tilde(value.trim(), home_dir);
    std::fs::canonicalize(&expanded)
        .unwrap_or(expanded)
        .to_string_lossy()
        .to_string()
}

pub fn load_workspace_dirs_from_path(path: &Path) -> Vec<String> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) else {
        return Vec::new();
    };
    val["dirs"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .map(WorkspaceState::new)
        .map(WorkspaceState::into_dirs)
        .unwrap_or_default()
}

pub fn save_workspace_dirs_to_path(path: &Path, dirs: &[String]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let normalized = WorkspaceState::new(dirs.to_vec()).into_dirs();
    let content =
        serde_json::to_string_pretty(&json!({"dirs": normalized})).map_err(|e| e.to_string())?;
    std::fs::write(path, content).map_err(|e| e.to_string())
}

pub fn workspace_reference_context(dirs: &[String]) -> Option<String> {
    let dirs = WorkspaceState::new(dirs.to_vec()).into_dirs();
    if dirs.is_empty() {
        return None;
    }
    let lines = dirs
        .iter()
        .map(|dir| format!("- {dir}"))
        .collect::<Vec<_>>()
        .join("\n");
    Some(format!(
        "workspace_dirs (model reference; not a host restriction):\n{lines}"
    ))
}

pub fn workspace_menu_report(dirs: &[String]) -> WorkspaceMenuReport {
    WorkspaceState::new(dirs.to_vec()).menu_report()
}

pub fn apply_workspace_command_to_path(
    path: &Path,
    command: WorkspaceCommand,
) -> WorkspaceCommandReport {
    let mut workspace = WorkspaceState::new(load_workspace_dirs_from_path(path));
    let change = match command {
        WorkspaceCommand::AddDir { value, home_dir } => workspace.add_dir(&value, &home_dir),
        WorkspaceCommand::RemoveIndex(index) => workspace.remove_index(index),
    };

    match change {
        WorkspaceChange::Added(normalized) => {
            let dirs = workspace.into_dirs();
            match save_workspace_dirs_to_path(path, &dirs) {
                Ok(()) => WorkspaceCommandReport {
                    outcome: WorkspaceCommandOutcome::Added(normalized),
                    dirs,
                    changed: true,
                },
                Err(error) => WorkspaceCommandReport {
                    outcome: WorkspaceCommandOutcome::SaveFailed {
                        attempted_change: WorkspaceChange::Added(normalized),
                        error,
                    },
                    dirs,
                    changed: false,
                },
            }
        }
        WorkspaceChange::Removed(removed) => {
            let dirs = workspace.into_dirs();
            match save_workspace_dirs_to_path(path, &dirs) {
                Ok(()) => WorkspaceCommandReport {
                    outcome: WorkspaceCommandOutcome::Removed(removed),
                    dirs,
                    changed: true,
                },
                Err(error) => WorkspaceCommandReport {
                    outcome: WorkspaceCommandOutcome::SaveFailed {
                        attempted_change: WorkspaceChange::Removed(removed),
                        error,
                    },
                    dirs,
                    changed: false,
                },
            }
        }
        WorkspaceChange::Unchanged(WorkspaceUnchangedReason::EmptyInput) => {
            WorkspaceCommandReport {
                outcome: WorkspaceCommandOutcome::EmptyInput,
                dirs: workspace.into_dirs(),
                changed: false,
            }
        }
        WorkspaceChange::Unchanged(WorkspaceUnchangedReason::Duplicate) => WorkspaceCommandReport {
            outcome: WorkspaceCommandOutcome::Duplicate,
            dirs: workspace.into_dirs(),
            changed: false,
        },
        WorkspaceChange::Unchanged(WorkspaceUnchangedReason::IndexOutOfRange) => {
            WorkspaceCommandReport {
                outcome: WorkspaceCommandOutcome::IndexOutOfRange,
                dirs: workspace.into_dirs(),
                changed: false,
            }
        }
    }
}

fn expand_tilde(value: &str, home_dir: &Path) -> PathBuf {
    if value == "~" {
        return home_dir.to_path_buf();
    }
    if let Some(rest) = value.strip_prefix("~/") {
        return home_dir.join(rest);
    }
    Path::new(value).to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn tmp_workspace_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "timem_workspace_{name}_{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        dir.join("workspace.json")
    }

    #[test]
    fn state_adds_sorts_and_deduplicates_workspace_dirs() {
        let home = Path::new("/home/example");
        let mut state =
            WorkspaceState::new(vec!["/z".to_string(), "/a".to_string(), "/z".to_string()]);

        assert_eq!(state.dirs(), &["/a".to_string(), "/z".to_string()]);
        assert_eq!(
            state.add_dir("~/project", home),
            WorkspaceChange::Added("/home/example/project".to_string())
        );
        assert_eq!(
            state.add_dir("~/project", home),
            WorkspaceChange::Unchanged(WorkspaceUnchangedReason::Duplicate)
        );
        assert_eq!(
            state.dirs(),
            &[
                "/a".to_string(),
                "/home/example/project".to_string(),
                "/z".to_string()
            ]
        );
    }

    #[test]
    fn state_removes_by_index_with_bounds_check() {
        let mut state = WorkspaceState::new(vec!["/a".to_string(), "/b".to_string()]);

        assert_eq!(
            state.remove_index(1),
            WorkspaceChange::Removed("/b".to_string())
        );
        assert_eq!(
            state.remove_index(2),
            WorkspaceChange::Unchanged(WorkspaceUnchangedReason::IndexOutOfRange)
        );
        assert_eq!(state.dirs(), &["/a".to_string()]);
    }

    #[test]
    fn workspace_menu_report_is_ui_neutral_command_data() {
        let report = workspace_menu_report(&[
            "/z".to_string(),
            "".to_string(),
            "/a".to_string(),
            "/z".to_string(),
        ]);
        let debug = format!("{report:?}");
        for forbidden in ["\x1b[", "▶", "Add..."] {
            assert!(
                !debug.contains(forbidden),
                "core workspace report must stay UI-neutral and avoid terminal marker {forbidden:?}"
            );
        }
        assert_eq!(report.dirs, vec!["/a".to_string(), "/z".to_string()]);
        assert!(!report.is_empty);
        assert_eq!(report.add_index, 2);

        let empty = workspace_menu_report(&[]);
        assert!(empty.is_empty);
        assert_eq!(empty.add_index, 0);
    }

    #[test]
    fn workspace_command_report_adds_removes_and_persists_dirs() {
        let path = tmp_workspace_path("command_report");
        let home_dir = PathBuf::from("/home/example");

        let added = apply_workspace_command_to_path(
            &path,
            WorkspaceCommand::AddDir {
                value: "~/project".to_string(),
                home_dir,
            },
        );
        assert_eq!(
            added.outcome,
            WorkspaceCommandOutcome::Added("/home/example/project".to_string())
        );
        assert_eq!(
            added.message(),
            WorkspaceCommandMessage {
                kind: WorkspaceCommandMessageKind::Added,
                level: HostStatusLevel::Info,
                subject: Some("/home/example/project".to_string()),
                error: None,
            }
        );
        assert_eq!(added.dirs, vec!["/home/example/project".to_string()]);
        assert!(added.changed);
        assert_eq!(
            load_workspace_dirs_from_path(&path),
            vec!["/home/example/project".to_string()]
        );

        let duplicate = apply_workspace_command_to_path(
            &path,
            WorkspaceCommand::AddDir {
                value: "/home/example/project".to_string(),
                home_dir: PathBuf::from("/home/example"),
            },
        );
        assert_eq!(duplicate.outcome, WorkspaceCommandOutcome::Duplicate);
        assert_eq!(
            duplicate.message().kind,
            WorkspaceCommandMessageKind::Duplicate
        );
        assert!(!duplicate.changed);

        let removed = apply_workspace_command_to_path(&path, WorkspaceCommand::RemoveIndex(0));
        assert_eq!(
            removed.outcome,
            WorkspaceCommandOutcome::Removed("/home/example/project".to_string())
        );
        assert_eq!(removed.message().kind, WorkspaceCommandMessageKind::Removed);
        assert!(removed.dirs.is_empty());
        assert!(removed.changed);

        let out_of_range = apply_workspace_command_to_path(&path, WorkspaceCommand::RemoveIndex(9));
        assert_eq!(
            out_of_range.outcome,
            WorkspaceCommandOutcome::IndexOutOfRange
        );
        assert_eq!(
            out_of_range.message().kind,
            WorkspaceCommandMessageKind::SelectionInvalid
        );
        assert!(!out_of_range.changed);

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn workspace_command_message_is_ui_neutral() {
        let report = WorkspaceCommandReport {
            outcome: WorkspaceCommandOutcome::SaveFailed {
                attempted_change: WorkspaceChange::Added("/tmp/project".to_string()),
                error: "disk full".to_string(),
            },
            dirs: vec![],
            changed: false,
        };

        let message = report.message();
        assert_eq!(message.kind, WorkspaceCommandMessageKind::SaveFailed);
        assert_eq!(message.level, HostStatusLevel::Error);
        assert_eq!(message.error.as_deref(), Some("disk full"));
        let debug = format!("{message:?}");
        for forbidden in ["\x1b[", "已加入", "workspace：", "▶"] {
            assert!(
                !debug.contains(forbidden),
                "workspace command message must stay UI-neutral and avoid terminal copy {forbidden:?}"
            );
        }
    }

    #[test]
    fn path_normalization_canonicalizes_existing_paths() {
        let dir = std::env::temp_dir().join(format!(
            "timem_workspace_core_{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        ));
        std::fs::create_dir_all(dir.join("child")).unwrap();
        let nested = dir.join(".").join("child").join("..");

        assert_eq!(
            normalize_workspace_dir(nested.to_str().unwrap(), Path::new("/home/example")),
            dir.canonicalize().unwrap().to_string_lossy().to_string()
        );

        let missing = dir.join("missing").join("path");
        assert_eq!(
            normalize_workspace_dir(missing.to_str().unwrap(), Path::new("/home/example")),
            missing.to_string_lossy().to_string()
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn workspace_dirs_load_and_save_json_state() {
        let dir = std::env::temp_dir().join(format!(
            "timem_workspace_store_{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        ));
        let path = dir.join("workspace.json");
        let dirs = vec![
            "/z".to_string(),
            "".to_string(),
            "/a".to_string(),
            "/z".to_string(),
        ];

        save_workspace_dirs_to_path(&path, &dirs).unwrap();
        assert_eq!(
            load_workspace_dirs_from_path(&path),
            vec!["/a".to_string(), "/z".to_string()]
        );

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn workspace_dirs_load_empty_on_missing_or_malformed_file() {
        let dir = std::env::temp_dir().join(format!(
            "timem_workspace_malformed_{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let missing = dir.join("missing.json");
        assert!(load_workspace_dirs_from_path(&missing).is_empty());

        let malformed = dir.join("workspace.json");
        std::fs::write(&malformed, "not json").unwrap();
        assert!(load_workspace_dirs_from_path(&malformed).is_empty());

        std::fs::write(&malformed, "{\"dirs\":[\"/b\", 123, \"/a\"]}").unwrap();
        assert_eq!(
            load_workspace_dirs_from_path(&malformed),
            vec!["/a".to_string(), "/b".to_string()]
        );

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn workspace_reference_context_is_core_owned_prompt_context() {
        assert_eq!(workspace_reference_context(&[]), None);
        assert_eq!(
            workspace_reference_context(&[
                "/z".to_string(),
                "".to_string(),
                "/a".to_string(),
                "/z".to_string(),
            ]),
            Some(
                "workspace_dirs (model reference; not a host restriction):\n- /a\n- /z".to_string()
            )
        );
        let context = workspace_reference_context(&["/a".to_string()]).unwrap();
        for forbidden in ["shell restriction", "terminal restriction"] {
            assert!(
                !context.contains(forbidden),
                "core workspace prompt context should stay host-neutral: {context}"
            );
        }
    }
}
