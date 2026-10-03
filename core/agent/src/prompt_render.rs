use crate::capability::CapabilityRegistry;
use crate::prompt_spec;
use crate::response_protocol::{PromptBoundarySpec, ResponseProtocolSuite};
use crate::tool_result_gate::{self, Retention};
use crate::{PromptDelta, PromptSlice, ToolCallMode};
use timem_ui_contract::preferences::{AssistantResponseFormat, InterfacePreferences};

pub(crate) const RESPONSE_TRAILER: &str =
    "Please continue the work and respond as protocol requires in user's language:";
pub(crate) const NATIVE_RESPONSE_TRAILER: &str = "Continue the work and express thought in the user's language.  Use tools smartly. When all work is genuinely done, call the task_finished tool with the complete final answer as its summary:";
pub(crate) const CONTEXT_COMPRESS_REQUIRED_TRAILER: &str =
    "Context is too long. Compact context as the tool context_compress desc suggests. Use this reasoning pass to carefully review the context and preserve essential decisions, constraints, and unfinished work. Your tool calls must start with context_compress:";
pub(crate) const MANUAL_CONTEXT_COMPRESS_TRAILER: &str =
    "User manually requests context compression. Compact context as the tool context_compress desc suggests, before further work. Use this reasoning pass to carefully review the context and preserve essential decisions, constraints, and unfinished work. Your tool calls must start with context_compress:";
const NATIVE_PROTOCOL_SECTION: &str = "## Tool Calling\n\nCapabilities are provided through the model API. Call them through the API tool-call channel. You may request independent calls together. Text accompanying calls is a user-visible progress note. A response without tool calls does not finish the turn; explicitly call the task_finished tool with the final answer to end it. `context_compress` may be followed by other capability calls in the same response, but it must be the first call. Later calls run only after compaction succeeds.";
const NATIVE_RESPONSE_MODE_INSTRUCTION: &str = "Use the API tool-call channel for runtime capabilities. Ordinary response text is user-visible, you should report to user your progress often, or answer questions while working; text without tool calls keeps the loop running; call task_finished to end it.";
const INLINE_RESPONSE_MODE_INSTRUCTION: &str =
    "Your response MUST be exactly protocol-compliant in the response protocol below.";
const INLINE_TOOL_CATALOG_SECTION_HEADING: &str = "## Actions\n\nGenerate actions to drive the runtime to do things for you. There are several builtin actions:\n\n### Available capabilities";
const NATIVE_BUILTIN_TOOL_DESCRIPTIONS_HEADING: &str =
    "## Built-in Tool Descriptions\n\nBuilt-in tool parameter schemas are provided separately through the model API. One response can reasonably contain multiple tool calls for better performance.";
pub(crate) const MAX_ACTION_RESULT_PROMPT_BYTES: usize =
    tool_result_gate::MAX_MODEL_TOOL_RESULT_BYTES;
pub(crate) const REASONING_INTENSITY_UPGRADE_TRAILER: &str = "This request is using stronger reasoning than the normal H0 baseline. Use this opportunity to provide more direction and methodology for the work, and to identify and correct possible mistakes or weak assumptions through reflection. Please continue the work and respond as protocol requires in user's language:";
pub(crate) const NATIVE_REASONING_INTENSITY_UPGRADE_TRAILER: &str = "This request is using stronger reasoning than the normal H0 baseline. Use this opportunity to provide more direction and methodology for the work, and to identify and correct possible mistakes or weak assumptions through reflection. Continue the work and express thought in the user's language. Use tools smartly. When all work is genuinely done, call the task_finished tool with the complete final answer as its summary:";
const LEGACY_REASONING_REVIEW_TRAILER: &str = "Note: reasoning effort is enabled for this request, while most work rounds run without it. Take advantage of this reasoning pass to review current work, update or adjust the work direction/plan if needed, and express in remarks as appropriate.";
pub(crate) const REASONING_UPGRADED_CONTEXT_COMPRESS_TRAILER: &str = "This request is using stronger reasoning than the normal H0 baseline. Use this opportunity to provide more direction and methodology for the work, and to identify and correct possible mistakes or weak assumptions through reflection. Context is too long. Compact context as the tool context_compress desc suggests. Carefully review the context and preserve essential decisions, constraints, and unfinished work. Your tool calls must start with context_compress:";
pub(crate) const REASONING_UPGRADED_MANUAL_CONTEXT_COMPRESS_TRAILER: &str = "This request is using stronger reasoning than the normal H0 baseline. Use this opportunity to provide more direction and methodology for the work, and to identify and correct possible mistakes or weak assumptions through reflection. User manually requests context compression. Compact context as the tool context_compress desc suggests, before further work. Carefully review the context and preserve essential decisions, constraints, and unfinished work. Your tool calls must start with context_compress:";

pub(crate) fn is_structured_action_result_envelope(text: &str) -> bool {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text.trim()) else {
        return false;
    };
    let Some(result) = value
        .get("action_result")
        .and_then(serde_json::Value::as_object)
    else {
        return false;
    };
    result
        .get("tool_call_id")
        .is_some_and(serde_json::Value::is_string)
        && result
            .get("runtime_metadata")
            .is_some_and(serde_json::Value::is_object)
        && result
            .get("tool_output")
            .is_some_and(serde_json::Value::is_object)
}

pub(crate) fn truncate_action_result_for_prompt(text: &str) -> String {
    if is_structured_action_result_envelope(text) {
        text.to_string()
    } else {
        tool_result_gate::fit(text, MAX_ACTION_RESULT_PROMPT_BYTES, Retention::Head)
    }
}

pub(crate) fn formatted_response_trailer(
    _response_shape_hint: &str,
    _assistant_heading: &str,
) -> String {
    RESPONSE_TRAILER.to_string()
}

pub(crate) fn apply_reasoning_intensity_upgrade_trailer(rendered_prompt: &str) -> String {
    let (body, trailer) = split_formatted_response_trailer(rendered_prompt);
    let upgraded_trailer = match trailer.as_deref() {
        Some(CONTEXT_COMPRESS_REQUIRED_TRAILER) => REASONING_UPGRADED_CONTEXT_COMPRESS_TRAILER,
        Some(MANUAL_CONTEXT_COMPRESS_TRAILER) => REASONING_UPGRADED_MANUAL_CONTEXT_COMPRESS_TRAILER,
        Some(NATIVE_RESPONSE_TRAILER) => NATIVE_REASONING_INTENSITY_UPGRADE_TRAILER,
        _ => REASONING_INTENSITY_UPGRADE_TRAILER,
    };
    format!("{}\n\n{}", body.trim_end(), upgraded_trailer)
}

pub(crate) fn split_formatted_response_trailer(rendered_prompt: &str) -> (&str, Option<String>) {
    let trimmed = rendered_prompt.trim_end();
    for trailer in [
        RESPONSE_TRAILER,
        NATIVE_RESPONSE_TRAILER,
        CONTEXT_COMPRESS_REQUIRED_TRAILER,
        MANUAL_CONTEXT_COMPRESS_TRAILER,
        REASONING_INTENSITY_UPGRADE_TRAILER,
        NATIVE_REASONING_INTENSITY_UPGRADE_TRAILER,
        REASONING_UPGRADED_CONTEXT_COMPRESS_TRAILER,
        REASONING_UPGRADED_MANUAL_CONTEXT_COMPRESS_TRAILER,
        LEGACY_REASONING_REVIEW_TRAILER,
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
    UserResumeDirectly,
    You,
    ContextCompressionSummary,
    Runtime,
}

impl VisiblePromptRole {
    fn label(self, spec: &PromptBoundarySpec) -> &str {
        match self {
            VisiblePromptRole::User
            | VisiblePromptRole::UserSupplement
            | VisiblePromptRole::UserResumeDirectly => spec.user_role,
            VisiblePromptRole::You | VisiblePromptRole::ContextCompressionSummary => {
                spec.assistant_role
            }
            VisiblePromptRole::Runtime => spec.runtime_role,
        }
    }

    fn assistant_id(self, assistant_heading: &str) -> Option<&str> {
        matches!(
            self,
            VisiblePromptRole::You | VisiblePromptRole::ContextCompressionSummary
        )
        .then_some(assistant_heading)
    }

    fn render_open(self, spec: &PromptBoundarySpec, assistant_heading: &str) -> String {
        if self == VisiblePromptRole::UserResumeDirectly {
            if spec.uses_xml_role_elements() {
                format!("<{} kind=\"user resume directly\">", spec.user_role)
            } else {
                format!("## {} (user resume directly)", spec.user_role)
            }
        } else if self == VisiblePromptRole::UserSupplement {
            if spec.uses_xml_role_elements() {
                format!("<{} kind=\"supplement\">", spec.user_role)
            } else {
                format!("## {} (supplement)", spec.user_role)
            }
        } else if self == VisiblePromptRole::ContextCompressionSummary {
            if spec.uses_xml_role_elements() {
                format!(
                    "<{} kind=\"context_compression_summary\">",
                    spec.assistant_role
                )
            } else {
                format!("## {} (context compression summary)", assistant_heading)
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
        "user_resume_directly" => VisiblePromptRole::UserResumeDirectly,
        "llm_response" | "llm_response_raw_xml" | "llm_free_talk" => VisiblePromptRole::You,
        "context_compression_summary" => VisiblePromptRole::ContextCompressionSummary,
        "result_of_llm_action" | "response_repair" | "context_compressed" => {
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

fn render_prompt_context_structure(
    boundaries: crate::response_protocol::PromptBoundarySpec,
) -> &'static str {
    if boundaries.uses_xml_role_elements() {
        "Each `<prompt_delta>` is an outer dynamic transport container that may wrap `<USER>`, \
`<ASSISTANT>`, and `<RUNTIME>` entries in chronological order. Initial \
user input uses `<USER>` and later input in the same turn uses \
`<USER kind=\"supplement\">`. Explicit direct resume uses `<USER kind=\"user resume directly\">` with an empty body. User-kind attributes describe structured behavior, not user-authored text. Readable UTC input timestamps annotate non-empty user questions and supplements only, never synthetic transport messages. Restart/supporting context precedes the user entry; later runtime observations remain after it. Static system content is separate in `<Timem System Prompt>`."
    } else {
        "A dynamic delta starts with `[BEGIN DELTA delta_id: <id>]` and extends through every following provider-native message until the next BEGIN DELTA marker or the end of the current model input. Deltas are transport batches, not user turns. Initial user input uses `## USER`; later input in the same turn uses `## USER (supplement)`. Explicit direct resume uses the header `## USER (user resume directly)` with an empty body (XML: `<USER kind=\"user resume directly\">`). These annotations describe structured user behavior, not user-authored text. Readable UTC input timestamps annotate non-empty user questions and supplements only, never synthetic transport messages. Restart/supporting context precedes the user entry; later runtime observations remain after it. There is no END DELTA marker. Static system content is enclosed separately by the system-prompt boundaries."
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

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn render_static_prompt(
    static_prompt: &str,
    capabilities: &CapabilityRegistry,
    protocol_suite: &dyn ResponseProtocolSuite,
    assistant_heading: &str,
) -> String {
    render_static_prompt_for_mode(
        static_prompt,
        capabilities,
        protocol_suite,
        assistant_heading,
        ToolCallMode::Inline,
    )
}

pub(crate) fn render_static_prompt_for_mode(
    static_prompt: &str,
    capabilities: &CapabilityRegistry,
    protocol_suite: &dyn ResponseProtocolSuite,
    assistant_heading: &str,
    tool_call_mode: ToolCallMode,
) -> String {
    render_static_prompt_for_mode_with_preferences(
        static_prompt,
        capabilities,
        protocol_suite,
        assistant_heading,
        tool_call_mode,
        InterfacePreferences::default(),
    )
}

pub(crate) fn render_static_prompt_for_mode_with_preferences(
    static_prompt: &str,
    capabilities: &CapabilityRegistry,
    protocol_suite: &dyn ResponseProtocolSuite,
    assistant_heading: &str,
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
    append_rendered_deltas_for_mode(
        &mut out,
        deltas,
        assistant_heading,
        protocol_suite,
        tool_call_mode,
    );
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

pub(crate) fn append_rendered_deltas_for_mode(
    out: &mut String,
    deltas: &[PromptDelta],
    assistant_heading: &str,
    protocol_suite: &dyn ResponseProtocolSuite,
    tool_call_mode: ToolCallMode,
) {
    for delta in deltas {
        let slices = render_delta_slices_for_mode(delta, tool_call_mode);
        if slices.is_empty() && tool_call_mode != ToolCallMode::Native {
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
            // Semantic input kinds, not provider message roles: native transport
            // uses user messages for synthetic delta separators too.
            if matches!(
                slice.prompt_type.as_str(),
                "user_question" | "user_supplement"
            ) && !slice.text.trim().is_empty()
            {
                if let Ok(time) =
                    time::OffsetDateTime::from_unix_timestamp(slice.time_ms.div_euclid(1000))
                {
                    out.push_str(&format!(
                        "\n[User input time: {:04}-{:02}-{:02} {:02}:{:02}:{:02} UTC]\n",
                        time.year(),
                        time.month() as u8,
                        time.day(),
                        time.hour(),
                        time.minute(),
                        time.second()
                    ));
                }
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
