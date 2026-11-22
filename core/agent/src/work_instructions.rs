use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use crate::status_view::HostStatusLevel;

pub const WORK_INSTRUCTION_FILENAMES: [&str; 2] = ["AGENTS.md", "CLAUDE.md"];

/// Ecosystem-compatible instruction candidates inside the working directory.
/// Order is the resident-context injection order: plain agents/claude files
/// first, then the nested Claude/Codex equivalents.
const PROJECT_INSTRUCTION_CANDIDATES: [&str; 4] = [
    "AGENTS.md",
    "CLAUDE.md",
    ".claude/CLAUDE.md",
    ".codex/AGENTS.md",
];

/// Claude/Codex user-global instruction files, relative to the user home.
const GLOBAL_INSTRUCTION_CANDIDATES: [&str; 2] = [".claude/CLAUDE.md", ".codex/AGENTS.md"];

/// Hard cap for one instruction file body. Claude/Codex memory files are
/// user-controlled, so an unbounded read would let a runaway file inflate
/// every prompt; a char-boundary-safe truncation keeps the block bounded.
pub const WORK_INSTRUCTION_FILE_CONTENT_LIMIT: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkInstructionLoadMode {
    Silent,
    Ask,
    Off,
}

/// Where a resident instruction candidate lives. `Global` files come from the
/// user home (`~/.claude`, `~/.codex`), `Project` files from the working
/// directory or its `.claude`/`.codex` subdirectories.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkInstructionSource {
    Project,
    Global,
}

impl WorkInstructionSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::Project => "project",
            Self::Global => "global",
        }
    }
}

/// One discoverable ecosystem instruction file. Candidates are the
/// authoritative allow-list for per-file resident-context selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkInstructionCandidate {
    pub path: PathBuf,
    pub filename: String,
    pub source: WorkInstructionSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkInstructionFile {
    pub path: PathBuf,
    pub directory: PathBuf,
    pub filename: String,
    pub content: String,
    pub source: WorkInstructionSource,
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

/// Resolves the user home used for global Claude/Codex instruction files.
/// The explicit override exists so sandboxes and containers can relocate the
/// instruction home without touching real user data.
pub fn work_instruction_home_from_env() -> Option<PathBuf> {
    for key in ["TIMEM_WORK_INSTRUCTION_HOME", "HOME", "USERPROFILE"] {
        if let Ok(value) = std::env::var(key) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Some(PathBuf::from(trimmed));
            }
        }
    }
    None
}

fn push_work_instruction_candidate(
    candidates: &mut Vec<WorkInstructionCandidate>,
    seen: &mut BTreeSet<PathBuf>,
    path: PathBuf,
    source: WorkInstructionSource,
) {
    let Some(filename) = path
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_string)
    else {
        return;
    };
    let canonical = std::fs::canonicalize(&path).ok();
    let dedupe_key = canonical.clone().unwrap_or_else(|| path.clone());
    if !seen.insert(dedupe_key) {
        return;
    }
    candidates.push(WorkInstructionCandidate {
        path,
        filename,
        source,
    });
}

/// Discovers all ecosystem instruction candidates for a working directory.
/// Project-scope candidates keep the working-directory order; global
/// candidates follow. Symlinked duplicates collapse onto one candidate via
/// canonical paths. `home` is injected, not read from the environment, so
/// callers and tests stay deterministic.
pub fn discover_work_instruction_candidates(
    work_dir: &Path,
    home: Option<&Path>,
) -> Vec<WorkInstructionCandidate> {
    let mut candidates = Vec::new();
    let mut seen = BTreeSet::new();
    for relative in PROJECT_INSTRUCTION_CANDIDATES {
        let path = work_dir.join(relative);
        if path.is_file() {
            push_work_instruction_candidate(
                &mut candidates,
                &mut seen,
                path,
                WorkInstructionSource::Project,
            );
        }
    }
    if let Some(home) = home {
        for relative in GLOBAL_INSTRUCTION_CANDIDATES {
            let path = home.join(relative);
            if path.is_file() {
                push_work_instruction_candidate(
                    &mut candidates,
                    &mut seen,
                    path,
                    WorkInstructionSource::Global,
                );
            }
        }
    }
    candidates
}

pub fn discover_work_instruction_files(dir: &Path) -> Vec<PathBuf> {
    WORK_INSTRUCTION_FILENAMES
        .iter()
        .map(|name| dir.join(name))
        .filter(|path| path.is_file())
        .collect()
}

/// Filters discovered candidates down to the resident selection. `None` keeps
/// every discovered candidate, which preserves the pre-selection loading
/// behavior; an explicit empty selection intentionally loads nothing.
fn selected_instruction_paths(
    candidates: &[WorkInstructionCandidate],
    selection: Option<&BTreeSet<String>>,
) -> Vec<PathBuf> {
    candidates
        .iter()
        .filter(|candidate| match selection {
            None => true,
            Some(selected) => selected.contains(&candidate.path.display().to_string()),
        })
        .map(|candidate| candidate.path.clone())
        .collect()
}

/// Validates a requested per-file selection against freshly discovered
/// candidates. Every path must resolve to a current candidate via canonical
/// comparison; unknown, missing, empty, or duplicate-escaping paths fail
/// closed so host state can never inject an arbitrary file.
pub fn validate_work_instruction_selection(
    work_dir: &Path,
    home: Option<&Path>,
    requested: &[String],
) -> Result<BTreeSet<String>, String> {
    let candidates = discover_work_instruction_candidates(work_dir, home);
    let mut selected = BTreeSet::new();
    for raw in requested {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err("work_instruction_selection_invalid".to_string());
        }
        let requested_path = Path::new(trimmed);
        let requested_canonical = std::fs::canonicalize(requested_path)
            .map_err(|_| "work_instruction_selection_invalid".to_string())?;
        let matched = candidates
            .iter()
            .find(|candidate| {
                std::fs::canonicalize(&candidate.path)
                    .map(|canonical| canonical == requested_canonical)
                    .unwrap_or(false)
            })
            .ok_or_else(|| "work_instruction_selection_invalid".to_string())?;
        selected.insert(matched.path.display().to_string());
    }
    Ok(selected)
}

fn cap_work_instruction_content(mut content: String) -> String {
    if content.len() <= WORK_INSTRUCTION_FILE_CONTENT_LIMIT {
        return content;
    }
    let mut end = WORK_INSTRUCTION_FILE_CONTENT_LIMIT;
    while end > 0 && !content.is_char_boundary(end) {
        end -= 1;
    }
    content.truncate(end);
    content.push_str("\n... [truncated]");
    content
}

fn read_work_instruction_file(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path)
        .map(cap_work_instruction_content)
        .map_err(|err| format!("read_work_instruction_failed:{}:{err}", path.display()))
}

fn work_instruction_file_from_candidate(
    path: PathBuf,
    filename: String,
    source: WorkInstructionSource,
) -> Result<WorkInstructionFile, String> {
    let content = read_work_instruction_file(&path)?;
    let directory = path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    Ok(WorkInstructionFile {
        directory,
        filename,
        path,
        content,
        source,
    })
}

/// Selection-aware loader over ecosystem candidates.
pub fn load_work_instruction_context_selected(
    work_dir: &Path,
    home: Option<&Path>,
    selection: Option<&BTreeSet<String>>,
) -> Result<Option<WorkInstructionContext>, String> {
    let candidates = discover_work_instruction_candidates(work_dir, home);
    let paths = selected_instruction_paths(&candidates, selection);
    if paths.is_empty() {
        return Ok(None);
    }

    let mut files = Vec::new();
    for candidate in candidates
        .iter()
        .filter(|candidate| paths.contains(&candidate.path))
    {
        files.push(work_instruction_file_from_candidate(
            candidate.path.clone(),
            candidate.filename.clone(),
            candidate.source,
        )?);
    }

    let context = format_work_instruction_context(&files).unwrap_or_default();
    Ok(Some(WorkInstructionContext { files, context }))
}

pub fn load_work_instruction_context(dir: &Path) -> Result<Option<WorkInstructionContext>, String> {
    let paths = discover_work_instruction_files(dir);
    if paths.is_empty() {
        return Ok(None);
    }

    let mut files = Vec::new();
    for path in paths {
        let content = read_work_instruction_file(&path)?;
        files.push(WorkInstructionFile {
            directory: dir.to_path_buf(),
            filename: path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("work_instruction")
                .to_string(),
            path,
            content,
            source: WorkInstructionSource::Project,
        });
    }

    let context = format_work_instruction_context(&files).unwrap_or_default();
    Ok(Some(WorkInstructionContext { files, context }))
}

pub fn work_instruction_load_report_selected(
    work_dir: &Path,
    home: Option<&Path>,
    selection: Option<&BTreeSet<String>>,
) -> WorkInstructionLoadReport {
    let candidates = discover_work_instruction_candidates(work_dir, home);
    let selected_names = |candidates: &[WorkInstructionCandidate]| {
        selected_instruction_paths(candidates, selection)
            .iter()
            .filter_map(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .map(str::to_string)
            })
            .collect::<Vec<_>>()
    };
    match load_work_instruction_context_selected(work_dir, home, selection) {
        Ok(Some(context)) => WorkInstructionLoadReport {
            status: WorkInstructionLoadStatus::Loaded,
            directory: work_dir.to_path_buf(),
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
            directory: work_dir.to_path_buf(),
            file_names: Vec::new(),
            context: None,
            error: None,
        },
        Err(err) => WorkInstructionLoadReport {
            status: WorkInstructionLoadStatus::Failed,
            directory: work_dir.to_path_buf(),
            file_names: selected_names(&candidates),
            context: None,
            error: Some(err),
        },
    }
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

/// Selection-aware host confirmation request: names only the files that the
/// current resident selection would actually load.
pub fn work_instruction_load_request_selected(
    work_dir: &Path,
    home: Option<&Path>,
    selection: Option<&BTreeSet<String>>,
) -> Option<WorkInstructionLoadRequest> {
    let candidates = discover_work_instruction_candidates(work_dir, home);
    let file_names = selected_instruction_paths(&candidates, selection)
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
            directory: work_dir.to_path_buf(),
            file_names,
        })
    }
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

pub fn format_work_instruction_context(files: &[WorkInstructionFile]) -> Option<String> {
    if files.is_empty() {
        return None;
    }

    let mut out = String::from(
        "work_directory_instructions:\nThese instructions were loaded from Claude/Codex-compatible instruction files for the current working directory. Follow them while working in that directory.\n",
    );
    for file in files {
        out.push_str(&format!(
            "\n[BEGIN PROJECT INSTRUCTION file=\"{}\" directory=\"{}\" source=\"{}\"]\n",
            file.filename,
            file.directory.display(),
            file.source.label()
        ));
        out.push_str(file.content.trim());
        out.push_str(&format!(
            "\n[END PROJECT INSTRUCTION file=\"{}\"]\n",
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
