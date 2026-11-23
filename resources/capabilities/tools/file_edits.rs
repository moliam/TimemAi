// Bounded file-edit evidence for shell tools. When the model declares `edit`
// paths on run_bash/run_powershell, the runtime snapshots those files before
// execution, re-reads them after, and produces structured unified diffs for
// the finish-topic payload consumed by interfaces. Everything here is bounded
// by design: hard caps on file count, snapshot bytes, per-file diff text, and
// the total serialized payload; overruns degrade to explicit skip reasons
// instead of unbounded data. Snapshot data never enters the model context.

use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

/// Maximum number of declared paths captured and reported per tool call.
pub(crate) const MAX_EDIT_FILES: usize = 16;
/// Per-file disk-snapshot cap. Larger files degrade to `too_large`.
const MAX_SNAPSHOT_FILE_BYTES: usize = 64 * 1024 * 1024;
/// Total disk budget retained per tool call.
const MAX_SNAPSHOT_TOTAL_BYTES: usize = 128 * 1024 * 1024;
/// Prefix bytes probed for NUL detection before a disk copy is attempted.
const SNAPSHOT_BINARY_PROBE_BYTES: usize = 8 * 1024;
/// Stream chunk size for the staged disk copies.
const SNAPSHOT_COPY_CHUNK_BYTES: usize = 64 * 1024;
/// Sequential counter disambiguating concurrent snapshot directories.
static SNAPSHOT_DIR_SEQ: AtomicU64 = AtomicU64::new(0);
/// Serialized diff text cap per file.
const MAX_DIFF_FILE_BYTES: usize = 32 * 1024;
/// Serialized total payload budget across all entries.
const MAX_EDITS_PAYLOAD_BYTES: usize = 96 * 1024;
/// Unified-diff context lines around each change region.
const DIFF_CONTEXT_LINES: usize = 3;
/// Anchor-less middle-section line cap before falling back to a whole-file
/// replace diff. Regions that share unique-line anchors are split regardless
/// of size; anchor discovery costs linear map passes, not DP cells.
const PATIENCE_MIDDLE_LINE_CAP: usize = 5000;
/// Maximum DP table cells for small-section LCS.
const DP_MAX_CELLS: usize = 40_000;
/// Patience nesting-depth cap; deeper sub-diffs fall back to whole replace.
/// Sequential anchor chains do not accumulate depth because every chain step
/// consumes at least one line per side; only nested sub-diffs count.
const PATIENCE_DEPTH_CAP: usize = 48;
/// Display/path clamp for model-declared path strings.
const MAX_DECLARED_PATH_CHARS: usize = 512;

const SKIP_BINARY: &str = "binary";
const SKIP_TOO_LARGE: &str = "too_large";
const SKIP_UNREADABLE: &str = "unreadable";
const SKIP_NOT_CAPTURED: &str = "not_captured";
const SKIP_BUDGET: &str = "budget_exhausted";
const SKIP_MISSING: &str = "missing";
/// Meta marker rendered when only line endings or the final newline changed.
const WHITESPACE_ONLY_NOTE: &str = "# Whitespace-only change: line endings or the final newline differ; content lines are identical.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileEditOp {
    Created,
    Modified,
    Deleted,
    Unchanged,
    Skipped,
}

impl FileEditOp {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Modified => "modified",
            Self::Deleted => "deleted",
            Self::Unchanged => "unchanged",
            Self::Skipped => "skipped",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEditEntry {
    pub path: String,
    pub op: FileEditOp,
    pub diff: Option<String>,
    pub added_lines: u32,
    pub removed_lines: u32,
    pub truncated: bool,
    pub skip_reason: Option<&'static str>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileEditsEvidence {
    pub edits: Vec<FileEditEntry>,
}

/// Snapshot of a file's content, or the reason it could not be diffed.
/// `OnDisk` points at a staged disk copy that is loaded at diff time, so the
/// capture side never holds file content in memory.
/// `Lines` holds the raw text (NUL-free, valid UTF-8, within the byte cap).
/// The line view is derived at diff time by splitting on `\n` and trimming a
/// trailing `\r` per line. Equality is decided on the raw bytes so invisible
/// rewrites (final newline, CRLF/LF flip) can never fold into an
/// "unchanged" card.
#[derive(Debug, Clone, PartialEq, Eq)]
enum FileContentSnapshot {
    Lines(String),
    OnDisk { path: PathBuf },
    Binary,
    TooLarge,
    Unreadable,
    Missing,
}

impl FileContentSnapshot {
    fn is_diffable(&self) -> bool {
        matches!(self, Self::Lines(_))
    }

    fn skip_reason(&self) -> &'static str {
        match self {
            Self::Binary => SKIP_BINARY,
            Self::TooLarge => SKIP_TOO_LARGE,
            _ => SKIP_UNREADABLE,
        }
    }
}

#[derive(Debug, Clone)]
struct DeclaredEditSnapshot {
    declared: String,
    path: PathBuf,
    /// `None` when the path was not captured (file-count overflow, or disk
    /// staging unavailable for an existing file).
    before: Option<FileContentSnapshot>,
}

/// Before-state of the declared edit paths for one shell tool call. Kept in
/// the job for background execution so the exit path can compute evidence.
/// Shared ownership of the private staging directory: dropping the last
/// handle removes the directory, so clones handed to background jobs stay
/// valid for as long as any holder needs them.
#[derive(Debug)]
struct SnapshotDirHandle(PathBuf);

impl Drop for SnapshotDirHandle {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

impl std::ops::Deref for SnapshotDirHandle {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.0
    }
}

impl AsRef<Path> for SnapshotDirHandle {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

#[derive(Debug, Clone, Default)]
pub struct EditContext {
    files: Vec<DeclaredEditSnapshot>,
    /// Private staging directory holding the before-copies; removed when the
    /// last clone of the context is dropped.
    snapshot_dir: Option<Arc<SnapshotDirHandle>>,
}

impl Drop for EditContext {
    fn drop(&mut self) {
        // Releasing this handle may remove the staging dir: the actual
        // removal happens when the last Arc clone (background job copy) is
        // dropped, see `SnapshotDirHandle`.
        drop(self.snapshot_dir.take());
    }
}

impl EditContext {
    /// Snapshot every declared path against `cwd` under the hard caps.
    /// Before-states are staged on disk in a private temp directory so the
    /// memory budget stays flat no matter how large the edited files are.
    pub(crate) fn capture(cwd: &Path, declared_paths: &[String]) -> Self {
        let mut files: Vec<DeclaredEditSnapshot> = Vec::new();
        let mut seen: HashSet<PathBuf> = HashSet::new();
        let mut budget = MAX_SNAPSHOT_TOTAL_BYTES as u64;
        let mut snapshot_dir: Option<Arc<SnapshotDirHandle>> = None;
        for declared in declared_paths {
            let declared = declared.trim();
            if declared.is_empty() || declared.chars().count() > MAX_DECLARED_PATH_CHARS {
                continue;
            }
            let path = resolve_declared_path(cwd, declared);
            if !seen.insert(path.clone()) {
                continue;
            }
            let before = if files.len() >= MAX_EDIT_FILES {
                None
            } else if path_missing(&path) {
                // Created-file detection needs no staging at all.
                Some(FileContentSnapshot::Missing)
            } else {
                if snapshot_dir.is_none() {
                    snapshot_dir =
                        create_snapshot_dir().map(|dir| Arc::new(SnapshotDirHandle(dir)));
                }
                // Staging unavailable (`None`): degrade to an explicit skip
                // instead of buffering file content in memory.
                snapshot_dir
                    .as_deref()
                    .map(|dir| capture_snapshot(&path, dir, files.len(), &mut budget))
            };
            files.push(DeclaredEditSnapshot {
                declared: declared.to_string(),
                path,
                before,
            });
        }
        Self {
            files,
            snapshot_dir,
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// Read the after-states and build bounded evidence. Runs at completion
    /// time (foreground) or on the job supervisor thread (background).
    pub(crate) fn compute_evidence(&self) -> FileEditsEvidence {
        let mut entries = Vec::with_capacity(self.files.len());
        for file in &self.files {
            let Some(before) = file.before.as_ref() else {
                entries.push(FileEditEntry {
                    path: file.declared.clone(),
                    op: FileEditOp::Skipped,
                    diff: None,
                    added_lines: 0,
                    removed_lines: 0,
                    truncated: false,
                    skip_reason: Some(SKIP_NOT_CAPTURED),
                });
                continue;
            };
            let before = match before {
                FileContentSnapshot::OnDisk { path } => read_snapshot(path).0,
                other => other.clone(),
            };
            let (after, _) = read_snapshot(&file.path);
            entries.push(build_entry(&file.declared, &before, &after));
        }
        FileEditsEvidence {
            edits: apply_payload_budget(entries),
        }
    }
}

fn resolve_declared_path(cwd: &Path, declared: &str) -> PathBuf {
    let path = Path::new(declared);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    }
}

/// Create the private per-capture staging directory under the system temp
/// dir. `None` disables disk staging; existing-file captures then degrade to
/// an explicit skip instead of buffering unbounded content in memory.
fn create_snapshot_dir() -> Option<PathBuf> {
    let seq = SNAPSHOT_DIR_SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "timem_edit_snapshots_{}_{}",
        std::process::id(),
        seq
    ));
    #[cfg(unix)]
    let created = {
        use std::os::unix::fs::DirBuilderExt;
        fs::DirBuilder::new().mode(0o700).create(&dir)
    };
    #[cfg(not(unix))]
    let created = fs::create_dir(&dir);
    created.ok().map(|()| dir)
}

/// Stage one file into the snapshot directory under the per-file/total disk
/// budget. Binary content is detected with a small prefix probe before any
/// copy; partial copies are removed on every failure path.
fn capture_snapshot(path: &Path, dir: &Path, slot: usize, budget: &mut u64) -> FileContentSnapshot {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(_) => return FileContentSnapshot::Unreadable,
    };
    if !metadata.is_file() {
        return FileContentSnapshot::Unreadable;
    }
    if metadata.len() > MAX_SNAPSHOT_FILE_BYTES as u64 || metadata.len() > *budget {
        return FileContentSnapshot::TooLarge;
    }
    let Ok(mut source) = fs::File::open(path) else {
        return FileContentSnapshot::Unreadable;
    };
    let mut probe = [0u8; SNAPSHOT_BINARY_PROBE_BYTES];
    let probed = match source.read(&mut probe) {
        Ok(probed) => probed,
        Err(_) => return FileContentSnapshot::Unreadable,
    };
    if probe[..probed].contains(&0) {
        return FileContentSnapshot::Binary;
    }
    let dest_path = dir.join(format!("before_{slot:02}"));
    let mut dest = match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&dest_path)
    {
        Ok(dest) => dest,
        Err(_) => return FileContentSnapshot::Unreadable,
    };
    if dest.write_all(&probe[..probed]).is_err() {
        let _ = fs::remove_file(&dest_path);
        return FileContentSnapshot::Unreadable;
    }
    let copied = match copy_stream(&mut source, &mut dest, MAX_SNAPSHOT_FILE_BYTES as u64 + 1) {
        Ok(copied) => copied,
        Err(_) => {
            let _ = fs::remove_file(&dest_path);
            return FileContentSnapshot::Unreadable;
        }
    };
    let staged = copied + probed as u64;
    if staged > MAX_SNAPSHOT_FILE_BYTES as u64 || staged > *budget {
        let _ = fs::remove_file(&dest_path);
        return FileContentSnapshot::TooLarge;
    }
    *budget -= staged;
    FileContentSnapshot::OnDisk { path: dest_path }
}

fn path_missing(path: &Path) -> bool {
    matches!(
        fs::metadata(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound
    )
}

/// Stream `source` into `dest` with a fixed chunk, failing once `limit`
/// bytes are exceeded so a racing append cannot balloon the staged copy.
fn copy_stream(source: &mut impl Read, dest: &mut impl Write, limit: u64) -> std::io::Result<u64> {
    let mut chunk = vec![0u8; SNAPSHOT_COPY_CHUNK_BYTES];
    let mut copied: u64 = 0;
    loop {
        let read = source.read(&mut chunk)?;
        if read == 0 {
            return Ok(copied);
        }
        if copied + read as u64 > limit {
            return Err(std::io::Error::other("snapshot exceeds byte cap"));
        }
        dest.write_all(&chunk[..read])?;
        copied += read as u64;
    }
}

/// Load one file into memory for diffing: used for after-states and for
/// loading staged `OnDisk` before-copies at evidence time. Oversized files
/// degrade to `TooLarge` instead of unbounded reads.
fn read_snapshot(path: &Path) -> (FileContentSnapshot, usize) {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return (FileContentSnapshot::Missing, 0)
        }
        Err(_) => return (FileContentSnapshot::Unreadable, 0),
    };
    if !metadata.is_file() {
        return (FileContentSnapshot::Unreadable, 0);
    }
    if metadata.len() > MAX_SNAPSHOT_FILE_BYTES as u64 {
        return (FileContentSnapshot::TooLarge, 0);
    }
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(_) => return (FileContentSnapshot::Unreadable, 0),
    };
    let mut handle = file.take(MAX_SNAPSHOT_FILE_BYTES as u64 + 1);
    let mut bytes = Vec::new();
    if handle.read_to_end(&mut bytes).is_err() {
        return (FileContentSnapshot::Unreadable, 0);
    }
    if bytes.len() as u64 > MAX_SNAPSHOT_FILE_BYTES as u64 {
        return (FileContentSnapshot::TooLarge, 0);
    }
    let size = bytes.len();
    if bytes.contains(&0) {
        return (FileContentSnapshot::Binary, size);
    }
    let Ok(text) = String::from_utf8(bytes) else {
        return (FileContentSnapshot::Binary, size);
    };
    (FileContentSnapshot::Lines(text), size)
}

fn split_diff_lines(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = text
        .split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line).to_string())
        .collect();
    if lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    lines
}

fn build_entry(
    declared: &str,
    before: &FileContentSnapshot,
    after: &FileContentSnapshot,
) -> FileEditEntry {
    let base = FileEditEntry {
        path: declared.to_string(),
        op: FileEditOp::Unchanged,
        diff: None,
        added_lines: 0,
        removed_lines: 0,
        truncated: false,
        skip_reason: None,
    };
    match (before, after) {
        (FileContentSnapshot::Lines(old_text), FileContentSnapshot::Lines(new_text)) => {
            if old_text == new_text {
                base
            } else {
                let old = split_diff_lines(old_text);
                let new = split_diff_lines(new_text);
                let mut entry = base;
                entry.op = FileEditOp::Modified;
                if old == new {
                    // Byte content changed while the line view did not (final
                    // newline or CRLF/LF rewrite). Expose a meta note instead
                    // of a false "unchanged" card; hunks would be empty.
                    entry.diff = Some(format!(
                        "--- a/{declared}\n+++ b/{declared}\n{WHITESPACE_ONLY_NOTE}\n"
                    ));
                } else {
                    entry.extend_from_diff(declared, &old, &new);
                }
                entry
            }
        }
        (FileContentSnapshot::Missing, FileContentSnapshot::Lines(new_text)) => {
            let mut entry = base;
            entry.op = FileEditOp::Created;
            let new = split_diff_lines(new_text);
            entry.extend_from_diff(declared, &[], &new);
            entry
        }
        (FileContentSnapshot::Lines(old_text), FileContentSnapshot::Missing) => {
            let mut entry = base;
            entry.op = FileEditOp::Deleted;
            let old = split_diff_lines(old_text);
            entry.extend_from_diff(declared, &old, &[]);
            entry
        }
        (FileContentSnapshot::Missing, FileContentSnapshot::Missing) => {
            // The declared path was absent before and after the call, so no
            // edit can have applied; surface it as skipped with an explicit
            // reason instead of an "unchanged" card that implies success.
            let mut entry = base;
            entry.op = FileEditOp::Skipped;
            entry.skip_reason = Some(SKIP_MISSING);
            entry
        }
        _ => {
            let mut entry = base;
            entry.op = FileEditOp::Skipped;
            entry.skip_reason = Some(if before.is_diffable() {
                after.skip_reason()
            } else {
                before.skip_reason()
            });
            entry
        }
    }
}

impl FileEditEntry {
    /// Fill diff fields for a created/modified/deleted file.
    fn extend_from_diff(&mut self, declared: &str, old: &[String], new: &[String]) {
        let ops = diff_lines(old, new);
        self.added_lines = ops
            .iter()
            .filter(|op| matches!(op, DiffOp::Insert(_)))
            .count() as u32;
        self.removed_lines = ops
            .iter()
            .filter(|op| matches!(op, DiffOp::Delete(_)))
            .count() as u32;
        let mut text = String::new();
        if self.op == FileEditOp::Created {
            text.push_str("--- /dev/null\n");
            text.push_str(&format!("+++ b/{declared}\n"));
        } else if self.op == FileEditOp::Deleted {
            text.push_str(&format!("--- a/{declared}\n"));
            text.push_str("+++ /dev/null\n");
        } else {
            text.push_str(&format!("--- a/{declared}\n+++ b/{declared}\n"));
        }
        for hunk in group_hunks(old, new, &ops) {
            let rendered = render_hunk(&hunk);
            if text.len() + rendered.len() > MAX_DIFF_FILE_BYTES {
                self.truncated = true;
                break;
            }
            text.push_str(&rendered);
        }
        self.diff = Some(text);
    }
}

fn entry_json(entry: &FileEditEntry) -> Value {
    json!({
        "path": entry.path,
        "op": entry.op.as_str(),
        "diff": entry.diff,
        "added_lines": entry.added_lines,
        "removed_lines": entry.removed_lines,
        "truncated": entry.truncated,
        "skip_reason": entry.skip_reason,
    })
}

/// Core-side payload projection for the finish topic (`edits` array). Bounded:
/// entries over the payload budget degrade to slim or skipped forms.
pub(crate) fn edits_payload(evidence: &FileEditsEvidence) -> Value {
    Value::Array(evidence.edits.iter().map(entry_json).collect())
}

fn apply_payload_budget(entries: Vec<FileEditEntry>) -> Vec<FileEditEntry> {
    let mut kept: Vec<FileEditEntry> = Vec::new();
    let mut used = 2usize; // "[]"
    for entry in entries {
        let full_len = entry_json(&entry).to_string().len() + 1;
        if used + full_len <= MAX_EDITS_PAYLOAD_BYTES {
            used += full_len;
            kept.push(entry);
            continue;
        }
        let mut slim = entry.clone();
        slim.diff = None;
        slim.truncated = true;
        let slim_len = entry_json(&slim).to_string().len() + 1;
        if used + slim_len <= MAX_EDITS_PAYLOAD_BYTES {
            used += slim_len;
            kept.push(slim);
            continue;
        }
        let minimal = FileEditEntry {
            path: clamp_chars(&entry.path, 128),
            op: FileEditOp::Skipped,
            diff: None,
            added_lines: 0,
            removed_lines: 0,
            truncated: false,
            skip_reason: Some(SKIP_BUDGET),
        };
        let minimal_len = entry_json(&minimal).to_string().len() + 1;
        if used + minimal_len > MAX_EDITS_PAYLOAD_BYTES {
            break;
        }
        used += minimal_len;
        kept.push(minimal);
    }
    kept
}

fn clamp_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    text.chars().take(max).collect()
}

// ---------------------------------------------------------------------------
// Line diffing: common prefix/suffix trim, patience anchors, small-section
// LCS, and a whole-file replace fallback. All paths are bounded.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DiffOp {
    Keep,
    Delete(usize),
    Insert(usize),
}

#[derive(Debug, Clone)]
struct Hunk {
    a_start: usize,
    a_len: usize,
    b_start: usize,
    b_len: usize,
    lines: Vec<(char, String)>,
}

/// Compute ordered diff ops over `old`/`new`; `Delete`/`Insert` carry the
/// absolute line index on their own side, and `Keep` positions are derived
/// from the op stream.
fn diff_lines(old: &[String], new: &[String]) -> Vec<DiffOp> {
    let mut interned: HashMap<&str, u32> = HashMap::new();
    let old_ids: Vec<u32> = old
        .iter()
        .map(|line| {
            let next = interned.len() as u32;
            *interned.entry(line.as_str()).or_insert(next)
        })
        .collect();
    let new_ids: Vec<u32> = new
        .iter()
        .map(|line| {
            let next = interned.len() as u32;
            *interned.entry(line.as_str()).or_insert(next)
        })
        .collect();
    let mut ops: Vec<DiffOp> = Vec::new();
    diff_id_ranges(&old_ids, &new_ids, 0, 0, 0, &mut ops);
    ops
}

/// Recursively diff the middle ranges starting at absolute offsets
/// `a_offset`/`b_offset`. Common prefix/suffix matches are emitted as `Keep`
/// ops; unsolvable middle regions become whole replace blocks.
fn diff_id_ranges(
    a: &[u32],
    b: &[u32],
    a_offset: usize,
    b_offset: usize,
    depth: usize,
    ops: &mut Vec<DiffOp>,
) {
    let mut a_cursor = 0usize;
    let mut b_cursor = 0usize;
    while a_cursor < a.len() && b_cursor < b.len() && a[a_cursor] == b[b_cursor] {
        ops.push(DiffOp::Keep);
        a_cursor += 1;
        b_cursor += 1;
    }
    let mut a_end = a.len();
    let mut b_end = b.len();
    while a_end > a_cursor && b_end > b_cursor && a[a_end - 1] == b[b_end - 1] {
        a_end -= 1;
        b_end -= 1;
    }
    let suffix_len = a.len() - a_end;
    let a_mid = &a[a_cursor..a_end];
    let b_mid = &b[b_cursor..b_end];
    let a_start = a_offset + a_cursor;
    let b_start = b_offset + b_cursor;
    if a_mid.is_empty() {
        for index in 0..b_mid.len() {
            ops.push(DiffOp::Insert(b_start + index));
        }
    } else if b_mid.is_empty() {
        for index in 0..a_mid.len() {
            ops.push(DiffOp::Delete(a_start + index));
        }
    } else if depth >= PATIENCE_DEPTH_CAP {
        push_whole_replace(a_mid.len(), b_mid.len(), a_start, b_start, ops);
    } else {
        let anchor_pairs = patience_anchors(a_mid, b_mid);
        let anchors = patience_lis(&anchor_pairs);
        if anchors.is_empty() {
            if a_mid.len() > PATIENCE_MIDDLE_LINE_CAP || b_mid.len() > PATIENCE_MIDDLE_LINE_CAP {
                push_whole_replace(a_mid.len(), b_mid.len(), a_start, b_start, ops);
            } else {
                match lcs_segment(a_mid, b_mid) {
                    Some(segment) => push_segment(&segment, a_start, b_start, ops),
                    None => push_whole_replace(a_mid.len(), b_mid.len(), a_start, b_start, ops),
                }
            }
        } else {
            let mut cursor_a = 0usize;
            let mut cursor_b = 0usize;
            for (anchor_a, anchor_b) in anchors {
                diff_id_ranges(
                    &a_mid[cursor_a..anchor_a],
                    &b_mid[cursor_b..anchor_b],
                    a_start + cursor_a,
                    b_start + cursor_b,
                    depth + 1,
                    ops,
                );
                ops.push(DiffOp::Keep);
                cursor_a = anchor_a + 1;
                cursor_b = anchor_b + 1;
            }
            // The trailing range continues the linear anchor chain: every step
            // consumes at least one line per side, so it stays at `depth`;
            // only nested sub-diffs accumulate toward PATIENCE_DEPTH_CAP.
            diff_id_ranges(
                &a_mid[cursor_a..],
                &b_mid[cursor_b..],
                a_start + cursor_a,
                b_start + cursor_b,
                depth,
                ops,
            );
        }
    }
    for _ in 0..suffix_len {
        ops.push(DiffOp::Keep);
    }
}

fn push_whole_replace(
    a_len: usize,
    b_len: usize,
    a_start: usize,
    b_start: usize,
    ops: &mut Vec<DiffOp>,
) {
    for index in 0..a_len {
        ops.push(DiffOp::Delete(a_start + index));
    }
    for index in 0..b_len {
        ops.push(DiffOp::Insert(b_start + index));
    }
}

/// Patience anchor candidates: line ids that occur exactly once on each side,
/// paired by index and returned sorted by old index.
fn patience_anchors(a_mid: &[u32], b_mid: &[u32]) -> Vec<(usize, usize)> {
    let mut count_a: HashMap<u32, usize> = HashMap::new();
    for id in a_mid {
        *count_a.entry(*id).or_insert(0) += 1;
    }
    let mut count_b: HashMap<u32, usize> = HashMap::new();
    for id in b_mid {
        *count_b.entry(*id).or_insert(0) += 1;
    }
    let mut position_b: HashMap<u32, usize> = HashMap::new();
    for (index, id) in b_mid.iter().enumerate() {
        position_b.entry(*id).or_insert(index);
    }
    let mut pairs: Vec<(usize, usize)> = Vec::new();
    for (index, id) in a_mid.iter().enumerate() {
        if count_a.get(id) != Some(&1) || count_b.get(id) != Some(&1) {
            continue;
        }
        if let Some(b_index) = position_b.get(id) {
            pairs.push((index, *b_index));
        }
    }
    pairs
}

/// Longest strictly increasing subsequence over anchor pairs ordered by old
/// index, comparing new indices. Returns the selected anchor pairs.
fn patience_lis(pairs: &[(usize, usize)]) -> Vec<(usize, usize)> {
    if pairs.is_empty() {
        return Vec::new();
    }
    let mut tails: Vec<(usize, usize)> = Vec::new();
    let mut prev: Vec<Option<usize>> = vec![None; pairs.len()];
    for (index, &(_, new_index)) in pairs.iter().enumerate() {
        let position = tails.partition_point(|&(_, tail)| tail < new_index);
        if position == tails.len() {
            tails.push((index, new_index));
        } else {
            tails[position] = (index, new_index);
        }
        if position > 0 {
            prev[index] = Some(tails[position - 1].0);
        }
    }
    let mut chain = Vec::with_capacity(tails.len());
    let mut cursor = tails.last().map(|&(index, _)| index);
    while let Some(index) = cursor {
        chain.push(pairs[index]);
        cursor = prev[index];
    }
    chain.reverse();
    chain
}

/// LCS via bounded DP; segment ops use relative indices. `None` when the
/// table would exceed the cap (caller falls back to whole replace).
fn lcs_segment(a_mid: &[u32], b_mid: &[u32]) -> Option<Vec<DiffOp>> {
    if a_mid.len() as u64 * b_mid.len() as u64 > DP_MAX_CELLS as u64 {
        return None;
    }
    let rows = a_mid.len() + 1;
    let columns = b_mid.len() + 1;
    let mut table = vec![0u32; rows * columns];
    for row in (0..a_mid.len()).rev() {
        for column in (0..b_mid.len()).rev() {
            table[row * columns + column] = if a_mid[row] == b_mid[column] {
                table[(row + 1) * columns + column + 1] + 1
            } else {
                table[(row + 1) * columns + column].max(table[row * columns + column + 1])
            };
        }
    }
    let mut segment = Vec::new();
    let mut row = 0usize;
    let mut column = 0usize;
    while row < a_mid.len() && column < b_mid.len() {
        if a_mid[row] == b_mid[column] {
            segment.push(DiffOp::Keep);
            row += 1;
            column += 1;
        } else if table[(row + 1) * columns + column] >= table[row * columns + column + 1] {
            segment.push(DiffOp::Delete(row));
            row += 1;
        } else {
            segment.push(DiffOp::Insert(column));
            column += 1;
        }
    }
    while row < a_mid.len() {
        segment.push(DiffOp::Delete(row));
        row += 1;
    }
    while column < b_mid.len() {
        segment.push(DiffOp::Insert(column));
        column += 1;
    }
    Some(segment)
}

/// Apply a small LCS segment (relative indices) with absolute offsets.
fn push_segment(segment: &[DiffOp], a_start: usize, b_start: usize, ops: &mut Vec<DiffOp>) {
    let mut offset_a = 0usize;
    let mut offset_b = 0usize;
    for op in segment {
        match *op {
            DiffOp::Keep => {
                ops.push(DiffOp::Keep);
                offset_a += 1;
                offset_b += 1;
            }
            DiffOp::Delete(_) => {
                ops.push(DiffOp::Delete(a_start + offset_a));
                offset_a += 1;
            }
            DiffOp::Insert(_) => {
                ops.push(DiffOp::Insert(b_start + offset_b));
                offset_b += 1;
            }
        }
    }
}

/// Precompute the absolute old/new line cursors before each op index, then
/// group changes into unified-diff hunks with context and 1-based numbering
/// (0 start for empty ranges, matching standard tools).
fn group_hunks(old: &[String], new: &[String], ops: &[DiffOp]) -> Vec<Hunk> {
    let mut a_positions = Vec::with_capacity(ops.len() + 1);
    let mut b_positions = Vec::with_capacity(ops.len() + 1);
    let (mut a_position, mut b_position) = (0usize, 0usize);
    for op in ops {
        a_positions.push(a_position);
        b_positions.push(b_position);
        match *op {
            DiffOp::Keep => {
                a_position += 1;
                b_position += 1;
            }
            DiffOp::Delete(_) => a_position += 1,
            DiffOp::Insert(_) => b_position += 1,
        }
    }
    a_positions.push(a_position);
    b_positions.push(b_position);

    let mut hunks: Vec<Hunk> = Vec::new();
    let mut change_cursor = 0usize;
    while change_cursor < ops.len() {
        while change_cursor < ops.len() && matches!(ops[change_cursor], DiffOp::Keep) {
            change_cursor += 1;
        }
        if change_cursor >= ops.len() {
            break;
        }
        let first = change_cursor;
        let mut last = first;
        let mut cursor = first + 1;
        while cursor < ops.len() {
            if matches!(ops[cursor], DiffOp::Keep) {
                cursor += 1;
                continue;
            }
            if cursor - last <= DIFF_CONTEXT_LINES * 2 {
                last = cursor;
                cursor += 1;
            } else {
                break;
            }
        }
        let context_start = first.saturating_sub(DIFF_CONTEXT_LINES);
        let context_end = (last + DIFF_CONTEXT_LINES + 1).min(ops.len());
        let a_raw_start = a_positions[context_start];
        let b_raw_start = b_positions[context_start];
        let mut hunk = Hunk {
            a_start: a_raw_start,
            a_len: 0,
            b_start: b_raw_start,
            b_len: 0,
            lines: Vec::new(),
        };
        for op_index in context_start..context_end {
            match ops[op_index] {
                DiffOp::Keep => {
                    hunk.lines.push((' ', old[a_positions[op_index]].clone()));
                    hunk.a_len += 1;
                    hunk.b_len += 1;
                }
                DiffOp::Delete(index) => {
                    hunk.lines.push(('-', old[index].clone()));
                    hunk.a_len += 1;
                }
                DiffOp::Insert(index) => {
                    hunk.lines.push(('+', new[index].clone()));
                    hunk.b_len += 1;
                }
            }
        }
        if hunk.a_len > 0 {
            hunk.a_start += 1;
        }
        if hunk.b_len > 0 {
            hunk.b_start += 1;
        }
        hunks.push(hunk);
        change_cursor = context_end;
    }
    hunks
}

fn render_hunk(hunk: &Hunk) -> String {
    let mut text = format!(
        "@@ -{},{} +{},{} @@\n",
        hunk.a_start, hunk.a_len, hunk.b_start, hunk.b_len
    );
    for (tag, content) in &hunk.lines {
        text.push(*tag);
        text.push_str(content);
        text.push('\n');
    }
    text
}

#[cfg(test)]
#[path = "../../../core/agent/tests/unit/file_edits_tests.rs"]
mod tests;
