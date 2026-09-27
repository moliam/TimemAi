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

/// Typed provisional destinations for public text preview.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PublicTextTarget {
    Response,
}

/// Per-Turn provisional projection; never decides authoritative Turn lifecycle.
#[derive(Default)]
pub struct TurnResponsePreview {
    response: ResponsePreviewState,
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
    pub fn append(&mut self, _target: PublicTextTarget, text: &str) -> Result<bool, String> {
        if self.failed {
            return Ok(false);
        }
        if self.awaiting_first_text && !text.is_empty() {
            self.awaiting_first_text = false;
            self.response.replace_prior(self.attempt);
            self.interruption = None;
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
        self.response.settle(self.attempt, final_response);
    }
    pub fn interrupt(&mut self, reason: &str) {
        self.failed = true;
        self.interruption = Some(reason.to_string());
    }
    pub fn retract(&mut self) {
        self.failed = true;
        self.interruption = None;
        self.response.retract(self.attempt);
    }
    pub fn publish(&mut self, ui: &mut dyn crate::TurnUi, session: &str, turn_id: &str) {
        self.revision = self.revision.saturating_add(1);
        ui.on_core_topic_events(&[crate::host::CoreTopicEvent::new(
            session,
            crate::host::CoreTopic::new("core.model.preview", serde_json::json!({})),
            crate::host::CoreSessionState::WaitingModel,
            serde_json::json!({"turn_id": turn_id, "attempt": self.attempt,
                "revision": self.revision, "interruption": self.interruption, "response": self.response.visible() }),
        )]);
    }
}
