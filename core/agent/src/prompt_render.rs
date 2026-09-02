use crate::capability::CapabilityRegistry;
use crate::prompt_spec;
use crate::response_protocol::{PromptBoundarySpec, ResponseProtocolSuite};
use crate::tool_result_gate::{self, Retention};
use crate::{
    ActionStatus, BashResultEvidence, MemmgrResultEvidence, PromptDelta, PromptSlice,
    ReadfileResultEvidence, SelfToolResultEvidence, ToolCallMode,
};
use std::hash::{DefaultHasher, Hash, Hasher};
use timem_ui_contract::preferences::{AssistantResponseFormat, InterfacePreferences};

pub(crate) const RESPONSE_TRAILER: &str =
    "Please continue the work and respond as protocol requires in user's language:";
pub(crate) const NATIVE_RESPONSE_TRAILER: &str = "Continue the work and express thought in the user's language. Call API tools when more evidence or actions are needed; otherwise give the final user-facing answer:";
pub(crate) const CONTEXT_COMPACT_REQUIRED_TRAILER: &str =
    "Context is too long. Your tool calls must start with context_compact:";
const NATIVE_PROTOCOL_SECTION: &str = "## Tool Calling\n\nCapabilities are provided through the model API. Call them through the API tool-call channel. You may request independent calls together. Text accompanying calls is a user-visible progress note. A response with no tool calls is the final user-facing answer. `context_compact` may be followed by other capability calls in the same response, but it must be the first call. Later calls run only after compaction succeeds.";
const NATIVE_RESPONSE_MODE_INSTRUCTION: &str = "Use the API tool-call channel for runtime capabilities. Ordinary response text is user-visible, you should report to user your progress often, or answer questions while working; text without tool calls finishes the loop.";
const INLINE_RESPONSE_MODE_INSTRUCTION: &str =
    "Your response MUST be exactly protocol-compliant in the response protocol below.";
const INLINE_TOOL_CATALOG_SECTION_HEADING: &str = "## Actions\n\nGenerate actions to drive the runtime to do things for you. There are several builtin actions:\n\n### Available capabilities";
const NATIVE_BUILTIN_TOOL_DESCRIPTIONS_HEADING: &str =
    "## Built-in Tool Descriptions\n\nBuilt-in tool parameter schemas are provided separately through the model API. One response can reasonably contain multiple tool calls for better performance.";
pub(crate) const MAX_ACTION_RESULT_PROMPT_BYTES: usize =
    tool_result_gate::MAX_MODEL_TOOL_RESULT_BYTES;

pub(crate) fn truncate_action_result_for_prompt(text: &str) -> String {
    tool_result_gate::gate(text, Retention::Head)
}

pub(crate) fn formatted_response_trailer(
    _response_shape_hint: &str,
    _assistant_heading: &str,
) -> String {
    RESPONSE_TRAILER.to_string()
}

pub(crate) fn split_formatted_response_trailer(rendered_prompt: &str) -> (&str, Option<String>) {
    let trimmed = rendered_prompt.trim_end();
    for trailer in [
        RESPONSE_TRAILER,
        NATIVE_RESPONSE_TRAILER,
        CONTEXT_COMPACT_REQUIRED_TRAILER,
    ] {
        let marker = format!("\n\n{trailer}");
        if let Some(trailer_start) = trimmed.strip_suffix(&marker).map(str::len) {
            let prefix = trimmed[..trailer_start].trim_end();
            return (prefix, Some(trailer.to_string()));
        }
    }
    (rendered_prompt, None)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VisiblePromptRole {
    User,
    UserSupplement,
    You,
    ContextCompactionSummary,
    Runtime,
}

impl VisiblePromptRole {
    fn label(self, spec: &PromptBoundarySpec) -> &str {
        match self {
            VisiblePromptRole::User | VisiblePromptRole::UserSupplement => spec.user_role,
            VisiblePromptRole::You | VisiblePromptRole::ContextCompactionSummary => {
                spec.assistant_role
            }
            VisiblePromptRole::Runtime => spec.runtime_role,
        }
    }

    fn assistant_id(self, assistant_heading: &str) -> Option<&str> {
        matches!(
            self,
            VisiblePromptRole::You | VisiblePromptRole::ContextCompactionSummary
        )
        .then_some(assistant_heading)
    }

    fn render_open(self, spec: &PromptBoundarySpec, assistant_heading: &str) -> String {
        if self == VisiblePromptRole::UserSupplement {
            if spec.uses_xml_role_elements() {
                format!("<{} kind=\"supplement\">", spec.user_role)
            } else {
                format!("## {} (supplement)", spec.user_role)
            }
        } else if self == VisiblePromptRole::ContextCompactionSummary {
            if spec.uses_xml_role_elements() {
                format!(
                    "<{} kind=\"context_compaction_summary\">",
                    spec.assistant_role
                )
            } else {
                format!("## {} (context compaction summary)", assistant_heading)
            }
        } else {
            spec.render_role_open(self.label(spec), self.assistant_id(assistant_heading))
        }
    }
}

fn visible_role(prompt_type: &str) -> VisiblePromptRole {
    match prompt_type {
        "user_question" => VisiblePromptRole::User,
        "user_supplement" => VisiblePromptRole::UserSupplement,
        "llm_response" | "llm_response_raw_xml" | "llm_free_talk" => VisiblePromptRole::You,
        "context_compaction_summary" => VisiblePromptRole::ContextCompactionSummary,
        "result_of_llm_action" | "response_repair" | "context_compacted" => {
            VisiblePromptRole::Runtime
        }
        _ => VisiblePromptRole::Runtime,
    }
}

fn is_action_result_prompt_type(prompt_type: &str) -> bool {
    prompt_type == "result_of_llm_action"
}

fn is_raw_xml_assistant_response(prompt_type: &str, boundaries: &PromptBoundarySpec) -> bool {
    boundaries.uses_xml_role_elements() && prompt_type == "llm_response_raw_xml"
}

fn escape_xml_text(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn escape_xml_attribute(text: &str) -> String {
    escape_xml_text(text)
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn truncate_xml_result_text_for_budget(text: &str, budget: usize, retention: Retention) -> String {
    // Escaping can expand the payload, so find the largest raw budget whose escaped,
    // already-gated representation still fits the XML envelope.
    let mut low = 0usize;
    let mut high = budget.min(text.len());
    let mut best = String::new();
    while low <= high {
        let candidate_budget = low + (high - low) / 2;
        let candidate = tool_result_gate::fit(text, candidate_budget, retention);
        let escaped = escape_xml_text(&candidate);
        if escaped.len() <= budget {
            best = escaped;
            low = candidate_budget.saturating_add(1);
        } else if candidate_budget == 0 {
            break;
        } else {
            high = candidate_budget - 1;
        }
    }
    best
}

fn action_output_id(output: &str, time_ms: i64) -> String {
    let mut hasher = DefaultHasher::new();
    output.hash(&mut hasher);
    time_ms.hash(&mut hasher);
    format!("{:06x}", hasher.finish() & 0x00ff_ffff)
}

fn bash_boundary_id(task: &str, stdout: &str, stderr: &str, output_time_ms: i64) -> String {
    for salt in 0_u32..=u32::MAX {
        let mut hasher = DefaultHasher::new();
        task.hash(&mut hasher);
        stdout.hash(&mut hasher);
        stderr.hash(&mut hasher);
        output_time_ms.hash(&mut hasher);
        salt.hash(&mut hasher);
        let id = format!("{:04x}", hasher.finish() & 0xffff);
        let output_marker = format!("OUTPUT_{id}");
        let stdout_marker = format!("OUT_{id}");
        let stderr_marker = format!("ERR_{id}");
        if !stdout.contains(&output_marker)
            && !stderr.contains(&output_marker)
            && !stdout.contains(&stdout_marker)
            && !stderr.contains(&stdout_marker)
            && !stdout.contains(&stderr_marker)
            && !stderr.contains(&stderr_marker)
        {
            return id;
        }
    }
    unreachable!("the finite Bash output cannot contain every possible salted boundary")
}

fn action_lifecycle_status(status: ActionStatus) -> &'static str {
    match status {
        ActionStatus::Timeout => "timeout",
        ActionStatus::BackgroundRunning => "running",
        _ => "finished",
    }
}

pub(crate) fn render_xml_bash_result_with_retention(
    action_name: Option<&str>,
    status: ActionStatus,
    evidence: &BashResultEvidence,
    output_time_ms: i64,
    retention: Retention,
) -> String {
    let task = action_name
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or(crate::os::local_shell_tool_name());
    let escaped_task = escape_xml_attribute(task);
    let status = action_lifecycle_status(status);
    let exit_code = evidence
        .exit_code
        .map(|code| format!(" exit_code=\"{code}\""))
        .unwrap_or_default();
    let signal = evidence
        .signal
        .map(|signal| format!(" signal=\"{signal}\""))
        .unwrap_or_default();
    let pid = evidence
        .pid
        .map(|pid| format!(" pid=\"{pid}\""))
        .unwrap_or_default();
    let timed_out = if evidence.timed_out {
        " timed_out=\"true\""
    } else {
        ""
    };
    let pid_kind = evidence
        .pid_kind
        .as_deref()
        .map(escape_xml_attribute)
        .map(|pid_kind| format!(" pid_kind=\"{pid_kind}\""))
        .unwrap_or_default();
    let error_type = evidence
        .error_type
        .as_deref()
        .map(escape_xml_attribute)
        .map(|error_type| format!(" error_type=\"{error_type}\""))
        .unwrap_or_default();
    let prefix = format!(
        "<bash_result task=\"{escaped_task}\" status=\"{status}\"{exit_code}{signal}{pid}{timed_out}{pid_kind}{error_type}>\n"
    );
    let suffix = "\n</bash_result>";
    let id = bash_boundary_id(task, &evidence.stdout, &evidence.stderr, output_time_ms);

    let stdout_nonempty = !evidence.stdout.is_empty();
    let stderr_nonempty = !evidence.stderr.is_empty();
    let both_streams = stdout_nonempty && stderr_nonempty;

    if both_streams {
        let out_open = format!("<stdout>\n<<<OUT_{id}\n");
        let out_close = format!("\nOUT_{id}\n</stdout>\n\n");
        let err_open = format!("<stderr>\n<<<ERR_{id}\n");
        let err_close = format!("\nERR_{id}\n</stderr>");
        let fixed = prefix.len()
            + suffix.len()
            + out_open.len()
            + out_close.len()
            + err_open.len()
            + err_close.len();
        let body_budget = MAX_ACTION_RESULT_PROMPT_BYTES.saturating_sub(fixed);
        let stdout_budget = body_budget / 2;
        let stderr_budget = body_budget.saturating_sub(stdout_budget);
        let stdout = tool_result_gate::fit(evidence.stdout.trim_end(), stdout_budget, retention);
        let stderr = tool_result_gate::fit(evidence.stderr.trim_end(), stderr_budget, retention);
        return format!(
            "{prefix}{out_open}{stdout}{out_close}{err_open}{stderr}{err_close}{suffix}"
        );
    }

    let content = if stdout_nonempty {
        evidence.stdout.as_str()
    } else {
        evidence.stderr.as_str()
    };
    let open = format!("<<<OUTPUT_{id}\n");
    let close = format!("\nOUTPUT_{id}");
    let fixed = prefix.len() + suffix.len() + open.len() + close.len();
    let body_budget = MAX_ACTION_RESULT_PROMPT_BYTES.saturating_sub(fixed);
    let content = tool_result_gate::fit(content.trim_end(), body_budget, retention);
    format!("{prefix}{open}{content}{close}{suffix}")
}

#[cfg(test)]
pub(crate) fn render_xml_bash_result(
    action_name: Option<&str>,
    status: ActionStatus,
    evidence: &BashResultEvidence,
    output_time_ms: i64,
) -> String {
    render_xml_bash_result_with_retention(
        action_name,
        status,
        evidence,
        output_time_ms,
        Retention::Head,
    )
}

fn specialized_boundary_id(
    kind: &str,
    task: &str,
    metadata: &str,
    content: &str,
    output_time_ms: i64,
) -> String {
    for salt in 0_u32..=u32::MAX {
        let mut hasher = DefaultHasher::new();
        kind.hash(&mut hasher);
        task.hash(&mut hasher);
        metadata.hash(&mut hasher);
        content.hash(&mut hasher);
        output_time_ms.hash(&mut hasher);
        salt.hash(&mut hasher);
        let id = format!("{:04x}", hasher.finish() & 0xffff);
        if !content.contains(&format!("CONTENT_{id}")) && !content.contains(&format!("ERROR_{id}"))
        {
            return id;
        }
    }
    unreachable!("finite tool output cannot contain every possible salted boundary")
}

fn append_xml_attribute(attributes: &mut String, name: &str, value: &str) {
    attributes.push(' ');
    attributes.push_str(name);
    attributes.push_str("=\"");
    attributes.push_str(&escape_xml_attribute(value));
    attributes.push('"');
}

struct XmlSpecializedResult<'a> {
    root: &'a str,
    task: &'a str,
    status: ActionStatus,
    attributes: String,
    content: &'a str,
    error_type: Option<&'a str>,
    output_time_ms: i64,
    retention: Retention,
}

fn render_xml_specialized_result(result: XmlSpecializedResult<'_>) -> String {
    let XmlSpecializedResult {
        root,
        task,
        status,
        mut attributes,
        content,
        error_type,
        output_time_ms,
        retention,
    } = result;
    append_xml_attribute(&mut attributes, "status", action_lifecycle_status(status));
    if let Some(error_type) = error_type {
        append_xml_attribute(&mut attributes, "error_type", error_type);
    }

    let escaped_task = escape_xml_attribute(task);
    let prefix = format!("<{root} task=\"{escaped_task}\"{attributes}>\n");
    let suffix = format!("\n</{root}>");
    let id = specialized_boundary_id(root, task, &attributes, content, output_time_ms);
    let label = if error_type.is_some()
        || matches!(
            status,
            ActionStatus::Failed | ActionStatus::Timeout | ActionStatus::Cancelled
        ) {
        "ERROR"
    } else {
        "CONTENT"
    };
    let open = format!("<<<{label}_{id}\n");
    let close = format!("\n{label}_{id}");
    let fixed = prefix.len() + suffix.len() + open.len() + close.len();
    let body_budget = MAX_ACTION_RESULT_PROMPT_BYTES.saturating_sub(fixed);
    let content = tool_result_gate::fit(content.trim_end(), body_budget, retention);
    format!("{prefix}{open}{content}{close}{suffix}")
}

pub(crate) fn render_xml_readfile_result_with_retention(
    action_name: Option<&str>,
    status: ActionStatus,
    evidence: &ReadfileResultEvidence,
    output_time_ms: i64,
    retention: Retention,
) -> String {
    let task = action_name
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or("readfile");
    let mut attributes = String::new();
    append_xml_attribute(&mut attributes, "path", &evidence.path);
    if let Some(matcher) = evidence.matcher.as_deref() {
        append_xml_attribute(&mut attributes, "matcher", matcher);
    }
    if let (Some(start), Some(end)) = (evidence.start_line, evidence.end_line) {
        append_xml_attribute(&mut attributes, "lines", &format!("{start}-{end}"));
    }
    if let Some(total_lines) = evidence.total_lines {
        append_xml_attribute(&mut attributes, "total_lines", &total_lines.to_string());
    }
    if let Some(encoding) = evidence.encoding.as_deref() {
        append_xml_attribute(&mut attributes, "encoding", encoding);
    }
    if let Some(file_bytes) = evidence.file_bytes {
        append_xml_attribute(&mut attributes, "file_bytes", &file_bytes.to_string());
    }
    if let Some(content_bytes) = evidence.content_bytes {
        append_xml_attribute(&mut attributes, "content_bytes", &content_bytes.to_string());
    }
    if let Some(limited) = evidence.limited {
        append_xml_attribute(&mut attributes, "truncated", &limited.to_string());
    }
    if let Some(tail_out) = evidence.tail_out {
        append_xml_attribute(&mut attributes, "tail_out", &tail_out.to_string());
    }
    render_xml_specialized_result(XmlSpecializedResult {
        root: "readfile_result",
        task,
        status,
        attributes,
        content: &evidence.content,
        error_type: evidence.error_type.as_deref(),
        output_time_ms,
        retention,
    })
}

#[cfg(test)]
pub(crate) fn render_xml_readfile_result(
    action_name: Option<&str>,
    status: ActionStatus,
    evidence: &ReadfileResultEvidence,
    output_time_ms: i64,
) -> String {
    render_xml_readfile_result_with_retention(
        action_name,
        status,
        evidence,
        output_time_ms,
        Retention::Head,
    )
}

pub(crate) fn render_xml_memmgr_result_with_retention(
    action_name: Option<&str>,
    status: ActionStatus,
    evidence: &MemmgrResultEvidence,
    output_time_ms: i64,
    retention: Retention,
) -> String {
    let task = action_name
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or("memmgr");
    let mut attributes = String::new();
    append_xml_attribute(&mut attributes, "type", &evidence.memory_type);
    append_xml_attribute(&mut attributes, "op", &evidence.op);
    render_xml_specialized_result(XmlSpecializedResult {
        root: "memmgr_result",
        task,
        status,
        attributes,
        content: &evidence.content,
        error_type: evidence.error_type.as_deref(),
        output_time_ms,
        retention,
    })
}

#[cfg(test)]
pub(crate) fn render_xml_memmgr_result(
    action_name: Option<&str>,
    status: ActionStatus,
    evidence: &MemmgrResultEvidence,
    output_time_ms: i64,
) -> String {
    render_xml_memmgr_result_with_retention(
        action_name,
        status,
        evidence,
        output_time_ms,
        Retention::Head,
    )
}

pub(crate) fn render_xml_self_tool_result_with_retention(
    action_name: Option<&str>,
    status: ActionStatus,
    evidence: &SelfToolResultEvidence,
    output_time_ms: i64,
    retention: Retention,
) -> String {
    let task = action_name
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or("self_tool");
    let mut attributes = String::new();
    append_xml_attribute(&mut attributes, "type", &evidence.self_type);
    if let Some(cwd) = evidence.cwd.as_deref() {
        append_xml_attribute(&mut attributes, "cwd", cwd);
    }
    render_xml_specialized_result(XmlSpecializedResult {
        root: "self_tool_result",
        task,
        status,
        attributes,
        content: &evidence.content,
        error_type: evidence.error_type.as_deref(),
        output_time_ms,
        retention,
    })
}

#[cfg(test)]
pub(crate) fn render_xml_self_tool_result(
    action_name: Option<&str>,
    status: ActionStatus,
    evidence: &SelfToolResultEvidence,
    output_time_ms: i64,
) -> String {
    render_xml_self_tool_result_with_retention(
        action_name,
        status,
        evidence,
        output_time_ms,
        Retention::Head,
    )
}

pub(crate) fn render_xml_action_result_with_retention(
    action: &str,
    action_name: Option<&str>,
    result: &str,
    output_time_ms: i64,
    retention: Retention,
) -> String {
    let display_name = action_name
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or(action);
    let escaped_name = escape_xml_attribute(display_name);
    let output = result.trim();
    let output_id = action_output_id(result, output_time_ms);
    let output_tag = format!("output_id_{output_id}");
    let prefix = format!("<action_result><{action} name=\"{escaped_name}\"><{output_tag}>");
    let suffix = format!("</{output_tag}></{action}></action_result>");
    let body_budget = MAX_ACTION_RESULT_PROMPT_BYTES
        .saturating_sub(prefix.len())
        .saturating_sub(suffix.len());
    let escaped_result = truncate_xml_result_text_for_budget(output, body_budget, retention);
    format!("{prefix}{escaped_result}{suffix}")
}

#[cfg(test)]
pub(crate) fn render_xml_action_result(
    action: &str,
    action_name: Option<&str>,
    output: &str,
    output_time_ms: i64,
) -> String {
    render_xml_action_result_with_retention(
        action,
        action_name,
        output,
        output_time_ms,
        Retention::Head,
    )
}

fn render_prompt_context_structure(
    boundaries: crate::response_protocol::PromptBoundarySpec,
) -> &'static str {
    if boundaries.uses_xml_role_elements() {
        "Each `<prompt_delta>` is an outer dynamic transport container that may wrap `<USER>`, \
`<ASSISTANT>`, and `<RUNTIME>` entries in chronological order. A runtime entry \
`[BEGIN TURN turn_id: <id>]` opens a logical user turn; all following entries, even \
across prompt deltas, belong to that turn until the next BEGIN TURN marker. Initial \
user input uses `<USER>` and later input in the same turn uses \
`<USER kind=\"supplement\">`. Static system content is separate in `<Timem System Prompt>`."
    } else {
        "A dynamic delta starts with `[BEGIN DELTA delta_id: <id>, time_ms: <time>]` and extends through every following provider-native message until the next BEGIN DELTA marker or the end of the current model input. Deltas are transport batches, not user turns. A RUNTIME entry `[BEGIN TURN turn_id: <id>]` opens a logical user turn; all following USER, ASSISTANT, RUNTIME, and native tool-call/result messages, even across deltas, belong to that turn until the next BEGIN TURN marker. Initial user input uses `## USER`; later input in the same turn uses `## USER (supplement)`. There is no END DELTA or END TURN marker. Static system content is enclosed separately by the system-prompt boundaries."
    }
}

fn render_prompt_delta_example(
    boundaries: crate::response_protocol::PromptBoundarySpec,
    assistant_heading: &str,
) -> String {
    let mut example = boundaries.render_delta_open("pd_1", 123);
    let roles = [
        (
            VisiblePromptRole::User,
            "new user input, or user supplement entered while the current turn was already in\nprogress.",
        ),
        (
            VisiblePromptRole::You,
            match boundaries.role_boundary {
                crate::response_protocol::PromptRoleBoundary::XmlElement => {
                    "this whole xml-root is your response"
                }
                crate::response_protocol::PromptRoleBoundary::MarkdownHeading => {
                    "your response in this round"
                }
            },
        ),
        (
            VisiblePromptRole::Runtime,
            "Timem Runtime's feedback, tips, etc.\nRUNTIME's 'TIPS' will occasionally show up. They are the philosophy you should really seriously respect.",
        ),
    ];

    example.push_str("\nUse the delta `id` for context maintenance when needed.\n");
    example.push('\n');
    example.push_str(
        &boundaries.render_role_open(VisiblePromptRole::Runtime.label(&boundaries), None),
    );
    example.push_str("\n[BEGIN TURN turn_id: turn_1]");
    if let Some(close) =
        boundaries.render_role_close(VisiblePromptRole::Runtime.label(&boundaries), None)
    {
        example.push('\n');
        example.push_str(&close);
    }
    example.push('\n');
    for (role, body) in roles {
        example.push('\n');
        example.push_str(&boundaries.render_role_open(
            role.label(&boundaries),
            role.assistant_id(assistant_heading),
        ));
        example.push('\n');
        example.push_str(body);
        if let Some(close) = boundaries.render_role_close(
            role.label(&boundaries),
            role.assistant_id(assistant_heading),
        ) {
            example.push('\n');
            example.push_str(&close);
        }
        example.push('\n');
    }
    example.push('\n');
    example.push_str(boundaries.delta_close());
    example
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn render_static_prompt(
    static_prompt: &str,
    capabilities: &CapabilityRegistry,
    protocol_suite: &dyn ResponseProtocolSuite,
    assistant_heading: &str,
    startup_stamp: &str,
) -> String {
    render_static_prompt_for_mode(
        static_prompt,
        capabilities,
        protocol_suite,
        assistant_heading,
        startup_stamp,
        ToolCallMode::Inline,
    )
}

pub(crate) fn render_static_prompt_for_mode(
    static_prompt: &str,
    capabilities: &CapabilityRegistry,
    protocol_suite: &dyn ResponseProtocolSuite,
    assistant_heading: &str,
    startup_stamp: &str,
    tool_call_mode: ToolCallMode,
) -> String {
    render_static_prompt_for_mode_with_preferences(
        static_prompt,
        capabilities,
        protocol_suite,
        assistant_heading,
        startup_stamp,
        tool_call_mode,
        InterfacePreferences::default(),
    )
}

pub(crate) fn render_static_prompt_for_mode_with_preferences(
    static_prompt: &str,
    capabilities: &CapabilityRegistry,
    protocol_suite: &dyn ResponseProtocolSuite,
    assistant_heading: &str,
    startup_stamp: &str,
    tool_call_mode: ToolCallMode,
    interface_preferences: InterfacePreferences,
) -> String {
    // 1. Fill {{RESPONSE_PROTOCOL_SECTION}} from protocol suite
    let protocol_section = if tool_call_mode == ToolCallMode::Native {
        NATIVE_PROTOCOL_SECTION.to_string()
    } else {
        protocol_suite.protocol_prompt_section()
    };
    let ui_preference = match interface_preferences.assistant_response_format {
        AssistantResponseFormat::Unspecified => "a format compatible with the active interface",
        AssistantResponseFormat::Markdown => "Markdown",
        AssistantResponseFormat::PlainText => "plain-text",
    };
    let with_protocol = static_prompt.replace("{{UI_PREFERENCE}}", ui_preference);
    let tool_discovery_instruction = if interface_preferences.claude_codex_tool_discovery {
        r#"If a task appears to involve some specific skill out of your scope, maybe in third-party agent's reusable skill or tool, search:
1. Infer the required capability from intent, not a named skill.
2. Inspect exposed tools, project/user Claude and Codex skill directories, and enabled plugin paths.
3. Cover Linux, macOS, and Windows locations, including symlinks and junctions.
4. Use available platform-native tools to enumerate files. Follow linked directories safely, prevent cycles, and do not use methods that may omit them.
5. Match SKILL.md frontmatter (name, description, requires) or head part to the task.
6. Read only matched instructions and required references.
7. Verify dependencies, authentication, permissions, and a minimal read-only call when possible.
8. Report candidate, loaded, or usable based only on verified evidence; disclose incomplete discovery."#
    } else {
        ""
    };
    let with_protocol = with_protocol.replace(
        "{{CLAUDE_CODEX_TOOL_DISCOVERY_INSTRUCTION}}",
        tool_discovery_instruction,
    );
    let with_protocol = with_protocol.replace("{{RESPONSE_PROTOCOL_SECTION}}", &protocol_section);
    let response_mode_instruction = if tool_call_mode == ToolCallMode::Native {
        NATIVE_RESPONSE_MODE_INSTRUCTION
    } else {
        INLINE_RESPONSE_MODE_INSTRUCTION
    };
    let with_protocol =
        with_protocol.replace("{{RESPONSE_MODE_INSTRUCTION}}", response_mode_instruction);
    let with_protocol =
        with_protocol.replace("{{CURRENT_PROTOCOL_LANG}}", protocol_suite.lang_format());
    let with_protocol = with_protocol.replace(
        "{{PROMPT_CONTEXT_STRUCTURE}}",
        render_prompt_context_structure(*protocol_suite.prompt_boundaries()),
    );
    let with_protocol = with_protocol.replace(
        "{{PROMPT_DELTA_EXAMPLE}}",
        &render_prompt_delta_example(
            *protocol_suite.prompt_boundaries(),
            assistant_heading.trim(),
        ),
    );
    let assistant_heading = assistant_heading.trim();
    let with_protocol = with_protocol.replace("{{ASSSISTANT_ID}}", assistant_heading);
    let with_protocol = with_protocol.replace("ASSSISTANT_ID", assistant_heading);
    let with_protocol = with_protocol.replace("{{STARTUP_STAMP}}", startup_stamp);
    let tool_catalog_heading = if tool_call_mode == ToolCallMode::Native {
        NATIVE_BUILTIN_TOOL_DESCRIPTIONS_HEADING
    } else {
        INLINE_TOOL_CATALOG_SECTION_HEADING
    };
    let with_protocol =
        with_protocol.replace("{{TOOL_CATALOG_SECTION_HEADING}}", tool_catalog_heading);
    // 2. Fill {{TOOL_CATALOG}} from capabilities
    let with_caps = if tool_call_mode == ToolCallMode::Native {
        with_protocol.replace(
            "{{TOOL_CATALOG}}",
            &capabilities.render_native_builtin_tool_descriptions_markdown(),
        )
    } else {
        capabilities.enrich_static_prompt_for_protocol(&with_protocol, protocol_suite.lang_format())
    };
    // 3. Fill {{RESPONSE_V1_SCHEMA}} from prompt_spec
    let static_prompt = prompt_spec::enrich_static_prompt_with_response_schema(
        &with_caps,
        protocol_suite.response_schema_summary(),
    );

    protocol_suite
        .prompt_boundaries()
        .wrap_static_prompt(&static_prompt)
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn render_prompt_with_rendered_static(
    rendered_static_prompt: &str,
    deltas: &[PromptDelta],
    assistant_heading: &str,
    protocol_suite: &dyn ResponseProtocolSuite,
) -> String {
    render_prompt_with_rendered_static_for_mode(
        rendered_static_prompt,
        deltas,
        assistant_heading,
        protocol_suite,
        ToolCallMode::Inline,
    )
}

pub(crate) fn render_prompt_with_rendered_static_for_mode(
    rendered_static_prompt: &str,
    deltas: &[PromptDelta],
    assistant_heading: &str,
    protocol_suite: &dyn ResponseProtocolSuite,
    tool_call_mode: ToolCallMode,
) -> String {
    let mut out = rendered_static_prompt.to_string();

    for delta in deltas {
        let slices = render_delta_slices_for_mode(delta, tool_call_mode);
        if slices.is_empty() {
            continue;
        }
        out.push('\n');
        let boundaries = protocol_suite.prompt_boundaries();
        out.push_str(&boundaries.render_delta_open(&delta.delta_id, delta.time_ms));
        let mut last_role: Option<VisiblePromptRole> = None;
        let mut last_was_action_result = false;
        let mut last_was_raw_xml = false;
        for slice in slices {
            if is_raw_xml_assistant_response(&slice.prompt_type, boundaries) {
                if let Some(previous_role) = last_role.take() {
                    if let Some(close) = boundaries.render_role_close(
                        previous_role.label(boundaries),
                        previous_role.assistant_id(assistant_heading),
                    ) {
                        out.push_str(&close);
                        out.push('\n');
                    }
                }
                if !last_was_raw_xml {
                    out.push('\n');
                }
                out.push_str(&slice.text);
                last_was_raw_xml = true;
                last_was_action_result = false;
                continue;
            }

            if last_was_raw_xml {
                out.push('\n');
                last_was_raw_xml = false;
            }
            let role = visible_role(&slice.prompt_type);
            if last_role != Some(role) {
                if let Some(previous_role) = last_role {
                    if let Some(close) = boundaries.render_role_close(
                        previous_role.label(boundaries),
                        previous_role.assistant_id(assistant_heading),
                    ) {
                        out.push_str(&close);
                        out.push('\n');
                    }
                }
                out.push('\n');
                out.push_str(&role.render_open(boundaries, assistant_heading));
                out.push('\n');
                last_role = Some(role);
                last_was_action_result = false;
            }
            let is_action_result = is_action_result_prompt_type(&slice.prompt_type);
            if is_action_result && !last_was_action_result {
                if let Some(heading) = protocol_suite.action_result_heading() {
                    out.push('\n');
                    out.push_str(heading);
                    out.push('\n');
                }
            }
            out.push('\n');
            if is_action_result {
                out.push_str(truncate_action_result_for_prompt(&slice.text).trim());
            } else if boundaries.uses_xml_role_elements() {
                out.push_str(&escape_xml_text(slice.text.trim()));
            } else {
                out.push_str(slice.text.trim());
            }
            out.push('\n');
            last_was_action_result = is_action_result;
        }
        if last_was_raw_xml {
            out.push('\n');
        }
        if let Some(role) = last_role {
            if let Some(close) = boundaries
                .render_role_close(role.label(boundaries), role.assistant_id(assistant_heading))
            {
                out.push_str(&close);
                out.push('\n');
            }
        }
        out.push('\n');
        out.push_str(boundaries.delta_close());
    }

    out.push_str("\n\n");
    if tool_call_mode == ToolCallMode::Native {
        out.push_str(NATIVE_RESPONSE_TRAILER);
    } else {
        out.push_str(&formatted_response_trailer(
            protocol_suite.response_shape_hint(),
            assistant_heading,
        ));
    }
    out
}

pub(crate) fn render_prompt_slices(deltas: &[PromptDelta]) -> Vec<PromptSlice> {
    deltas
        .iter()
        .flat_map(render_delta_slices)
        .collect::<Vec<_>>()
}

fn render_delta_slices_for_mode(
    delta: &PromptDelta,
    tool_call_mode: ToolCallMode,
) -> Vec<PromptSlice> {
    render_delta_slices(delta)
        .into_iter()
        .filter(|slice| {
            tool_call_mode != ToolCallMode::Native
                || !matches!(
                    slice.prompt_type.as_str(),
                    "mcp_capability_catalog" | "mcp_capability_update"
                )
        })
        .collect()
}

pub(crate) fn render_delta_slices(delta: &PromptDelta) -> Vec<PromptSlice> {
    delta
        .slices
        .iter()
        .filter(|slice| !delta.hidden_slice_ids.contains(&slice.slice_id))
        .cloned()
        .collect()
}

#[cfg(test)]
#[path = "../tests/unit/prompt_render_tests.rs"]
mod tests;
