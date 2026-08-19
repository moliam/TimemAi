use crate::capability::CapabilityRegistry;
use crate::prompt_spec;
use crate::response_protocol::{PromptBoundarySpec, ResponseProtocolSuite};
use crate::{
    ActionStatus, BashResultEvidence, MemmgrResultEvidence, PromptDelta, PromptSlice,
    ReadfileResultEvidence, SelfToolResultEvidence,
};
use std::hash::{DefaultHasher, Hash, Hasher};

pub(crate) const RESPONSE_TRAILER: &str =
    "Please continue the work and respond as protocol requires in user's language:";
pub(crate) const MAX_ACTION_RESULT_PROMPT_BYTES: usize = 32 * 1024;

pub(crate) fn truncate_action_result_for_prompt(text: &str) -> String {
    if text.len() <= MAX_ACTION_RESULT_PROMPT_BYTES {
        return text.to_string();
    }
    let mut retained_budget = MAX_ACTION_RESULT_PROMPT_BYTES;
    loop {
        let mut retained_end = retained_budget.min(text.len());
        while retained_end > 0 && !text.is_char_boundary(retained_end) {
            retained_end -= 1;
        }
        let truncated_words = text[retained_end..].split_whitespace().count();
        let notice = format!(
            "!!!Too long, {truncated_words} words truncated. Generate more actions if necessary !!!"
        );
        let next_budget = MAX_ACTION_RESULT_PROMPT_BYTES.saturating_sub(notice.len() + 1);
        if next_budget == retained_budget {
            return format!("{}\n{notice}", text[..retained_end].trim_end());
        }
        retained_budget = next_budget;
    }
}

pub(crate) fn formatted_response_trailer(
    _response_shape_hint: &str,
    _assistant_heading: &str,
) -> String {
    RESPONSE_TRAILER.to_string()
}

pub(crate) fn split_formatted_response_trailer(rendered_prompt: &str) -> (&str, Option<String>) {
    let trimmed = rendered_prompt.trim_end();
    let marker = format!("\n\n{RESPONSE_TRAILER}");
    let Some(trailer_start) = trimmed.strip_suffix(&marker).map(str::len) else {
        return (rendered_prompt, None);
    };
    let prefix = trimmed[..trailer_start].trim_end();
    (prefix, Some(RESPONSE_TRAILER.to_string()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VisiblePromptRole {
    User,
    You,
    Runtime,
}

impl VisiblePromptRole {
    fn label(self, spec: &PromptBoundarySpec) -> &str {
        match self {
            VisiblePromptRole::User => spec.user_role,
            VisiblePromptRole::You => spec.assistant_role,
            VisiblePromptRole::Runtime => spec.runtime_role,
        }
    }

    fn assistant_id(self, assistant_heading: &str) -> Option<&str> {
        (self == VisiblePromptRole::You).then_some(assistant_heading)
    }
}

fn visible_role(prompt_type: &str) -> VisiblePromptRole {
    match prompt_type {
        "user_question" | "user_supplement" => VisiblePromptRole::User,
        "llm_response" | "llm_response_raw_xml" | "llm_free_talk" => VisiblePromptRole::You,
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

fn escaped_xml_text_len(text: &str) -> usize {
    text.chars()
        .map(|ch| match ch {
            '&' => 5,
            '<' | '>' => 4,
            _ => ch.len_utf8(),
        })
        .sum()
}

fn xml_prefix_end_for_escaped_budget(text: &str, budget: usize) -> usize {
    let mut escaped_bytes = 0usize;
    let mut end = 0usize;
    for (index, ch) in text.char_indices() {
        let char_bytes = match ch {
            '&' => 5,
            '<' | '>' => 4,
            _ => ch.len_utf8(),
        };
        if escaped_bytes.saturating_add(char_bytes) > budget {
            break;
        }
        escaped_bytes += char_bytes;
        end = index + ch.len_utf8();
    }
    end
}

fn truncate_xml_result_text_for_budget(text: &str, budget: usize) -> String {
    if escaped_xml_text_len(text) <= budget {
        return escape_xml_text(text);
    }

    let mut retained_end = xml_prefix_end_for_escaped_budget(text, budget);
    loop {
        let truncated_words = text[retained_end..].split_whitespace().count();
        let notice = format!(
            "!!!Too long, {truncated_words} words truncated. Generate more actions if necessary !!!"
        );
        let notice_bytes = escaped_xml_text_len(&notice).saturating_add(1);
        let retained_budget = budget.saturating_sub(notice_bytes);
        let next_end = xml_prefix_end_for_escaped_budget(text, retained_budget);
        if next_end == retained_end {
            return format!(
                "{}\n{}",
                escape_xml_text(text[..retained_end].trim_end()),
                escape_xml_text(&notice)
            );
        }
        retained_end = next_end;
    }
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

fn truncate_raw_text_for_budget(text: &str, budget: usize) -> String {
    if text.len() <= budget {
        return text.to_string();
    }
    let mut retained_budget = budget;
    loop {
        let mut retained_end = retained_budget.min(text.len());
        while retained_end > 0 && !text.is_char_boundary(retained_end) {
            retained_end -= 1;
        }
        let truncated_words = text[retained_end..].split_whitespace().count();
        let notice = format!(
            "!!!Too long, {truncated_words} words truncated. Generate more actions if necessary !!!"
        );
        let next_budget = budget.saturating_sub(notice.len() + 1);
        if next_budget == retained_budget {
            return format!("{}\n{notice}", text[..retained_end].trim_end());
        }
        retained_budget = next_budget;
    }
}

pub(crate) fn render_xml_bash_result(
    action_name: Option<&str>,
    status: ActionStatus,
    evidence: &BashResultEvidence,
    output_time_ms: i64,
) -> String {
    let task = action_name
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or("run_bash");
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
    let timed_out = evidence
        .timed_out
        .then_some(" timed_out=\"true\"")
        .unwrap_or_default();
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
        let stdout = truncate_raw_text_for_budget(evidence.stdout.trim_end(), stdout_budget);
        let stderr = truncate_raw_text_for_budget(evidence.stderr.trim_end(), stderr_budget);
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
    let content = truncate_raw_text_for_budget(content.trim_end(), body_budget);
    format!("{prefix}{open}{content}{close}{suffix}")
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

fn render_xml_specialized_result(
    root: &str,
    task: &str,
    status: ActionStatus,
    mut attributes: String,
    content: &str,
    error_type: Option<&str>,
    output_time_ms: i64,
) -> String {
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
    let content = truncate_raw_text_for_budget(content.trim_end(), body_budget);
    format!("{prefix}{open}{content}{close}{suffix}")
}

pub(crate) fn render_xml_readfile_result(
    action_name: Option<&str>,
    status: ActionStatus,
    evidence: &ReadfileResultEvidence,
    output_time_ms: i64,
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
    render_xml_specialized_result(
        "readfile_result",
        task,
        status,
        attributes,
        &evidence.content,
        evidence.error_type.as_deref(),
        output_time_ms,
    )
}

pub(crate) fn render_xml_memmgr_result(
    action_name: Option<&str>,
    status: ActionStatus,
    evidence: &MemmgrResultEvidence,
    output_time_ms: i64,
) -> String {
    let task = action_name
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or("memmgr");
    let mut attributes = String::new();
    append_xml_attribute(&mut attributes, "type", &evidence.memory_type);
    append_xml_attribute(&mut attributes, "op", &evidence.op);
    render_xml_specialized_result(
        "memmgr_result",
        task,
        status,
        attributes,
        &evidence.content,
        evidence.error_type.as_deref(),
        output_time_ms,
    )
}

pub(crate) fn render_xml_self_tool_result(
    action_name: Option<&str>,
    status: ActionStatus,
    evidence: &SelfToolResultEvidence,
    output_time_ms: i64,
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
    render_xml_specialized_result(
        "self_tool_result",
        task,
        status,
        attributes,
        &evidence.content,
        evidence.error_type.as_deref(),
        output_time_ms,
    )
}

pub(crate) fn render_xml_action_result(
    action: &str,
    action_name: Option<&str>,
    result: &str,
    output_time_ms: i64,
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
    let escaped_result = truncate_xml_result_text_for_budget(output, body_budget);
    format!("{prefix}{escaped_result}{suffix}")
}

fn render_prompt_context_structure(
    boundaries: crate::response_protocol::PromptBoundarySpec,
) -> &'static str {
    if boundaries.uses_xml_role_elements() {
        "Each `<prompt_delta>` is an outer dynamic container that may wrap `<USER>`, \
`<ASSISTANT>`, and `<RUNTIME>` entries in chronological order. Static system \
content is separate in `<Timem System Prompt>`."
    } else {
        "Each dynamic delta is enclosed by `[BEGIN DELTA]` and `[END DELTA]`, with \
USER, ASSISTANT, and RUNTIME entries rendered as headings in chronological order. \
Static system content is enclosed separately by the system-prompt boundaries."
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

pub(crate) fn render_static_prompt(
    static_prompt: &str,
    capabilities: &CapabilityRegistry,
    protocol_suite: &dyn ResponseProtocolSuite,
    assistant_heading: &str,
    startup_stamp: &str,
) -> String {
    // 1. Fill {{RESPONSE_PROTOCOL_SECTION}} from protocol suite
    let with_protocol = static_prompt.replace(
        "{{RESPONSE_PROTOCOL_SECTION}}",
        &protocol_suite.protocol_prompt_section(),
    );
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
    // 2. Fill {{TOOL_CATALOG}} from capabilities
    let with_caps = capabilities
        .enrich_static_prompt_for_protocol(&with_protocol, protocol_suite.lang_format());
    // 3. Fill {{RESPONSE_V1_SCHEMA}} from prompt_spec
    let static_prompt = prompt_spec::enrich_static_prompt_with_response_schema(
        &with_caps,
        protocol_suite.response_schema_summary(),
    );

    protocol_suite
        .prompt_boundaries()
        .wrap_static_prompt(&static_prompt)
}

pub(crate) fn render_prompt_with_rendered_static(
    rendered_static_prompt: &str,
    deltas: &[PromptDelta],
    assistant_heading: &str,
    protocol_suite: &dyn ResponseProtocolSuite,
) -> String {
    let mut out = rendered_static_prompt.to_string();

    for delta in deltas {
        let slices = render_delta_slices(delta);
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
                out.push_str(&boundaries.render_role_open(
                    role.label(boundaries),
                    role.assistant_id(assistant_heading),
                ));
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
    out.push_str(&formatted_response_trailer(
        protocol_suite.response_shape_hint(),
        assistant_heading,
    ));
    out
}

pub(crate) fn render_prompt_slices(deltas: &[PromptDelta]) -> Vec<PromptSlice> {
    deltas
        .iter()
        .flat_map(render_delta_slices)
        .collect::<Vec<_>>()
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
