//! Bounded SSE content decoding. Display evidence is not protocol acceptance.
use serde_json::Value;

const MAX_EVENT_BYTES: usize = 1024 * 1024;
const MAX_SSE_EVENT_BYTES: usize = 4 * 1024 * 1024;
/// Latest-reasoning window carried by preview frames. The full chain stays
/// transient; only this bounded tail crosses the UI boundary, so preview
/// payloads and DOM work stay flat while thinking grows.
const MAX_THINKING_TAIL_CHARS: usize = 4096;

/// Provider-neutral reasoning delta extraction. Unknown/custom event shapes
/// intentionally return `None` so telemetry can degrade without affecting the
/// accepted model response.
pub fn reasoning_delta(event: &Value) -> Option<&str> {
    event
        .pointer("/choices/0/delta/reasoning_content")
        .or_else(|| event.pointer("/choices/0/delta/reasoning"))
        .or_else(|| event.pointer("/choices/0/delta/thinking"))
        .or_else(|| {
            matches!(
                event.get("type").and_then(Value::as_str),
                Some(
                    "response.reasoning_summary_text.delta"
                        | "response.reasoning_text.delta"
                        | "response.reasoning.delta"
                )
            )
            .then(|| event.get("delta"))
            .flatten()
        })
        .or_else(|| {
            (event.get("type").and_then(Value::as_str) == Some("content_block_delta"))
                .then(|| {
                    event
                        .pointer("/delta/thinking")
                        .or_else(|| event.pointer("/delta/reasoning"))
                        .or_else(|| {
                            matches!(
                                event.pointer("/delta/type").and_then(Value::as_str),
                                Some("thinking_delta" | "reasoning_delta")
                            )
                            .then(|| event.pointer("/delta/text"))
                            .flatten()
                        })
                })
                .flatten()
        })
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
}

pub fn reasoning_token_count(event: &Value) -> Option<u64> {
    [
        "/usage/completion_tokens_details/reasoning_tokens",
        "/usage/output_tokens_details/reasoning_tokens",
        "/usage/reasoning_tokens",
        "/response/usage/output_tokens_details/reasoning_tokens",
        "/response/usage/reasoning_tokens",
        "/message/usage/reasoning_tokens",
        "/usage/thinking_tokens",
    ]
    .into_iter()
    .find_map(|path| event.pointer(path).and_then(Value::as_u64))
}

pub fn public_text_delta(event: &Value) -> Option<&str> {
    event
        .pointer("/choices/0/delta/content")
        .or_else(|| {
            (event.get("type").and_then(Value::as_str) == Some("response.output_text.delta"))
                .then(|| event.get("delta"))
                .flatten()
        })
        .or_else(|| {
            (event.get("type").and_then(Value::as_str) == Some("content_block_delta"))
                .then(|| {
                    matches!(
                        event.pointer("/delta/type").and_then(Value::as_str),
                        Some("text_delta")
                    )
                    .then(|| event.pointer("/delta/text"))
                    .flatten()
                })
                .flatten()
        })
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
}

/// Decodes public choice-zero content, never reasoning or tool arguments.
#[derive(Default)]
pub struct OpenAiContentStream {
    line: Vec<u8>,
    data: Vec<u8>,
    pending_cr: bool,
    stopped: bool,
    event_count: usize,
    failure_sizes: Option<(usize, usize)>,
}

impl OpenAiContentStream {
    pub fn push(&mut self, bytes: &[u8], emit: &mut dyn FnMut(&str)) -> Result<(), String> {
        self.push_events(bytes, &mut |event| {
            if let Some(content) = event
                .pointer("/choices/0/delta/content")
                .and_then(Value::as_str)
            {
                if !content.is_empty() {
                    emit(content);
                }
            }
        })
    }

    pub fn push_events(
        &mut self,
        bytes: &[u8],
        emit: &mut dyn FnMut(&Value),
    ) -> Result<(), String> {
        if self.stopped {
            return Ok(());
        }
        for &byte in bytes {
            if self.pending_cr {
                self.pending_cr = false;
                if byte == b'\n' {
                    continue;
                }
            }
            if byte == b'\r' || byte == b'\n' {
                let sizes = (self.line.len(), self.data.len());
                if let Err(error) = self.finish_line(emit) {
                    self.failure_sizes = Some(sizes);
                    self.stopped = true;
                    self.line.clear();
                    self.data.clear();
                    return Err(error);
                }
                self.pending_cr = byte == b'\r';
                if self.stopped {
                    break;
                }
            } else {
                if self.line.len().saturating_add(self.data.len()) >= MAX_SSE_EVENT_BYTES {
                    self.failure_sizes = Some((self.line.len(), self.data.len()));
                    self.stopped = true;
                    self.line.clear();
                    self.data.clear();
                    return Err("model_stream_event_too_large".into());
                }
                self.line.push(byte);
            }
        }
        Ok(())
    }

    pub(crate) fn diagnostics(&self) -> Value {
        let (line_bytes, event_data_bytes) = self
            .failure_sizes
            .unwrap_or((self.line.len(), self.data.len()));
        serde_json::json!({
            "event_count": self.event_count,
            "line_bytes": line_bytes,
            "event_data_bytes": event_data_bytes,
            "event_limit_bytes": MAX_SSE_EVENT_BYTES,
        })
    }

    fn finish_line(&mut self, emit: &mut dyn FnMut(&Value)) -> Result<(), String> {
        let line = std::mem::take(&mut self.line);
        if line.is_empty() {
            if self.data.is_empty() {
                return Ok(());
            }
            let data = std::mem::take(&mut self.data);
            let text =
                std::str::from_utf8(&data).map_err(|_| "invalid_model_stream_utf8".to_string())?;
            if text.trim() == "[DONE]" {
                self.stopped = true;
                return Ok(());
            }
            let event: Value =
                serde_json::from_str(text).map_err(|_| "invalid_model_stream_event".to_string())?;
            self.event_count = self.event_count.saturating_add(1);
            emit(&event);
        } else if let Some(data) = line.strip_prefix(b"data:") {
            let data = data.strip_prefix(b" ").unwrap_or(data);
            self.data.extend_from_slice(data);
            self.data.push(b'\n');
        }
        Ok(())
    }
}

/// Incremental JSON lexical filter. Only root-level public text string values
/// are emitted. Full JSON/schema validation remains the response parser's job.
#[derive(Default)]
pub struct JsonPublicTextStream {
    depth: usize,
    in_string: bool,
    escaped: bool,
    token: String,
    key: String,
    key_expected: bool,
    value_expected: bool,
    public_value: bool,
    failed: bool,
    started: bool,
    total_bytes: usize,
}

impl JsonPublicTextStream {
    pub fn push(&mut self, text: &str, emit: &mut dyn FnMut(&str)) -> Result<(), String> {
        if self.failed {
            return Ok(());
        }
        self.total_bytes = self.total_bytes.saturating_add(text.len());
        if self.total_bytes > MAX_EVENT_BYTES {
            self.failed = true;
            self.token.clear();
            return Err("model_preview_text_too_large".into());
        }
        for ch in text.chars() {
            if !self.started {
                if ch.is_ascii_whitespace() {
                    continue;
                }
                if ch != '{' {
                    self.failed = true;
                    return Err("model_preview_expected_json_object".into());
                }
                self.started = true;
            } else if self.depth == 0 {
                if ch.is_ascii_whitespace() {
                    continue;
                }
                self.failed = true;
                return Err("model_preview_trailing_json".into());
            }
            if self.in_string {
                if ch == '"' && !self.escaped {
                    self.in_string = false;
                    if self.key_expected && self.depth == 1 {
                        self.key = serde_json::from_str::<String>(&format!("\"{}\"", self.token))
                            .map_err(|_| "invalid_preview_json_string".to_string())?;
                        self.key_expected = false;
                    } else if self.public_value {
                        let value = serde_json::from_str::<String>(&format!("\"{}\"", self.token))
                            .map_err(|_| "invalid_preview_json_string".to_string())?;
                        emit(&value);
                    }
                    self.token.clear();
                    self.public_value = false;
                    continue;
                }
                if self.key_expected || self.public_value {
                    if self.token.len() + ch.len_utf8() > MAX_EVENT_BYTES {
                        self.failed = true;
                        self.token.clear();
                        return Err("model_preview_text_too_large".into());
                    }
                    self.token.push(ch);
                    // Emit only fully decoded string prefixes. Hold escapes (including
                    // surrogate pairs) until serde can decode them without replacement.
                    if self.public_value {
                        if let Ok(value) =
                            serde_json::from_str::<String>(&format!("\"{}\"", self.token))
                        {
                            emit(&value);
                            self.token.clear();
                        }
                    }
                }
                self.escaped = ch == '\\' && !self.escaped;
                continue;
            }
            match ch {
                '"' => {
                    self.in_string = true;
                    self.escaped = false;
                    self.token.clear();
                    self.public_value = self.depth == 1
                        && self.value_expected
                        && matches!(self.key.as_str(), "free_talk" | "final_answer");
                    self.value_expected = false;
                }
                '{' | '[' => {
                    self.depth += 1;
                    self.key_expected = self.depth == 1 && ch == '{';
                    self.value_expected = false;
                }
                '}' | ']' => {
                    self.depth = self.depth.saturating_sub(1);
                    self.value_expected = false;
                }
                ':' if self.depth == 1 => self.value_expected = true,
                ',' if self.depth == 1 => {
                    self.key_expected = true;
                    self.value_expected = false;
                    self.key.clear();
                }
                _ => {}
            }
        }
        Ok(())
    }
}

/// UI-neutral provisional response state. It never changes Turn lifecycle.
/// A settled intermediate remains visible until a later attempt supplies text.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreviewStatus {
    Streaming,
    Intermediate,
    Final,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ResponsePreview {
    pub attempt: u64,
    pub revision: u64,
    pub text: String,
    pub status: PreviewStatus,
}

#[derive(Default)]
pub struct ResponsePreviewState {
    attempt: u64,
    revision: u64,
    accepting: bool,
    visible: Option<ResponsePreview>,
}

impl ResponsePreviewState {
    pub fn begin(&mut self) -> u64 {
        self.attempt = self.attempt.saturating_add(1);
        self.accepting = true;
        self.attempt
    }

    /// Clears the current attempt's streamed text (the optimistic finish
    /// summary supersedes a same-attempt progress note as the answer draft).
    pub fn replace_current(&mut self, attempt: u64) {
        if attempt != self.attempt {
            return;
        }
        if self
            .visible
            .as_ref()
            .is_some_and(|preview| preview.attempt == attempt && !preview.text.is_empty())
        {
            self.revision = self.revision.saturating_add(1);
            self.visible = None;
        }
    }

    /// Any destination's first visible text ends the prior response preview.
    pub fn replace_prior(&mut self, attempt: u64) {
        if attempt == self.attempt
            && self.accepting
            && self
                .visible
                .as_ref()
                .is_some_and(|visible| visible.attempt != attempt)
        {
            self.visible = None;
            self.revision = self.revision.saturating_add(1);
        }
    }

    pub fn visible(&self) -> Option<&ResponsePreview> {
        self.visible.as_ref()
    }

    pub fn append(&mut self, attempt: u64, text: &str) -> Result<bool, String> {
        if !self.accepting || attempt != self.attempt || text.is_empty() {
            return Ok(false);
        }
        let existing_len = self
            .visible
            .as_ref()
            .filter(|preview| preview.attempt == attempt)
            .map_or(0, |preview| preview.text.len());
        if existing_len.saturating_add(text.len()) > MAX_EVENT_BYTES {
            self.retract(attempt);
            return Err("model_preview_text_too_large".into());
        }
        self.revision = self.revision.saturating_add(1);
        let preview = self.visible.get_or_insert_with(|| ResponsePreview {
            attempt,
            revision: 0,
            text: String::new(),
            status: PreviewStatus::Streaming,
        });
        if preview.attempt != attempt {
            preview.attempt = attempt;
            preview.text.clear();
        }
        preview.text.push_str(text);
        preview.revision = self.revision;
        preview.status = PreviewStatus::Streaming;
        Ok(true)
    }

    /// Call only after the authoritative complete response parser accepts it.
    pub fn settle(&mut self, attempt: u64, final_response: bool) -> bool {
        if !self.accepting || attempt != self.attempt {
            return false;
        }
        self.accepting = false;
        let Some(preview) = self.visible.as_mut().filter(|p| p.attempt == attempt) else {
            return false;
        };
        self.revision = self.revision.saturating_add(1);
        preview.revision = self.revision;
        preview.status = if final_response {
            PreviewStatus::Final
        } else {
            PreviewStatus::Intermediate
        };
        true
    }

    pub fn retract(&mut self, attempt: u64) -> bool {
        if attempt != self.attempt {
            return false;
        }
        self.accepting = false;
        if self.visible.as_ref().is_some_and(|p| p.attempt == attempt) {
            self.revision = self.revision.saturating_add(1);
            self.visible = None;
            return true;
        }
        false
    }

    pub fn cancel(&mut self) {
        self.accepting = false;
        self.revision = self.revision.saturating_add(1);
        self.visible = None;
    }
}

/// XML public-field filter with a separate allowlisted interim Chat destination.
#[derive(Default)]
pub struct XmlPublicTextStream {
    pending: String,
    stack: Vec<String>,
    cdata: bool,
    failed: bool,
    total_bytes: usize,
}
impl XmlPublicTextStream {
    pub fn push(&mut self, text: &str, emit: &mut dyn FnMut(&str)) -> Result<(), String> {
        self.push_typed(text, &mut |target, text| {
            if target == PublicTextTarget::Response {
                emit(text);
            }
        })
    }
    fn target(&self) -> Option<PublicTextTarget> {
        if self.stack.len() == 2
            && self.stack[0] == "assistant"
            && matches!(self.stack[1].as_str(), "free_talk" | "final_answer")
        {
            return Some(PublicTextTarget::Response);
        }
        None
    }
    pub fn push_typed(
        &mut self,
        text: &str,
        emit: &mut dyn FnMut(PublicTextTarget, &str),
    ) -> Result<(), String> {
        if self.failed {
            return Ok(());
        }
        self.total_bytes = self.total_bytes.saturating_add(text.len());
        if self.total_bytes > MAX_EVENT_BYTES {
            self.failed = true;
            return Err("model_preview_text_too_large".into());
        }
        self.pending.push_str(text);
        let result = self.drain(emit);
        if result.is_err() {
            self.failed = true;
            self.pending.clear();
        }
        result
    }
    fn drain(&mut self, emit: &mut dyn FnMut(PublicTextTarget, &str)) -> Result<(), String> {
        while !self.pending.is_empty() {
            if self.cdata {
                if self.pending.starts_with("]]>") {
                    self.pending.drain(..3);
                    self.cdata = false;
                    continue;
                }
                if "]]>".starts_with(self.pending.as_str()) {
                    break;
                }
                let width = self.pending.chars().next().unwrap().len_utf8();
                if let Some(target) = self.target() {
                    emit(target, &self.pending[..width]);
                }
                self.pending.drain(..width);
                continue;
            }
            if self.pending.starts_with('<') {
                if "<![CDATA[".starts_with(self.pending.as_str()) {
                    break;
                }
                if self.pending.starts_with("<![CDATA[") {
                    self.pending.drain(..9);
                    self.cdata = true;
                    continue;
                }
                let Some(end) = self.pending.find('>') else {
                    break;
                };
                let raw = self.pending[1..end].trim();
                if let Some(close) = raw.strip_prefix('/') {
                    if self.stack.pop().as_deref()
                        != Some(close.trim().to_ascii_lowercase().as_str())
                    {
                        return Err("model_preview_xml_mismatched_tag".into());
                    }
                } else {
                    let name = raw
                        .trim_end_matches('/')
                        .split_whitespace()
                        .next()
                        .unwrap_or("")
                        .to_ascii_lowercase();
                    if name.is_empty() || (self.stack.is_empty() && name != "assistant") {
                        return Err("model_preview_expected_xml_root".into());
                    }
                    if !raw.ends_with('/') {
                        if self.stack.len() >= 64 {
                            return Err("model_preview_xml_depth".into());
                        }
                        self.stack.push(name);
                    }
                }
                self.pending.drain(..=end);
                continue;
            }
            if self.target().is_some() && self.pending.starts_with('&') {
                let Some(end) = self.pending.find(';') else {
                    break;
                };
                let entity = &self.pending[..=end];
                emit(
                    self.target().unwrap(),
                    match entity {
                        "&lt;" => "<",
                        "&gt;" => ">",
                        "&quot;" => "\"",
                        "&apos;" => "'",
                        "&amp;" => "&",
                        _ => entity,
                    },
                );
                self.pending.drain(..=end);
                continue;
            }
            let ch = self.pending.chars().next().unwrap();
            if self.stack.is_empty() && !ch.is_whitespace() {
                return Err("model_preview_expected_xml_root".into());
            }
            if let Some(target) = self.target() {
                emit(target, &self.pending[..ch.len_utf8()]);
            }
            self.pending.drain(..ch.len_utf8());
        }
        Ok(())
    }
}

/// Typed provisional destinations for public text preview. `FinishSummary`
/// marks text extracted from a streaming `task_finished` tool-call argument:
/// it renders like response text but opts the preview into optimistic finish
/// semantics (deferred settle, retriable on intercepted finishes).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PublicTextTarget {
    Response,
    FinishSummary,
}

/// Incremental extractor for a `task_finished` summary from streaming native
/// tool-call argument fragments. It only unescapes and forwards summary string
/// bytes; structural JSON validation stays with the authoritative response
/// parser, and any unexpected shape simply stops emitting.
#[derive(Default)]
pub struct NativeFinishSummaryStream {
    calls: Vec<(String, Option<String>)>,
    parser: SummaryArgumentParser,
}

impl NativeFinishSummaryStream {
    /// Feeds one transport stream event and emits newly resolved summary text.
    pub fn push_event(&mut self, event: &serde_json::Value, emit: &mut dyn FnMut(&str)) {
        for (key, name, arguments) in Self::event_fragments(event) {
            if let Some(name) = name {
                match self.calls.iter_mut().find(|(existing, _)| existing == &key) {
                    Some((_, slot)) => *slot = Some(name),
                    None => self.calls.push((key.clone(), Some(name))),
                }
            }
            if arguments.is_empty() {
                continue;
            }
            let is_finish_call = self
                .calls
                .iter()
                .find(|(existing, _)| existing == &key)
                .and_then(|(_, name)| name.as_deref())
                == Some("task_finished");
            if is_finish_call {
                self.parser.push(&arguments, emit);
            }
        }
    }

    /// Returns `(call key, optional function name, argument fragment)` tuples.
    fn event_fragments(event: &serde_json::Value) -> Vec<(String, Option<String>, String)> {
        let mut fragments = Vec::new();
        if let Some(tool_calls) = event
            .pointer("/choices/0/delta/tool_calls")
            .and_then(|value| value.as_array())
        {
            for call in tool_calls {
                let Some(index) = call.get("index").and_then(|value| value.as_u64()) else {
                    continue;
                };
                let function = call.pointer("/function");
                let name = function
                    .and_then(|value| value.get("name"))
                    .and_then(|value| value.as_str())
                    .filter(|name| !name.is_empty())
                    .map(str::to_string);
                let arguments = function
                    .and_then(|value| value.get("arguments"))
                    .and_then(|value| value.as_str())
                    .unwrap_or_default()
                    .to_string();
                fragments.push((index.to_string(), name, arguments));
            }
            return fragments;
        }
        let event_type = event.get("type").and_then(|value| value.as_str());
        if event_type == Some("response.output_item.added") {
            let item = event.get("item");
            if item
                .and_then(|value| value.get("type"))
                .and_then(|value| value.as_str())
                == Some("function_call")
            {
                if let (Some(id), Some(name)) = (
                    item.and_then(|value| value.get("id"))
                        .and_then(|value| value.as_str()),
                    item.and_then(|value| value.get("name"))
                        .and_then(|value| value.as_str()),
                ) {
                    fragments.push((id.to_string(), Some(name.to_string()), String::new()));
                }
            }
        } else if event_type == Some("response.function_call_arguments.delta") {
            if let (Some(item_id), Some(delta)) = (
                event.get("item_id").and_then(|value| value.as_str()),
                event.get("delta").and_then(|value| value.as_str()),
            ) {
                fragments.push((item_id.to_string(), None, delta.to_string()));
            }
        }
        fragments
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum SummaryState {
    #[default]
    ObjectStart,
    KeyStart,
    InKey,
    AfterKey,
    ValueStart,
    InValue,
    InSkipString,
    SkipEscaped,
    Escaped,
    Unicode,
    Done,
    Failed,
}

/// Byte-by-byte JSON state machine extracting the top-level `"summary"`
/// string value. It tolerates arbitrary chunk boundaries and escapes; any
/// other shape stops emitting without failing the transport. String values of
/// other keys are skipped so `"summary"` need not be the first key.
#[derive(Default)]
struct SummaryArgumentParser {
    state: SummaryState,
    key: String,
    key_is_summary: bool,
    unicode: [u8; 4],
    unicode_index: usize,
    pending_high_surrogate: Option<u16>,
}

impl SummaryArgumentParser {
    fn push(&mut self, text: &str, emit: &mut dyn FnMut(&str)) {
        for ch in text.chars() {
            match self.state {
                SummaryState::ObjectStart => match ch {
                    '{' => self.state = SummaryState::KeyStart,
                    c if c.is_whitespace() => {}
                    _ => self.state = SummaryState::Failed,
                },
                SummaryState::KeyStart => match ch {
                    '"' => {
                        self.key.clear();
                        self.state = SummaryState::InKey;
                    }
                    c if c.is_whitespace() => {}
                    _ => self.state = SummaryState::Failed,
                },
                SummaryState::InKey => match ch {
                    '"' => {
                        self.key_is_summary = self.key == "summary";
                        self.state = SummaryState::AfterKey;
                    }
                    '\\' => self.state = SummaryState::Failed,
                    c => self.key.push(c),
                },
                SummaryState::AfterKey => match ch {
                    ':' => self.state = SummaryState::ValueStart,
                    ',' => self.state = SummaryState::KeyStart,
                    c if c.is_whitespace() => {}
                    _ => self.state = SummaryState::Failed,
                },
                SummaryState::ValueStart => match ch {
                    '"' if self.key_is_summary => self.state = SummaryState::InValue,
                    '"' => self.state = SummaryState::InSkipString,
                    c if c.is_whitespace() => {}
                    // Numbers, booleans, null, or nested values are unsupported;
                    // stop emitting rather than mis-track the structure.
                    _ => self.state = SummaryState::Failed,
                },
                SummaryState::InValue => match ch {
                    '"' => self.state = SummaryState::Done,
                    '\\' => self.state = SummaryState::Escaped,
                    c => self.emit_char(c, emit),
                },
                SummaryState::InSkipString => match ch {
                    '"' => self.state = SummaryState::AfterKey,
                    '\\' => self.state = SummaryState::SkipEscaped,
                    _ => {}
                },
                SummaryState::SkipEscaped => self.state = SummaryState::InSkipString,
                SummaryState::Escaped => match ch {
                    '"' | '\\' | '/' => {
                        self.emit_char(ch, emit);
                        self.state = SummaryState::InValue;
                    }
                    'b' => {
                        self.emit_char('\u{8}', emit);
                        self.state = SummaryState::InValue;
                    }
                    'f' => {
                        self.emit_char('\u{c}', emit);
                        self.state = SummaryState::InValue;
                    }
                    'n' => {
                        self.emit_char('\n', emit);
                        self.state = SummaryState::InValue;
                    }
                    'r' => {
                        self.emit_char('\r', emit);
                        self.state = SummaryState::InValue;
                    }
                    't' => {
                        self.emit_char('\t', emit);
                        self.state = SummaryState::InValue;
                    }
                    'u' => {
                        self.unicode_index = 0;
                        self.state = SummaryState::Unicode;
                    }
                    _ => self.state = SummaryState::Failed,
                },
                SummaryState::Unicode => {
                    let Some(digit) = ch.to_digit(16) else {
                        self.state = SummaryState::Failed;
                        continue;
                    };
                    self.unicode[self.unicode_index] = digit as u8;
                    self.unicode_index += 1;
                    if self.unicode_index < 4 {
                        continue;
                    }
                    let value = (0..4).fold(0u16, |acc, i| {
                        acc | ((self.unicode[i] as u16) << (4 * (3 - i)))
                    });
                    self.state = SummaryState::InValue;
                    self.emit_code_unit(value, emit);
                }
                SummaryState::Done | SummaryState::Failed => {}
            }
        }
    }

    fn emit_code_unit(&mut self, value: u16, emit: &mut dyn FnMut(&str)) {
        match (self.pending_high_surrogate.take(), value) {
            (Some(high), 0xDC00..=0xDFFF) => {
                let combined =
                    0x10000 + (((high - 0xD800) as u32) << 10) + u32::from(value - 0xDC00);
                if let Some(c) = char::from_u32(combined) {
                    emit(&c.to_string());
                }
            }
            (Some(_), 0xD800..=0xDBFF) => {
                self.emit_replacement(emit);
                self.pending_high_surrogate = Some(value);
            }
            (Some(_), lone) => {
                self.emit_replacement(emit);
                if let Some(c) = char::from_u32(u32::from(lone)) {
                    emit(&c.to_string());
                }
            }
            (None, 0xD800..=0xDBFF) => self.pending_high_surrogate = Some(value),
            (None, plain) => {
                if let Some(c) = char::from_u32(u32::from(plain)) {
                    emit(&c.to_string());
                }
            }
        }
    }

    fn emit_char(&mut self, c: char, emit: &mut dyn FnMut(&str)) {
        if self.pending_high_surrogate.take().is_some() {
            // Plain chars never complete a surrogate pair: pairs arrive as two
            // consecutive \u escapes. The pending unit was a lone surrogate.
            self.emit_replacement(emit);
        }
        emit(&c.to_string());
    }

    fn emit_replacement(&self, emit: &mut dyn FnMut(&str)) {
        emit("\u{FFFD}");
    }
}

/// Per-Turn provisional projection; never decides authoritative Turn lifecycle.
#[derive(Default)]
pub struct TurnResponsePreview {
    response: ResponsePreviewState,
    attempt: u64,
    round: u32,
    revision: u64,
    failed: bool,
    interruption: Option<String>,
    awaiting_first_text: bool,
    finish_candidate: bool,
    request_started_at: Option<std::time::Instant>,
    first_output_at: Option<std::time::Duration>,
    thinking_started_at: Option<std::time::Instant>,
    thinking_words: u64,
    thinking_tail: String,
    thinking_tail_chars: usize,
    thinking_truncated: bool,
    reasoning_in_word: bool,
    reasoning_tokens: Option<u64>,
    stream_phase: Option<&'static str>,
}
impl TurnResponsePreview {
    pub fn begin(&mut self, round: u32) {
        self.attempt = self.response.begin();
        self.round = round;
        self.awaiting_first_text = true;
        self.failed = false;
        self.finish_candidate = false;
        self.request_started_at = Some(std::time::Instant::now());
        self.first_output_at = None;
        self.thinking_started_at = None;
        self.thinking_words = 0;
        self.thinking_tail.clear();
        self.thinking_tail_chars = 0;
        self.thinking_truncated = false;
        self.reasoning_in_word = false;
        self.reasoning_tokens = None;
        self.stream_phase = None;
    }
    pub fn observe_stream_event(&mut self, event: &Value) -> bool {
        let now = std::time::Instant::now();
        let mut changed = false;
        if let Some(tokens) = reasoning_token_count(event) {
            if self.reasoning_tokens != Some(tokens) {
                self.reasoning_tokens = Some(tokens);
                changed = true;
            }
        }
        if let Some(text) = reasoning_delta(event) {
            if self.first_output_at.is_none() {
                self.first_output_at = self.request_started_at.map(|started| started.elapsed());
            }
            if self.thinking_started_at.is_none() {
                self.thinking_started_at = Some(now);
                changed = true;
            }
            if self.stream_phase != Some("thinking") {
                self.stream_phase = Some("thinking");
                changed = true;
            }
            for ch in text.chars() {
                let cjk = matches!(ch as u32,
                    0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF |
                    0x3040..=0x30FF | 0xAC00..=0xD7AF);
                if cjk {
                    self.reasoning_in_word = false;
                    self.thinking_words = self.thinking_words.saturating_add(1);
                    changed = true;
                } else if ch.is_alphanumeric() || ch == '_' {
                    if !self.reasoning_in_word {
                        self.reasoning_in_word = true;
                        self.thinking_words = self.thinking_words.saturating_add(1);
                        changed = true;
                    }
                } else {
                    self.reasoning_in_word = false;
                }
            }
            self.append_thinking_tail(text);
        }
        if public_text_delta(event).is_some() {
            if self.first_output_at.is_none() {
                self.first_output_at = self.request_started_at.map(|started| started.elapsed());
            }
            if self.stream_phase != Some("responding") {
                self.stream_phase = Some("responding");
                changed = true;
            }
            // The inline thinking window is only live during reasoning. Once
            // the answer starts, stop carrying its tail on every preview frame.
            self.clear_inline_thinking();
        }
        changed
    }
    fn clear_inline_thinking(&mut self) {
        self.thinking_tail.clear();
        self.thinking_tail_chars = 0;
        self.thinking_truncated = false;
    }

    fn append_thinking_tail(&mut self, text: &str) {
        self.thinking_tail.push_str(text);
        self.thinking_tail_chars = self
            .thinking_tail_chars
            .saturating_add(text.chars().count());
        if self.thinking_tail_chars > MAX_THINKING_TAIL_CHARS {
            let excess = self.thinking_tail_chars - MAX_THINKING_TAIL_CHARS;
            if let Some((cut, _)) = self.thinking_tail.char_indices().nth(excess) {
                self.thinking_tail.drain(..cut);
            }
            self.thinking_tail_chars = MAX_THINKING_TAIL_CHARS;
            self.thinking_truncated = true;
        }
    }

    pub fn thinking_active(&self) -> bool {
        self.stream_phase == Some("thinking")
    }

    pub fn ttft(&self) -> Option<std::time::Duration> {
        self.first_output_at
    }
    pub fn thinking_elapsed(&self) -> Option<std::time::Duration> {
        self.thinking_started_at.map(|started| started.elapsed())
    }
    pub fn append(&mut self, target: PublicTextTarget, text: &str) -> Result<bool, String> {
        if self.failed {
            return Ok(false);
        }
        if target == PublicTextTarget::FinishSummary {
            if !self.finish_candidate && !text.is_empty() {
                self.response.replace_current(self.attempt);
            }
            self.finish_candidate = true;
        }
        if self.awaiting_first_text && !text.is_empty() {
            self.awaiting_first_text = false;
            self.response.replace_prior(self.attempt);
            self.interruption = None;
        }
        if !text.is_empty() {
            self.stream_phase = Some("responding");
            // `append` is the single public-text entry point, including the
            // native finish-summary stream which has no public text delta
            // event. Clear here so a completed reasoning attempt never leaks
            // its tail into an answer preview.
            self.clear_inline_thinking();
        }
        let result = self.response.append(self.attempt, text);
        if result.is_err() {
            self.retract();
        }
        result
    }
    pub fn validated(&mut self, accepted: bool, final_response: bool) {
        if self.failed {
            return;
        }
        if !accepted {
            self.retract();
            return;
        }
        if self.finish_candidate && !final_response {
            // An optimistic task_finished summary is streamed before the
            // authoritative outcome is known (memo guard, action results).
            // Keep it provisional and let resolve_finish settle the attempt.
            return;
        }
        self.response.settle(self.attempt, final_response);
    }
    /// The attempt streamed a task_finished summary and its authoritative
    /// outcome has not been resolved yet.
    pub fn finish_pending(&self) -> bool {
        self.finish_candidate && !self.failed
    }
    /// Whether the visible preview is already settled Final with exactly the
    /// authoritative summary (the optimistic stream was confirmed).
    pub fn finish_confirmed_final(&self, text: &str) -> bool {
        !self.failed
            && self.response.visible().is_some_and(|preview| {
                preview.status == PreviewStatus::Final && preview.text == text
            })
    }
    /// Settles a pending optimistic finish. `final_response == false` demotes
    /// the text to a settled intermediate (finish was intercepted/rejected);
    /// `true` confirms it as the Final answer.
    pub fn resolve_finish(&mut self, final_response: bool) -> bool {
        if !self.finish_pending() {
            self.finish_candidate = false;
            return false;
        }
        self.finish_candidate = false;
        self.response.settle(self.attempt, final_response)
    }
    pub fn interrupt(&mut self, reason: &str) {
        self.failed = true;
        self.interruption = Some(reason.to_string());
        self.clear_inline_thinking();
        if self.stream_phase == Some("thinking") {
            self.stream_phase = None;
        }
    }
    pub fn retract(&mut self) {
        self.failed = true;
        self.interruption = None;
        self.clear_inline_thinking();
        if self.stream_phase == Some("thinking") {
            self.stream_phase = None;
        }
        self.response.retract(self.attempt);
    }
    pub fn publish(&mut self, ui: &mut dyn crate::TurnUi, session: &str, turn_id: &str) {
        self.revision = self.revision.saturating_add(1);
        ui.on_core_topic_events(&[crate::host::CoreTopicEvent::new(
            session,
            crate::host::CoreTopic::new("core.model.preview", serde_json::json!({})),
            crate::host::CoreSessionState::WaitingModel,
            serde_json::json!({"turn_id": turn_id, "attempt": self.attempt, "round": self.round,
                "revision": self.revision, "interruption": self.interruption, "response": self.response.visible(),
                "thinking_words": self.thinking_words,
                "thinking_tail": self.thinking_tail.as_str(),
                "thinking_truncated": self.thinking_truncated,
                "thinking_elapsed_ms": self.thinking_elapsed().map(|value| value.as_millis()),
                "ttft_ms": self.ttft().map(|value| value.as_millis()),
                "reasoning_tokens": self.reasoning_tokens,
                "stream_phase": self.stream_phase }),
        )]);
    }
}
