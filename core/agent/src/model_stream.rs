//! Bounded SSE content decoding. Display evidence is not protocol acceptance.
use serde_json::Value;

const MAX_EVENT_BYTES: usize = 1024 * 1024;
const MAX_SSE_EVENT_BYTES: usize = 4 * 1024 * 1024;

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
    chat_count: usize,
    active_chat: Option<(usize, usize)>,
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
        self.active_chat
            .filter(|(depth, _)| self.stack.len() == depth + 1)
            .and_then(|(_, index)| match self.stack.last().map(String::as_str) {
                Some("task") => Some(PublicTextTarget::ChatTask { index }),
                Some("answer") => Some(PublicTextTarget::ChatAnswer { index }),
                _ => None,
            })
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
                    if self
                        .active_chat
                        .is_some_and(|(depth, _)| depth == self.stack.len())
                    {
                        self.active_chat = None;
                    }
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
                        let is_chat = name == "sub_answer"
                            && self.stack.first().is_some_and(|s| s == "assistant")
                            && (self.stack.as_slice() == ["assistant", "actions"]
                                || self.stack.as_slice() == ["assistant", "actions", "parallel"]);
                        self.stack.push(name);
                        if is_chat {
                            if self.chat_count >= 128 {
                                return Err("model_preview_chat_limit".into());
                            }
                            self.active_chat = Some((self.stack.len(), self.chat_count));
                            self.chat_count += 1;
                        }
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

/// Typed provisional destinations. Chat fields are an explicit display allowlist,
/// not generic tool argument disclosure or permission to execute a tool.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PublicTextTarget {
    Response,
    ChatTask { index: usize },
    ChatAnswer { index: usize },
}

#[derive(Default)]
pub struct JsonChatTextStream {
    stack: Vec<(bool, String, usize)>,
    key: String,
    token: String,
    in_string: bool,
    escaped: bool,
    reading_key: bool,
    expecting_key: bool,
    target: Option<PublicTextTarget>,
    chat_count: usize,
    active_chat: Option<(usize, usize)>,
    total_bytes: usize,
    failed: bool,
}

impl JsonChatTextStream {
    pub fn push(
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
        for ch in text.chars() {
            if self.in_string {
                if ch == '"' && !self.escaped {
                    self.in_string = false;
                    if self.reading_key {
                        match serde_json::from_str::<String>(&format!("\"{}\"", self.token)) {
                            Ok(key) => self.key = key,
                            Err(_) => {
                                self.failed = true;
                                return Err("invalid_preview_json_string".into());
                            }
                        }
                    } else if !self.token.is_empty() {
                        self.failed = true;
                        return Err("invalid_preview_json_string".into());
                    }
                    self.token.clear();
                    self.target = None;
                    continue;
                }
                if self.reading_key || self.target.is_some() {
                    self.token.push(ch);
                    if let Some(target) = &self.target {
                        if let Ok(value) =
                            serde_json::from_str::<String>(&format!("\"{}\"", self.token))
                        {
                            emit(target.clone(), &value);
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
                    self.reading_key = self.expecting_key;
                    self.expecting_key = false;
                    self.target = if !self.reading_key {
                        self.active_chat
                            .filter(|(depth, _)| *depth == self.stack.len())
                            .and_then(|(_, index)| match self.key.as_str() {
                                "task" => Some(PublicTextTarget::ChatTask { index }),
                                "answer" => Some(PublicTextTarget::ChatAnswer { index }),
                                _ => None,
                            })
                    } else {
                        None
                    };
                }
                '{' | '[' => {
                    let path_key = std::mem::take(&mut self.key);
                    let is_chat = ch == '{'
                        && path_key == "sub_answer"
                        && self.stack.len() >= 2
                        && self.stack[1..]
                            .iter()
                            .any(|(_, key, _)| key == "working_still_action")
                        && self.stack[1..]
                            .iter()
                            .all(|(_, key, _)| key.is_empty() || key == "working_still_action");
                    if self.stack.len() >= 64 {
                        self.failed = true;
                        return Err("model_preview_json_depth".into());
                    }
                    self.stack.push((ch == '{', path_key, 0));
                    if is_chat {
                        if self.chat_count >= 128 {
                            self.failed = true;
                            return Err("model_preview_chat_limit".into());
                        }
                        self.active_chat = Some((self.stack.len(), self.chat_count));
                        self.chat_count += 1;
                    }
                    self.expecting_key = ch == '{';
                }
                '}' | ']' => {
                    if self
                        .active_chat
                        .is_some_and(|(depth, _)| depth == self.stack.len())
                    {
                        self.active_chat = None;
                    }
                    self.stack.pop();
                    self.key.clear();
                    self.expecting_key = false;
                }
                ',' => {
                    self.expecting_key = self.stack.last().is_some_and(|(object, _, _)| *object);
                    self.key.clear();
                }
                ':' => self.expecting_key = false,
                _ => {}
            }
        }
        Ok(())
    }
}

#[derive(Default)]
pub struct NativeChatTextStream {
    calls: std::collections::BTreeMap<usize, NativeChatCall>,
    total_bytes: usize,
    failed: bool,
}
#[derive(Default)]
struct NativeChatCall {
    name: String,
    pending: String,
    decoder: JsonChatTextStream,
    started: bool,
}
impl NativeChatTextStream {
    pub fn push(
        &mut self,
        event: &Value,
        emit: &mut dyn FnMut(PublicTextTarget, &str),
    ) -> Result<(), String> {
        if self.failed {
            return Ok(());
        }
        let result = self.push_inner(event, emit);
        if result.is_err() {
            self.failed = true;
            self.calls.clear();
        }
        result
    }
    fn push_inner(
        &mut self,
        event: &Value,
        emit: &mut dyn FnMut(PublicTextTarget, &str),
    ) -> Result<(), String> {
        let Some(calls) = event
            .pointer("/choices/0/delta/tool_calls")
            .and_then(Value::as_array)
        else {
            return Ok(());
        };
        for call in calls {
            let Some(index) = call
                .get("index")
                .and_then(Value::as_u64)
                .and_then(|i| usize::try_from(i).ok())
            else {
                return Err("model_preview_tool_index_required".into());
            };
            if index >= 128 {
                return Err("model_preview_chat_limit".into());
            }
            let name = call
                .pointer("/function/name")
                .and_then(Value::as_str)
                .unwrap_or("");
            let args = call
                .pointer("/function/arguments")
                .and_then(Value::as_str)
                .unwrap_or("");
            self.total_bytes = self
                .total_bytes
                .saturating_add(name.len())
                .saturating_add(args.len());
            if self.total_bytes > MAX_EVENT_BYTES {
                return Err("model_preview_text_too_large".into());
            }
            let state = self.calls.entry(index).or_default();
            state.name.push_str(name);
            if state.started && !name.is_empty() {
                return Err("model_preview_tool_name_changed".into());
            }
            if !state.started {
                state.pending.push_str(args);
                if state.name != "sub_answer" {
                    continue;
                }
                state
                    .decoder
                    .push(r#"{"working_still_action":{"sub_answer":"#, &mut |_, _| {})?;
                state.started = true;
            } else {
                state.pending.push_str(args);
            }
            state
                .decoder
                .push(&std::mem::take(&mut state.pending), &mut |target, text| {
                    let target = match target {
                        PublicTextTarget::ChatTask { .. } => PublicTextTarget::ChatTask { index },
                        PublicTextTarget::ChatAnswer { .. } => {
                            PublicTextTarget::ChatAnswer { index }
                        }
                        PublicTextTarget::Response => return,
                    };
                    emit(target, text);
                })?;
        }
        Ok(())
    }
}

/// One attempt owns all provisional chat windows. Confirmed chat is delivered
/// through core.sub_answer and is never stored in this provisional collection.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ProvisionalChat {
    pub index: usize,
    pub task: String,
    pub answer: String,
}

#[derive(Default)]
pub struct ChatPreviewState {
    attempt: u64,
    accepting: bool,
    bytes: usize,
    windows: std::collections::BTreeMap<usize, ProvisionalChat>,
}
impl ChatPreviewState {
    pub fn begin(&mut self, attempt: u64) {
        self.attempt = attempt;
        self.accepting = true;
        self.bytes = 0;
        self.windows.clear();
    }
    pub fn windows(&self) -> impl Iterator<Item = &ProvisionalChat> {
        self.windows.values()
    }
    pub fn append(
        &mut self,
        attempt: u64,
        target: PublicTextTarget,
        text: &str,
    ) -> Result<bool, String> {
        if !self.accepting || attempt != self.attempt || text.is_empty() {
            return Ok(false);
        }
        let (index, task) = match target {
            PublicTextTarget::ChatTask { index } => (index, true),
            PublicTextTarget::ChatAnswer { index } => (index, false),
            PublicTextTarget::Response => return Ok(false),
        };
        if index >= 128 || self.bytes.saturating_add(text.len()) > MAX_EVENT_BYTES {
            self.retract(attempt);
            return Err("model_preview_chat_limit".into());
        }
        self.bytes += text.len();
        let window = self
            .windows
            .entry(index)
            .or_insert_with(|| ProvisionalChat {
                index,
                ..Default::default()
            });
        if task {
            window.task.push_str(text);
        } else {
            window.answer.push_str(text);
        }
        Ok(true)
    }
    /// Validation alone does not confirm chat delivery. Hold the window until
    /// the actual sub_answer success event replaces it in the same UI position.
    pub fn validated(&mut self, attempt: u64, accepted: bool) {
        if attempt != self.attempt {
            return;
        }
        self.accepting = false;
        if !accepted {
            self.retract(attempt);
        }
    }
    pub fn delivered(&mut self, attempt: u64, index: usize) -> bool {
        if attempt != self.attempt {
            return false;
        }
        self.windows.remove(&index).is_some()
    }
    pub fn retract(&mut self, attempt: u64) -> bool {
        if attempt != self.attempt {
            return false;
        }
        self.accepting = false;
        self.bytes = 0;
        let changed = !self.windows.is_empty();
        self.windows.clear();
        changed
    }
}

/// Per-Turn provisional projection; never decides authoritative Turn lifecycle.
#[derive(Default)]
pub struct TurnResponsePreview {
    response: ResponsePreviewState,
    chat: ChatPreviewState,
    attempt: u64,
    revision: u64,
    failed: bool,
    interruption: Option<String>,
    awaiting_first_text: bool,
}
impl TurnResponsePreview {
    pub fn begin(&mut self) {
        self.attempt = self.response.begin();
        self.awaiting_first_text = true;
        self.failed = false;
    }
    pub fn append(&mut self, target: PublicTextTarget, text: &str) -> Result<bool, String> {
        if self.failed {
            return Ok(false);
        }
        if self.awaiting_first_text && !text.is_empty() {
            self.awaiting_first_text = false;
            self.response.replace_prior(self.attempt);
            self.chat.begin(self.attempt);
            self.interruption = None;
        }
        let result = match target {
            PublicTextTarget::Response => self.response.append(self.attempt, text),
            target => self.chat.append(self.attempt, target, text),
        };
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
        self.response.settle(self.attempt, final_response);
        self.chat.validated(self.attempt, true);
    }
    pub fn interrupt(&mut self, reason: &str) {
        self.failed = true;
        self.interruption = Some(reason.to_string());
        self.chat.validated(self.attempt, true);
    }
    pub fn retract(&mut self) {
        self.failed = true;
        self.interruption = None;
        self.response.retract(self.attempt);
        self.chat.retract(self.attempt);
    }
    pub fn confirm_chat(&mut self, task: &str, answer: &str) -> Option<(u64, usize)> {
        let index = self
            .chat
            .windows()
            .find(|window| window.task.trim() == task && window.answer.trim() == answer)
            .map(|window| window.index)?;
        self.chat.delivered(self.attempt, index);
        Some((self.attempt, index))
    }
    pub fn clear_chat(&mut self) {
        self.chat.retract(self.attempt);
    }
    pub fn publish(&mut self, ui: &mut dyn crate::TurnUi, session: &str, turn_id: &str) {
        self.revision = self.revision.saturating_add(1);
        ui.on_core_topic_events(&[crate::host::CoreTopicEvent::new(
            session,
            crate::host::CoreTopic::new("core.model.preview", serde_json::json!({})),
            crate::host::CoreSessionState::WaitingModel,
            serde_json::json!({"turn_id": turn_id, "attempt": self.attempt,
                "revision": self.revision, "interruption": self.interruption, "response": self.response.visible(),
                "chat": self.chat.windows().collect::<Vec<_>>() }),
        )]);
    }
}
