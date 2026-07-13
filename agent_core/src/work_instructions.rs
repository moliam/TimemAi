use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::status_view::HostStatusLevel;

pub const WORK_INSTRUCTION_FILENAMES: [&str; 2] = ["AGENTS.md", "CLAUDE.md"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkInstructionLoadMode {
    Silent,
    Ask,
    Off,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkInstructionFile {
    pub path: PathBuf,
    pub directory: PathBuf,
    pub filename: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkInstructionContext {
    pub files: Vec<WorkInstructionFile>,
    pub context: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkInstructionLoadStatus {
    Loaded,
    NotFound,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkInstructionLoadReport {
    pub status: WorkInstructionLoadStatus,
    pub directory: PathBuf,
    pub file_names: Vec<String>,
    pub context: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkInstructionLoadMessageKind {
    Loaded,
    NotFound,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkInstructionLoadMessage {
    pub kind: WorkInstructionLoadMessageKind,
    pub level: Option<HostStatusLevel>,
    pub directory: PathBuf,
    pub file_names: Vec<String>,
    pub error: Option<String>,
}

impl WorkInstructionLoadReport {
    pub fn message(&self) -> WorkInstructionLoadMessage {
        let kind = match self.status {
            WorkInstructionLoadStatus::Loaded => WorkInstructionLoadMessageKind::Loaded,
            WorkInstructionLoadStatus::NotFound => WorkInstructionLoadMessageKind::NotFound,
            WorkInstructionLoadStatus::Failed => WorkInstructionLoadMessageKind::Failed,
        };
        let level = match kind {
            WorkInstructionLoadMessageKind::Loaded => Some(HostStatusLevel::Info),
            WorkInstructionLoadMessageKind::NotFound => None,
            WorkInstructionLoadMessageKind::Failed => Some(HostStatusLevel::Warning),
        };
        WorkInstructionLoadMessage {
            kind,
            level,
            directory: self.directory.clone(),
            file_names: self.file_names.clone(),
            error: self.error.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkInstructionLoadRequest {
    pub directory: PathBuf,
    pub file_names: Vec<String>,
}

pub fn work_instruction_mode_from_sources(
    option: Option<&str>,
    env: &HashMap<String, String>,
) -> WorkInstructionLoadMode {
    option
        .or_else(|| env.get("TIMEM_WORK_INSTRUCTIONS").map(String::as_str))
        .map(parse_work_instruction_mode)
        .unwrap_or(WorkInstructionLoadMode::Silent)
}

pub fn parse_work_instruction_mode(value: &str) -> WorkInstructionLoadMode {
    match value.trim().to_ascii_lowercase().as_str() {
        "ask" => WorkInstructionLoadMode::Ask,
        "off" | "disable" | "disabled" => WorkInstructionLoadMode::Off,
        _ => WorkInstructionLoadMode::Silent,
    }
}

pub fn discover_work_instruction_files(dir: &Path) -> Vec<PathBuf> {
    WORK_INSTRUCTION_FILENAMES
        .iter()
        .map(|name| dir.join(name))
        .filter(|path| path.is_file())
        .collect()
}

pub fn work_instruction_load_request(dir: &Path) -> Option<WorkInstructionLoadRequest> {
    let file_names = discover_work_instruction_files(dir)
        .iter()
        .filter_map(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .map(str::to_string)
        })
        .collect::<Vec<_>>();
    if file_names.is_empty() {
        None
    } else {
        Some(WorkInstructionLoadRequest {
            directory: dir.to_path_buf(),
            file_names,
        })
    }
}

pub fn load_work_instruction_context(dir: &Path) -> Result<Option<WorkInstructionContext>, String> {
    let paths = discover_work_instruction_files(dir);
    if paths.is_empty() {
        return Ok(None);
    }

    let mut files = Vec::new();
    for path in paths {
        let content = std::fs::read_to_string(&path)
            .map_err(|err| format!("read_work_instruction_failed:{}:{err}", path.display()))?;
        files.push(WorkInstructionFile {
            directory: dir.to_path_buf(),
            filename: path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("work_instruction")
                .to_string(),
            path,
            content,
        });
    }

    let context = format_work_instruction_context(&files).unwrap_or_default();
    Ok(Some(WorkInstructionContext { files, context }))
}

pub fn work_instruction_load_report(dir: &Path) -> WorkInstructionLoadReport {
    match load_work_instruction_context(dir) {
        Ok(Some(context)) => WorkInstructionLoadReport {
            status: WorkInstructionLoadStatus::Loaded,
            directory: dir.to_path_buf(),
            file_names: context
                .files
                .iter()
                .map(|file| file.filename.clone())
                .collect(),
            context: Some(context.context),
            error: None,
        },
        Ok(None) => WorkInstructionLoadReport {
            status: WorkInstructionLoadStatus::NotFound,
            directory: dir.to_path_buf(),
            file_names: Vec::new(),
            context: None,
            error: None,
        },
        Err(err) => WorkInstructionLoadReport {
            status: WorkInstructionLoadStatus::Failed,
            directory: dir.to_path_buf(),
            file_names: discover_work_instruction_files(dir)
                .iter()
                .filter_map(|path| {
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .map(str::to_string)
                })
                .collect(),
            context: None,
            error: Some(err),
        },
    }
}

pub fn format_work_instruction_context(files: &[WorkInstructionFile]) -> Option<String> {
    if files.is_empty() {
        return None;
    }

    let mut out = String::from(
        "work_directory_instructions:\nThese instructions were loaded from files in the current working directory. Follow them while working in that directory.\n",
    );
    for file in files {
        out.push_str(&format!(
            "\n[BEGIN WORK_DIRECTORY_INSTRUCTION file=\"{}\" directory=\"{}\"]\n",
            file.filename,
            file.directory.display()
        ));
        out.push_str(file.content.trim());
        out.push_str(&format!(
            "\n[END WORK_DIRECTORY_INSTRUCTION file=\"{}\"]\n",
            file.filename
        ));
    }
    Some(out)
}

pub fn combine_additional_contexts<'a>(
    sections: impl IntoIterator<Item = Option<&'a str>>,
) -> Option<String> {
    let joined = sections
        .into_iter()
        .flatten()
        .map(str::trim)
        .filter(|section| !section.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    if joined.is_empty() {
        None
    } else {
        Some(joined)
    }
}

#[cfg(test)]
#[path = "../tests/unit/work_instructions_tests.rs"]
mod tests;
