use crate::response_protocol::ParsedAction;
use crate::AgentCore;
use encoding_rs::{Encoding, UTF_8};
use serde_json::Value;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

pub const DEFAULT_MAX_BYTES: usize = 32 * 1024;
pub const MAX_RETURN_BYTES: usize = 32 * 1024;
pub const MAX_SCAN_BYTES: u64 = 64 * 1024 * 1024;
const MAX_MATCH_BYTES: usize = 64 * 1024;

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

pub(crate) fn execute_action(core: &AgentCore, action: &ParsedAction) -> String {
    execute(core.current_prompt_cwd(), &action.raw_input)
}

pub fn execute(cwd: &Path, input: &Value) -> String {
    match execute_inner(cwd, input) {
        Ok(result) => result,
        Err(error) => error_result(input_path(input), &error),
    }
}

fn execute_inner(cwd: &Path, input: &Value) -> Result<String, ReadfileError> {
    let object = input.as_object().ok_or_else(|| {
        ReadfileError::new("invalid_input", "The readfile input must be an object.")
    })?;
    if let Some(field) = object.keys().find(|field| {
        !matches!(
            field.as_str(),
            "path" | "encoding" | "starter" | "ender" | "max_bytes"
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
    let starter = parse_selector(object.get("starter"), "starter")?;
    let ender = parse_selector(object.get("ender"), "ender")?;
    let requested_encoding = parse_encoding_label(object.get("encoding"))?;
    let candidate = resolve_path(cwd, path_text);
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

    let start = resolve_start(starter.as_ref(), text, &raw, encoding, bom_len)?;
    let window_end = floor_char_boundary(text, start.saturating_add(max_bytes).min(text.len()));
    let (requested_end, match_window_limited) = resolve_end(
        ender.as_ref(),
        text,
        &raw,
        encoding,
        bom_len,
        start,
        window_end,
    )?;
    if requested_end < start || (requested_end == start && ender.is_some() && !text.is_empty()) {
        return Err(ReadfileError::new(
            "range_before_start",
            "The inclusive end selector resolves before the start selector.",
        ));
    }
    let end = floor_char_boundary(text, requested_end.min(window_end));
    let limited =
        end < requested_end || match_window_limited || (ender.is_none() && end < text.len());
    let content = &text[start..end];
    let canonical = fs::canonicalize(&candidate).unwrap_or(candidate);

    Ok(format!(
        "Action result: readfile\nstatus: ok\npath: {}\nencoding: {}\nfile_bytes: {}\nstart_utf8_byte: {}\nend_utf8_byte_exclusive: {}\ncontent_bytes: {}\nlimited: {}\ncontent:\n{}",
        quote(&canonical.to_string_lossy()),
        encoding.name(),
        raw.len(),
        start,
        end,
        content.len(),
        limited,
        content
    ))
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
