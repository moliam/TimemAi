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
#[path = "../tests/unit/workspace_tests.rs"]
mod tests;
