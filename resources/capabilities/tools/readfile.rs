use crate::response_protocol::ParsedAction;
use crate::{ActionOutcome, AgentCore, ReadfileResultEvidence};
use encoding_rs::{Encoding, UTF_8};
use serde_json::Value;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
use std::sync::mpsc;
#[cfg(test)]
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const DEFAULT_MAX_BYTES: usize = 32 * 1024;
pub const MAX_RETURN_BYTES: usize = 32 * 1024;
pub const MAX_SCAN_BYTES: u64 = 64 * 1024 * 1024;
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_MATCH_BYTES: usize = 64 * 1024;

#[cfg(test)]
static TEST_PARALLEL_PROBES: Mutex<Vec<Arc<TestParallelReadProbeState>>> = Mutex::new(Vec::new());

#[cfg(test)]
struct TestParallelReadProbeState {
    root: PathBuf,
    delay: Duration,
    active: AtomicUsize,
    max_active: AtomicUsize,
}

#[cfg(test)]
pub(crate) struct TestParallelReadProbe {
    state: Arc<TestParallelReadProbeState>,
}

#[cfg(test)]
impl TestParallelReadProbe {
    pub(crate) fn max_active(&self) -> usize {
        self.state.max_active.load(AtomicOrdering::SeqCst)
    }
}

#[cfg(test)]
impl Drop for TestParallelReadProbe {
    fn drop(&mut self) {
        TEST_PARALLEL_PROBES
            .lock()
            .unwrap()
            .retain(|state| !Arc::ptr_eq(state, &self.state));
    }
}

#[cfg(test)]
pub(crate) fn install_test_parallel_read_probe(
    root: PathBuf,
    delay: Duration,
) -> TestParallelReadProbe {
    let state = Arc::new(TestParallelReadProbeState {
        root: fs::canonicalize(&root).unwrap_or(root),
        delay,
        active: AtomicUsize::new(0),
        max_active: AtomicUsize::new(0),
    });
    TEST_PARALLEL_PROBES.lock().unwrap().push(state.clone());
    TestParallelReadProbe { state }
}

#[cfg(test)]
struct ActiveTestReadProbe {
    state: Arc<TestParallelReadProbeState>,
}

#[cfg(test)]
impl Drop for ActiveTestReadProbe {
    fn drop(&mut self) {
        self.state.active.fetch_sub(1, AtomicOrdering::SeqCst);
    }
}

#[cfg(test)]
fn begin_test_parallel_read_probe(path: &Path) -> Option<ActiveTestReadProbe> {
    let canonical_path = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let state = TEST_PARALLEL_PROBES
        .lock()
        .unwrap()
        .iter()
        .rev()
        .find(|state| canonical_path.starts_with(&state.root))
        .cloned()?;

    let active = state.active.fetch_add(1, AtomicOrdering::SeqCst) + 1;
    state.max_active.fetch_max(active, AtomicOrdering::SeqCst);
    if !state.delay.is_zero() {
        std::thread::sleep(state.delay);
    }
    Some(ActiveTestReadProbe { state })
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Selector {
    Line(u64),
    Byte(u64),
    Match(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReadfileError {
    code: &'static str,
    message: String,
}

impl ReadfileError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReadfileSuccess {
    text: String,
    evidence: ReadfileResultEvidence,
}

fn readfile_error_type(code: &str) -> &'static str {
    match code {
        "path_not_found" => "NotFound",
        "permission_denied" => "PermissionDenied",
        "not_regular_file" => "NotRegularFile",
        "scan_limit_exceeded" => "LimitExceeded",
        "invalid_text_encoding"
        | "unsupported_encoding"
        | "encoding_bom_mismatch"
        | "byte_selector_splits_bom"
        | "byte_selector_splits_character" => "InvalidEncoding",
        "binary_file" => "BinaryFile",
        "start_line_not_found"
        | "start_byte_not_found"
        | "start_match_not_found"
        | "end_byte_not_found"
        | "end_match_not_found" => "SelectorNotFound",
        "open_failed" | "metadata_failed" | "read_failed" => "IoError",
        "builtin_action_panicked" => "InternalError",
        _ => "InvalidInput",
    }
}

fn readfile_error_evidence(
    path: impl Into<String>,
    error_type: impl Into<String>,
    content: impl Into<String>,
) -> ReadfileResultEvidence {
    ReadfileResultEvidence {
        path: path.into(),
        matcher: None,
        start_line: None,
        end_line: None,
        total_lines: None,
        encoding: None,
        file_bytes: None,
        content_bytes: None,
        limited: None,
        tail_out: None,
        content: content.into(),
        error_type: Some(error_type.into()),
    }
}

pub(crate) fn execute_action_outcome(core: &AgentCore, action: &ParsedAction) -> ActionOutcome {
    execute_with_timeout_outcome(
        core.current_prompt_cwd(),
        &action.raw_input,
        DEFAULT_TIMEOUT,
    )
}

pub(crate) fn execute_with_timeout_outcome(
    cwd: &Path,
    input: &Value,
    timeout: Duration,
) -> ActionOutcome {
    let cwd = cwd.to_path_buf();
    let input = input.clone();
    let path = input_path(&input).unwrap_or("<unknown>").to_string();
    let (sender, receiver) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let outcome = execute_outcome(&cwd, &input);
        let _ = sender.send(outcome);
    });

    match receiver.recv_timeout(timeout) {
        Ok(outcome) => outcome,
        Err(mpsc::RecvTimeoutError::Timeout) => {
            let message = "The readfile operation exceeded its execution timeout.";
            ActionOutcome::timeout(format!(
                "Action result: readfile\nstatus: error\npath: {}\nerror: timeout\nmessage: {}",
                quote(&path),
                quote(message)
            ))
            .with_readfile_result(ReadfileResultEvidence {
                error_type: None,
                ..readfile_error_evidence(path, "", message)
            })
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            let message =
                "The readfile worker failed internally. Timem isolated the failure and remains available.";
            ActionOutcome::failed(format!(
                "Action result: readfile\nstatus: error\npath: {}\nerror: builtin_action_panicked\nmessage: {}",
                quote(&path),
                quote(message)
            ))
            .with_readfile_result(readfile_error_evidence(
                path,
                "InternalError",
                message,
            ))
        }
    }
}

pub fn execute(cwd: &Path, input: &Value) -> String {
    execute_outcome(cwd, input).text
}

pub(crate) fn execute_outcome(cwd: &Path, input: &Value) -> ActionOutcome {
    match execute_inner(cwd, input) {
        Ok(result) => ActionOutcome::completed(result.text).with_readfile_result(result.evidence),
        Err(error) => {
            let path = input_path(input).unwrap_or("<unknown>");
            let error_type = readfile_error_type(error.code);
            ActionOutcome::failed(error_result(input_path(input), &error))
                .with_readfile_result(readfile_error_evidence(path, error_type, error.message))
        }
    }
}

fn execute_inner(cwd: &Path, input: &Value) -> Result<ReadfileSuccess, ReadfileError> {
    let object = input.as_object().ok_or_else(|| {
        ReadfileError::new("invalid_input", "The readfile input must be an object.")
    })?;
    if let Some(field) = object.keys().find(|field| {
        !matches!(
            field.as_str(),
            "path" | "encoding" | "starter" | "ender" | "max_bytes" | "tail_out"
        )
    }) {
        return Err(ReadfileError::new(
            "unsupported_input",
            format!("Unsupported readfile input field {}.", quote(field)),
        ));
    }
    let path_text = object
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| ReadfileError::new("path_required", "Provide a string `path`."))?;
    if path_text.trim().is_empty() {
        return Err(ReadfileError::new(
            "path_required",
            "Provide a non-empty file path.",
        ));
    }

    let max_bytes = parse_max_bytes(object.get("max_bytes"))?;
    let tail_out = parse_tail_out(object.get("tail_out"))?;
    let starter = parse_selector(object.get("starter"), "starter")?;
    let ender = parse_selector(object.get("ender"), "ender")?;
    let requested_encoding = parse_encoding_label(object.get("encoding"))?;
    let candidate = resolve_path(cwd, path_text);
    #[cfg(test)]
    let _test_parallel_probe = begin_test_parallel_read_probe(&candidate);
    let (mut file, metadata) = open_regular_file(&candidate)?;
    if metadata.len() > MAX_SCAN_BYTES {
        return Err(ReadfileError::new(
            "scan_limit_exceeded",
            format!(
                "The file is {} bytes; readfile scans at most {} bytes. Narrow or split the file before reading it.",
                metadata.len(),
                MAX_SCAN_BYTES
            ),
        ));
    }

    let mut raw = Vec::with_capacity(metadata.len().min(MAX_SCAN_BYTES) as usize);
    file.by_ref()
        .take(MAX_SCAN_BYTES + 1)
        .read_to_end(&mut raw)
        .map_err(|error| map_read_error("read_failed", &candidate, error))?;
    if raw.len() as u64 > MAX_SCAN_BYTES {
        return Err(ReadfileError::new(
            "scan_limit_exceeded",
            format!(
                "The file grew beyond the {} byte scan limit while it was being read.",
                MAX_SCAN_BYTES
            ),
        ));
    }

    let (encoding, bom_len) = select_encoding(&raw, requested_encoding)?;
    let (decoded, had_errors) = encoding.decode_with_bom_removal(&raw);
    if had_errors {
        return Err(ReadfileError::new(
            "invalid_text_encoding",
            format!(
                "The file is not valid {} text. Specify the correct encoding; replacement decoding is intentionally disabled.",
                encoding.name()
            ),
        ));
    }
    let text = decoded.as_ref();
    if text.contains('\0') {
        return Err(ReadfileError::new(
            "binary_file",
            "The decoded file contains NUL characters and is not treated as a normal text file.",
        ));
    }

    let requested_start = resolve_start(starter.as_ref(), text, &raw, encoding, bom_len)?;
    let selector_window_end = if tail_out {
        text.len()
    } else {
        floor_char_boundary(
            text,
            requested_start.saturating_add(max_bytes).min(text.len()),
        )
    };
    let (requested_end, match_window_limited) = resolve_end(
        ender.as_ref(),
        text,
        &raw,
        encoding,
        bom_len,
        requested_start,
        selector_window_end,
    )?;
    if requested_end < requested_start
        || (requested_end == requested_start && ender.is_some() && !text.is_empty())
    {
        return Err(ReadfileError::new(
            "range_before_start",
            "The inclusive end selector resolves before the start selector.",
        ));
    }

    let (start, end) = if tail_out {
        let earliest = requested_end.saturating_sub(max_bytes).max(requested_start);
        (
            ceil_char_boundary(text, earliest),
            floor_char_boundary(text, requested_end),
        )
    } else {
        (
            requested_start,
            floor_char_boundary(text, requested_end.min(selector_window_end)),
        )
    };
    let limited = start > requested_start
        || end < requested_end
        || match_window_limited
        || (ender.is_none() && !tail_out && end < text.len());
    let content = &text[start..end];
    let rendered_content = if limited {
        if tail_out {
            let truncated_words = text[requested_start..start].split_whitespace().count();
            format!(
                "!!!Too long, {truncated_words} words truncated before. Generate more actions if necessary !!!\n{content}"
            )
        } else {
            let truncated_words = text[end..requested_end].split_whitespace().count();
            format!(
                "{content}\n!!!Too long, {truncated_words} words truncated after. Generate more actions if necessary !!!"
            )
        }
    } else {
        content.to_string()
    };
    let canonical = fs::canonicalize(&candidate).unwrap_or(candidate);
    let canonical_path = canonical.to_string_lossy();
    let file_name = canonical
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_else(|| canonical_path.clone());
    let start_line = line_number_at(text, start);
    let end_line = selected_end_line(text, start, end);
    let output_heading = matcher_expression(starter.as_ref(), ender.as_ref())
        .map(|matcher| {
            format!(
                "{}, matcher '{}' line is [{}, {}]:",
                file_name,
                escape_matcher_expression(&matcher),
                start_line,
                end_line
            )
        })
        .unwrap_or_else(|| format!("{}, line [{}, {}]:", file_name, start_line, end_line));

    let result_text = format!(
        "Action result: readfile\nstatus: ok\npath: {}\nencoding: {}\nfile_bytes: {}\nstart_utf8_byte: {}\nend_utf8_byte_exclusive: {}\ncontent_bytes: {}\nlimited: {}\ntail_out: {}\n{}\ncontent:\n{}",
        quote(&canonical_path),
        encoding.name(),
        raw.len(),
        start,
        end,
        content.len(),
        limited,
        tail_out,
        output_heading,
        rendered_content
    );
    let total_lines = if text.is_empty() {
        0
    } else {
        selected_end_line(text, 0, text.len())
    };
    Ok(ReadfileSuccess {
        text: result_text,
        evidence: ReadfileResultEvidence {
            path: canonical_path.into_owned(),
            matcher: matcher_expression(starter.as_ref(), ender.as_ref()),
            start_line: Some(start_line),
            end_line: Some(end_line),
            total_lines: Some(total_lines),
            encoding: Some(encoding.name().to_string()),
            file_bytes: Some(raw.len() as u64),
            content_bytes: Some(content.len()),
            limited: Some(limited),
            tail_out: Some(tail_out),
            content: rendered_content,
            error_type: None,
        },
    })
}

fn parse_tail_out(value: Option<&Value>) -> Result<bool, ReadfileError> {
    let Some(value) = value else {
        return Ok(false);
    };
    value
        .as_bool()
        .ok_or_else(|| ReadfileError::new("invalid_tail_out", "`tail_out` must be a boolean."))
}

fn parse_max_bytes(value: Option<&Value>) -> Result<usize, ReadfileError> {
    let Some(value) = value else {
        return Ok(DEFAULT_MAX_BYTES);
    };
    let Some(value) = value.as_u64() else {
        return Err(ReadfileError::new(
            "invalid_max_bytes",
            "`max_bytes` must be a positive integer.",
        ));
    };
    if value == 0 || value > MAX_RETURN_BYTES as u64 {
        return Err(ReadfileError::new(
            "invalid_max_bytes",
            format!("`max_bytes` must be between 1 and {}.", MAX_RETURN_BYTES),
        ));
    }
    Ok(value as usize)
}

fn parse_encoding_label(value: Option<&Value>) -> Result<Option<&str>, ReadfileError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let Some(label) = value.as_str() else {
        return Err(ReadfileError::new(
            "invalid_encoding",
            "`encoding` must be a string label.",
        ));
    };
    let label = label.trim();
    if label.is_empty() {
        return Err(ReadfileError::new(
            "invalid_encoding",
            "`encoding` cannot be empty.",
        ));
    }
    Ok(Some(label))
}

fn parse_selector(value: Option<&Value>, field: &str) -> Result<Option<Selector>, ReadfileError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let Some(object) = value.as_object() else {
        return Err(ReadfileError::new(
            "invalid_selector",
            format!("`{field}` must be an object containing exactly one of `line_nr`, `byte_nr`, or `match`."),
        ));
    };
    if object.len() != 1 {
        return Err(ReadfileError::new(
            "invalid_selector",
            format!("`{field}` must contain exactly one selector field."),
        ));
    }
    let (kind, value) = object.iter().next().expect("one selector entry");
    match kind.as_str() {
        "line_nr" => {
            let line = value.as_u64().filter(|line| *line > 0).ok_or_else(|| {
                ReadfileError::new(
                    "invalid_selector",
                    format!("`{field}.line_nr` must be a positive 1-based integer."),
                )
            })?;
            Ok(Some(Selector::Line(line)))
        }
        "byte_nr" => {
            let byte = value.as_u64().ok_or_else(|| {
                ReadfileError::new(
                    "invalid_selector",
                    format!("`{field}.byte_nr` must be a non-negative integer."),
                )
            })?;
            Ok(Some(Selector::Byte(byte)))
        }
        "match" => {
            let pattern = value.as_str().ok_or_else(|| {
                ReadfileError::new(
                    "invalid_selector",
                    format!("`{field}.match` must be a string."),
                )
            })?;
            if pattern.is_empty() {
                return Err(ReadfileError::new(
                    "invalid_selector",
                    format!("`{field}.match` cannot be empty."),
                ));
            }
            if pattern.len() > MAX_MATCH_BYTES {
                return Err(ReadfileError::new(
                    "invalid_selector",
                    format!(
                        "`{field}.match` exceeds the {} byte limit.",
                        MAX_MATCH_BYTES
                    ),
                ));
            }
            Ok(Some(Selector::Match(pattern.to_string())))
        }
        _ => Err(ReadfileError::new(
            "invalid_selector",
            format!("`{field}` supports only `line_nr`, `byte_nr`, or `match`."),
        )),
    }
}

fn resolve_path(cwd: &Path, path: &str) -> PathBuf {
    let path = Path::new(path);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    }
}

fn open_regular_file(path: &Path) -> Result<(File, fs::Metadata), ReadfileError> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK);
    }
    let file = options
        .open(path)
        .map_err(|error| map_open_error(path, error))?;
    let metadata = file
        .metadata()
        .map_err(|error| map_read_error("metadata_failed", path, error))?;
    if !metadata.is_file() {
        return Err(ReadfileError::new(
            "not_regular_file",
            "The path is a directory or special file. readfile accepts regular files only.",
        ));
    }
    Ok((file, metadata))
}

fn select_encoding(
    raw: &[u8],
    requested: Option<&str>,
) -> Result<(&'static Encoding, usize), ReadfileError> {
    let bom = Encoding::for_bom(raw);
    let encoding = if let Some(label) = requested {
        let requested = Encoding::for_label(label.as_bytes()).ok_or_else(|| {
            ReadfileError::new(
                "unsupported_encoding",
                format!("Unsupported encoding label {}.", quote(label)),
            )
        })?;
        if let Some((bom_encoding, _)) = bom {
            if requested != bom_encoding {
                return Err(ReadfileError::new(
                    "encoding_bom_mismatch",
                    format!(
                        "The file BOM indicates {}, but `encoding` requested {}.",
                        bom_encoding.name(),
                        requested.name()
                    ),
                ));
            }
        }
        requested
    } else {
        bom.map(|(encoding, _)| encoding).unwrap_or(UTF_8)
    };
    let bom_len = bom
        .filter(|(bom_encoding, _)| *bom_encoding == encoding)
        .map(|(_, length)| length)
        .unwrap_or(0);
    Ok((encoding, bom_len))
}

fn resolve_start(
    selector: Option<&Selector>,
    text: &str,
    raw: &[u8],
    encoding: &'static Encoding,
    bom_len: usize,
) -> Result<usize, ReadfileError> {
    match selector {
        None => Ok(0),
        Some(Selector::Line(line)) => line_start(text, *line).ok_or_else(|| {
            ReadfileError::new(
                "start_line_not_found",
                format!("Starter line {line} does not exist."),
            )
        }),
        Some(Selector::Byte(byte)) => {
            let byte = usize::try_from(*byte).map_err(|_| {
                ReadfileError::new("start_byte_not_found", "Starter byte is out of range.")
            })?;
            if byte >= raw.len() {
                return Err(ReadfileError::new(
                    "start_byte_not_found",
                    format!(
                        "Starter byte {byte} is outside the {} byte file.",
                        raw.len()
                    ),
                ));
            }
            raw_boundary_to_utf8(raw, encoding, bom_len, byte, "starter")
        }
        Some(Selector::Match(pattern)) => text.find(pattern).ok_or_else(|| {
            ReadfileError::new(
                "start_match_not_found",
                format!("Starter match {} was not found.", quote(pattern)),
            )
        }),
    }
}

#[allow(clippy::too_many_arguments)]
fn resolve_end(
    selector: Option<&Selector>,
    text: &str,
    raw: &[u8],
    encoding: &'static Encoding,
    bom_len: usize,
    start: usize,
    window_end: usize,
) -> Result<(usize, bool), ReadfileError> {
    match selector {
        None => Ok((text.len(), false)),
        Some(Selector::Line(line)) => {
            // An inclusive end line beyond EOF means "through the actual last
            // line". This keeps bounded range requests useful when callers do
            // not know the file's exact line count.
            Ok((line_end(text, *line).unwrap_or(text.len()), false))
        }
        Some(Selector::Byte(byte)) => {
            let byte = usize::try_from(*byte).map_err(|_| {
                ReadfileError::new("end_byte_not_found", "Ender byte is out of range.")
            })?;
            if byte >= raw.len() {
                return Err(ReadfileError::new(
                    "end_byte_not_found",
                    format!("Ender byte {byte} is outside the {} byte file.", raw.len()),
                ));
            }
            raw_boundary_to_utf8(raw, encoding, bom_len, byte + 1, "ender").map(|end| (end, false))
        }
        Some(Selector::Match(pattern)) => {
            let window = &text[start..window_end];
            let offset = window.rfind(pattern).ok_or_else(|| {
                ReadfileError::new(
                    "end_match_not_found",
                    format!(
                        "Ender match {} was not found after the start within the max_bytes window.",
                        quote(pattern)
                    ),
                )
            })?;
            Ok((start + offset + pattern.len(), window_end < text.len()))
        }
    }
}

fn raw_boundary_to_utf8(
    raw: &[u8],
    encoding: &'static Encoding,
    bom_len: usize,
    boundary: usize,
    field: &str,
) -> Result<usize, ReadfileError> {
    if boundary == 0 {
        return Ok(0);
    }
    if boundary < bom_len {
        return Err(ReadfileError::new(
            "byte_selector_splits_bom",
            format!(
                "The {field} byte selector points inside the {} byte BOM.",
                bom_len
            ),
        ));
    }
    let (prefix, had_errors) = encoding.decode_without_bom_handling(&raw[bom_len..boundary]);
    if had_errors {
        return Err(ReadfileError::new(
            "byte_selector_splits_character",
            format!("The {field} byte selector splits an encoded character."),
        ));
    }
    Ok(prefix.len())
}

fn line_start(text: &str, line: u64) -> Option<usize> {
    if line == 1 {
        return Some(0);
    }
    let mut current = 1;
    let mut start = 0;
    while let Some((end, terminated)) = next_line_end(text, start) {
        if !terminated {
            return None;
        }
        current += 1;
        start = end;
        if current == line {
            return Some(start);
        }
    }
    None
}

fn line_end(text: &str, line: u64) -> Option<usize> {
    let start = line_start(text, line)?;
    next_line_end(text, start).map(|(end, _)| end)
}

fn escape_matcher_expression(matcher: &str) -> String {
    matcher
        .replace('\\', "\\\\")
        .replace('\'', "\\'")
        .replace('\r', "\\r")
        .replace('\n', "\\n")
        .replace('\t', "\\t")
}

fn matcher_expression(starter: Option<&Selector>, ender: Option<&Selector>) -> Option<String> {
    let starter = match starter {
        Some(Selector::Match(pattern)) => Some(pattern.as_str()),
        _ => None,
    };
    let ender = match ender {
        Some(Selector::Match(pattern)) => Some(pattern.as_str()),
        _ => None,
    };

    match (starter, ender) {
        (Some(starter), Some(ender)) if starter == ender => Some(starter.to_string()),
        (Some(starter), Some(ender)) => Some(format!("{starter} ... {ender}")),
        (Some(starter), None) => Some(starter.to_string()),
        (None, Some(ender)) => Some(ender.to_string()),
        (None, None) => None,
    }
}

fn line_number_at(text: &str, offset: usize) -> u64 {
    let offset = offset.min(text.len());
    let bytes = text.as_bytes();
    let mut line = 1;
    let mut index = 0;

    while index < offset {
        match bytes[index] {
            b'\r' if bytes.get(index + 1) == Some(&b'\n') => {
                line += 1;
                index += 2;
            }
            b'\r' | b'\n' => {
                line += 1;
                index += 1;
            }
            _ => index += 1,
        }
    }
    line
}

fn selected_end_line(text: &str, start: usize, end: usize) -> u64 {
    if end <= start {
        return line_number_at(text, start);
    }

    let mut last_content_byte = end - 1;
    if text.as_bytes().get(last_content_byte) == Some(&b'\n')
        && last_content_byte > start
        && text.as_bytes().get(last_content_byte - 1) == Some(&b'\r')
    {
        last_content_byte -= 1;
    }
    line_number_at(text, last_content_byte)
}

fn next_line_end(text: &str, start: usize) -> Option<(usize, bool)> {
    if start > text.len() {
        return None;
    }
    let bytes = text.as_bytes();
    let mut index = start;
    while index < bytes.len() {
        match bytes[index] {
            b'\n' => return Some((index + 1, true)),
            b'\r' if bytes.get(index + 1) == Some(&b'\n') => {
                return Some((index + 2, true));
            }
            b'\r' => return Some((index + 1, true)),
            _ => index += 1,
        }
    }
    Some((text.len(), false))
}

fn ceil_char_boundary(text: &str, mut index: usize) -> usize {
    while index < text.len() && !text.is_char_boundary(index) {
        index += 1;
    }
    index
}

fn floor_char_boundary(text: &str, mut index: usize) -> usize {
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn map_open_error(path: &Path, error: io::Error) -> ReadfileError {
    let (code, message) = match error.kind() {
        io::ErrorKind::NotFound => ("path_not_found", "The file does not exist."),
        io::ErrorKind::PermissionDenied => ("permission_denied", "Permission denied."),
        _ => ("open_failed", "The file could not be opened."),
    };
    ReadfileError::new(
        code,
        format!("{} Path: {}", message, quote(&path.to_string_lossy())),
    )
}

fn map_read_error(code: &'static str, path: &Path, error: io::Error) -> ReadfileError {
    let message = if error.kind() == io::ErrorKind::PermissionDenied {
        "Permission denied while reading the file."
    } else {
        "The file could not be read completely."
    };
    ReadfileError::new(
        code,
        format!("{} Path: {}", message, quote(&path.to_string_lossy())),
    )
}

fn error_result(path: Option<&str>, error: &ReadfileError) -> String {
    let mut output = format!(
        "Action result: readfile\nstatus: error\nerror: {}\nmessage: {}",
        error.code,
        quote(&error.message)
    );
    if let Some(path) = path {
        output.push_str("\npath: ");
        output.push_str(&quote(path));
    }
    output
}

fn input_path(input: &Value) -> Option<&str> {
    input.get("path").and_then(Value::as_str)
}

fn quote(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"<unprintable>\"".to_string())
}

#[cfg(test)]
#[path = "../../../agent_core/tests/unit/capability_tool_readfile_tests.rs"]
mod tests;
