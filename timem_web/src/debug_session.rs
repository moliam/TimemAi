use std::{
    collections::{BTreeMap, VecDeque},
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const ACTION_BUCKET_MS: u64 = 20;
const ACTION_LAST_BUCKET_MS: u64 = 1_000;
const LLM_BUCKET_MS: u64 = 200;
const LLM_LAST_BUCKET_MS: u64 = 30_000;
const MAX_LLM_RESPONSE_DUMP_ENTRIES: usize = 10;
const STATISTICS_REFRESH_MS: u64 = 2_000;
static NEXT_DEBUG_STORE_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
pub(crate) struct DebugStore {
    root: PathBuf,
    sessions: Mutex<BTreeMap<String, SessionDebug>>,
    file_render_lock: Mutex<()>,
}

#[derive(Debug)]
struct LlmResponseDumpEntry {
    sequence: u64,
    request_sequence: Option<u64>,
    worker_id: String,
    round: u32,
    received_at_ms: u128,
    content: String,
    tool_calls: Vec<agent_core::NativeToolCall>,
}

#[derive(Debug)]
struct LlmRequestDumpEntry {
    sequence: u64,
    worker_id: String,
    round: u32,
    generated_at_ms: u128,
    prompt: String,
    interaction_request: Option<agent_core::ModelInteractionRequest>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct EndpointKey {
    model: String,
    gateway: String,
    tool_call_mode: String,
}

impl EndpointKey {
    fn from_profile(profile: &agent_core::InteractionProfile) -> Self {
        Self {
            model: profile.model.clone(),
            gateway: profile.gateway.clone(),
            tool_call_mode: profile.resolved_mode.label().to_string(),
        }
    }

    fn pending() -> Self {
        Self {
            model: "unresolved".to_string(),
            gateway: "unresolved".to_string(),
            tool_call_mode: "pending".to_string(),
        }
    }
}

#[derive(Debug, Default)]
struct EndpointDebug {
    profile: Option<agent_core::InteractionProfile>,
    requests: u64,
    successes: u64,
    failures: u64,
    action_cpu_ns: Vec<u64>,
    action_cpu_unavailable: u64,
    llm_latency_ms: Vec<u64>,
    tools_per_response: [u64; 11],
    repairs: BTreeMap<String, u64>,
    runtime_root_repair_help: u64,
}

#[derive(Debug, Default)]
struct SessionDebug {
    request_sequence: u64,
    response_sequence: u64,
    latest_request: Option<LlmRequestDumpEntry>,
    responses: VecDeque<LlmResponseDumpEntry>,
    started_at_ms: u128,
    updated_at_ms: u128,
    active_endpoint_by_worker: BTreeMap<String, EndpointKey>,
    in_flight_by_worker: BTreeMap<String, EndpointKey>,
    latest_request_sequence_by_worker: BTreeMap<String, u64>,
    endpoints: BTreeMap<EndpointKey, EndpointDebug>,
}

impl DebugStore {
    pub(crate) fn create() -> Result<Self, String> {
        #[cfg(unix)]
        let temporary_root = PathBuf::from("/tmp");
        #[cfg(not(unix))]
        let temporary_root = std::env::temp_dir();

        let store_id = NEXT_DEBUG_STORE_ID.fetch_add(1, Ordering::Relaxed);
        let root = temporary_root.join(format!("timem-debug-{}-{store_id}", std::process::id()));
        if root.exists() {
            fs::remove_dir_all(&root)
                .map_err(|error| format!("debug_root_cleanup_failed:{error}"))?;
        }
        create_private_dir(&root)?;
        Ok(Self {
            root,
            sessions: Mutex::new(BTreeMap::new()),
            file_render_lock: Mutex::new(()),
        })
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn session_dir(&self, session_id: &str) -> Result<PathBuf, String> {
        let component = safe_session_component(session_id)?;
        let dir = self.root.join(component);
        create_private_dir(&dir)?;
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| "debug_store_poisoned".to_string())?;
        let now = now_ms();
        let inserted = match sessions.entry(session_id.to_string()) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(SessionDebug {
                    started_at_ms: now,
                    updated_at_ms: now,
                    ..SessionDebug::default()
                });
                true
            }
            std::collections::btree_map::Entry::Occupied(_) => false,
        };
        drop(sessions);
        if inserted {
            self.render_statistics(session_id)?;
            self.render_llm_prompts(session_id)?;
            self.render_llm_responses(session_id)?;
        }
        Ok(dir)
    }

    pub(crate) fn record_prompt(
        &self,
        session_id: &str,
        worker_id: &str,
        round: u32,
        prompt: &str,
        interaction_request: Option<&agent_core::ModelInteractionRequest>,
    ) -> Result<(), String> {
        self.session_dir(session_id)?;
        {
            let mut sessions = self
                .sessions
                .lock()
                .map_err(|_| "debug_store_poisoned".to_string())?;
            let stats = sessions
                .get_mut(session_id)
                .ok_or_else(|| "debug_session_not_found".to_string())?;
            if let Some(previous) = stats.in_flight_by_worker.remove(worker_id) {
                let endpoint = stats.endpoints.entry(previous).or_default();
                endpoint.failures = endpoint.failures.saturating_add(1);
            }
            let endpoint_key = stats
                .active_endpoint_by_worker
                .get(worker_id)
                .cloned()
                .unwrap_or_else(EndpointKey::pending);
            let endpoint = stats.endpoints.entry(endpoint_key.clone()).or_default();
            endpoint.requests = endpoint.requests.saturating_add(1);
            stats
                .in_flight_by_worker
                .insert(worker_id.to_string(), endpoint_key);
            stats.request_sequence = stats.request_sequence.saturating_add(1);
            let generated_at_ms = now_ms();
            stats
                .latest_request_sequence_by_worker
                .insert(worker_id.to_string(), stats.request_sequence);
            stats.latest_request = Some(LlmRequestDumpEntry {
                sequence: stats.request_sequence,
                worker_id: worker_id.to_string(),
                round,
                generated_at_ms,
                prompt: prompt.to_string(),
                interaction_request: interaction_request.cloned(),
            });
            stats.updated_at_ms = generated_at_ms;
        }
        self.render_llm_prompts(session_id)?;
        self.render_statistics(session_id)
    }

    pub(crate) fn record_llm_response(
        &self,
        session_id: &str,
        worker_id: &str,
        round: u32,
        content: &str,
        tool_calls: &[agent_core::NativeToolCall],
    ) -> Result<(), String> {
        self.session_dir(session_id)?;
        {
            let mut sessions = self
                .sessions
                .lock()
                .map_err(|_| "debug_store_poisoned".to_string())?;
            let stats = sessions
                .get_mut(session_id)
                .ok_or_else(|| "debug_session_not_found".to_string())?;
            stats.response_sequence = stats.response_sequence.saturating_add(1);
            let received_at_ms = now_ms();
            let request_sequence = stats
                .latest_request_sequence_by_worker
                .get(worker_id)
                .copied();
            stats.responses.push_front(LlmResponseDumpEntry {
                sequence: stats.response_sequence,
                request_sequence,
                worker_id: worker_id.to_string(),
                round,
                received_at_ms,
                content: content.to_string(),
                tool_calls: tool_calls.to_vec(),
            });
            stats.responses.truncate(MAX_LLM_RESPONSE_DUMP_ENTRIES);
            stats.updated_at_ms = received_at_ms;
        }
        self.render_llm_responses(session_id)
    }

    pub(crate) fn record_llm_latency(
        &self,
        session_id: &str,
        worker_id: &str,
        latency: Duration,
    ) -> Result<(), String> {
        self.update(session_id, |stats| {
            if let Some(endpoint_key) = take_in_flight_endpoint(stats, worker_id) {
                let endpoint = stats.endpoints.entry(endpoint_key).or_default();
                endpoint.successes = endpoint.successes.saturating_add(1);
                endpoint
                    .llm_latency_ms
                    .push(latency.as_millis().min(u64::MAX as u128) as u64);
            }
        })
    }

    pub(crate) fn record_model_failure(
        &self,
        session_id: &str,
        worker_id: &str,
    ) -> Result<(), String> {
        self.update(session_id, |stats| {
            if let Some(endpoint_key) = take_in_flight_endpoint(stats, worker_id) {
                let endpoint = stats.endpoints.entry(endpoint_key).or_default();
                endpoint.failures = endpoint.failures.saturating_add(1);
            }
        })
    }

    pub(crate) fn record_tools_per_response(
        &self,
        session_id: &str,
        worker_id: &str,
        count: usize,
    ) -> Result<(), String> {
        self.update(session_id, |stats| {
            let endpoint = endpoint_for_worker(stats, worker_id);
            endpoint.tools_per_response[count.min(10)] =
                endpoint.tools_per_response[count.min(10)].saturating_add(1);
        })
    }

    pub(crate) fn record_interaction_profile(
        &self,
        session_id: &str,
        worker_id: &str,
        profile: &agent_core::InteractionProfile,
    ) -> Result<(), String> {
        self.update(session_id, |stats| {
            let key = EndpointKey::from_profile(profile);
            stats
                .active_endpoint_by_worker
                .insert(worker_id.to_string(), key.clone());
            stats.endpoints.entry(key).or_default().profile = Some(profile.clone());
        })
    }

    pub(crate) fn record_runtime_root_repair_help(
        &self,
        session_id: &str,
        worker_id: &str,
    ) -> Result<(), String> {
        self.update(session_id, |stats| {
            let endpoint = endpoint_for_worker(stats, worker_id);
            endpoint.runtime_root_repair_help = endpoint.runtime_root_repair_help.saturating_add(1);
        })
    }

    pub(crate) fn record_repair(
        &self,
        session_id: &str,
        worker_id: &str,
        issue: &str,
    ) -> Result<(), String> {
        let category = normalize_repair_category(issue);
        self.update(session_id, |stats| {
            let count = endpoint_for_worker(stats, worker_id)
                .repairs
                .entry(category)
                .or_default();
            *count = count.saturating_add(1);
        })
    }

    pub(crate) fn record_action_cpu(
        &self,
        session_id: &str,
        worker_id: &str,
        cpu_time: Option<Duration>,
    ) -> Result<(), String> {
        self.update(session_id, |stats| match cpu_time {
            Some(duration) => endpoint_for_worker(stats, worker_id)
                .action_cpu_ns
                .push(duration.as_nanos().min(u64::MAX as u128) as u64),
            None => {
                let endpoint = endpoint_for_worker(stats, worker_id);
                endpoint.action_cpu_unavailable = endpoint.action_cpu_unavailable.saturating_add(1);
            }
        })
    }

    pub(crate) fn cleanup(&self) -> Result<(), String> {
        match fs::remove_dir_all(&self.root) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!("debug_root_remove_failed:{error}")),
        }
    }

    fn update(
        &self,
        session_id: &str,
        update: impl FnOnce(&mut SessionDebug),
    ) -> Result<(), String> {
        self.session_dir(session_id)?;
        {
            let mut sessions = self
                .sessions
                .lock()
                .map_err(|_| "debug_store_poisoned".to_string())?;
            let stats = sessions
                .get_mut(session_id)
                .ok_or_else(|| "debug_session_not_found".to_string())?;
            update(stats);
            stats.updated_at_ms = now_ms();
        }
        self.render_statistics(session_id)
    }

    fn render_statistics(&self, session_id: &str) -> Result<(), String> {
        let _render_guard = self
            .file_render_lock
            .lock()
            .map_err(|_| "debug_file_render_lock_poisoned".to_string())?;
        let dir = self.root.join(safe_session_component(session_id)?);
        let body = {
            let sessions = self
                .sessions
                .lock()
                .map_err(|_| "debug_store_poisoned".to_string())?;
            let stats = sessions
                .get(session_id)
                .ok_or_else(|| "debug_session_not_found".to_string())?;
            render_statistics_html(session_id, stats)
        };
        atomic_private_write(&dir.join("statistics.html"), body.as_bytes())
    }

    fn render_llm_prompts(&self, session_id: &str) -> Result<(), String> {
        let _render_guard = self
            .file_render_lock
            .lock()
            .map_err(|_| "debug_file_render_lock_poisoned".to_string())?;
        let dir = self.root.join(safe_session_component(session_id)?);
        let (prompt_body, tool_schema_body) = {
            let sessions = self
                .sessions
                .lock()
                .map_err(|_| "debug_store_poisoned".to_string())?;
            let stats = sessions
                .get(session_id)
                .ok_or_else(|| "debug_session_not_found".to_string())?;
            (
                render_llm_prompt_dump(session_id, stats.latest_request.as_ref()),
                render_tool_schema_dump(session_id, stats.latest_request.as_ref()),
            )
        };
        atomic_private_write(&dir.join("llm_prompt.dump"), prompt_body.as_bytes())?;
        atomic_private_write(&dir.join("tool_schema.dump"), tool_schema_body.as_bytes())
    }

    fn render_llm_responses(&self, session_id: &str) -> Result<(), String> {
        let _render_guard = self
            .file_render_lock
            .lock()
            .map_err(|_| "debug_file_render_lock_poisoned".to_string())?;
        let dir = self.root.join(safe_session_component(session_id)?);
        let body = {
            let sessions = self
                .sessions
                .lock()
                .map_err(|_| "debug_store_poisoned".to_string())?;
            let stats = sessions
                .get(session_id)
                .ok_or_else(|| "debug_session_not_found".to_string())?;
            render_llm_response_dump(session_id, &stats.responses)
        };
        atomic_private_write(&dir.join("llm_response.dump"), body.as_bytes())
    }
}

fn endpoint_for_worker<'a>(stats: &'a mut SessionDebug, worker_id: &str) -> &'a mut EndpointDebug {
    let key = stats
        .active_endpoint_by_worker
        .get(worker_id)
        .cloned()
        .unwrap_or_else(EndpointKey::pending);
    stats.endpoints.entry(key).or_default()
}

fn take_in_flight_endpoint(stats: &mut SessionDebug, worker_id: &str) -> Option<EndpointKey> {
    stats.in_flight_by_worker.remove(worker_id)
}

fn render_llm_prompt_dump(session_id: &str, request: Option<&LlmRequestDumpEntry>) -> String {
    let mut out = String::new();
    out.push_str("TIMEM LLM PROMPT DUMP\n");
    out.push_str(&format!("session_id: {session_id}\n"));
    out.push_str("scope: latest_request_only\n");
    let Some(request) = request else {
        out.push_str("\n(no model requests recorded)\n");
        return out;
    };
    out.push('\n');
    out.push_str("============================================================\n");
    out.push_str(&format!("request_sequence: {}\n", request.sequence));
    out.push_str(&format!("worker_id: {}\n", request.worker_id));
    out.push_str(&format!("round: {}\n", request.round));
    out.push_str(&format!(
        "generated_at: {}\n",
        format_timestamp_ms(request.generated_at_ms)
    ));
    out.push_str(&format!("content_bytes: {}\n", request.prompt.len()));
    if let Some(interaction) = request.interaction_request.as_ref() {
        out.push_str(&format!(
            "tool_call_mode: {}\nparallel_tool_calls: {}\ntool_choice: {:?}\nstatic_builtin_tool_definitions: {}\ndynamic_tool_definitions: {}\nnative_exchanges: {}\n",
            interaction.resolved_mode.label(),
            interaction.parallel_tool_calls,
            interaction.tool_choice,
            interaction.static_tool_count,
            interaction.tools.len().saturating_sub(interaction.static_tool_count),
            interaction.native_exchanges.len(),
        ));
    }
    out.push_str("-------------------- RENDERED PROMPT -----------------------\n");
    out.push_str(&request.prompt);
    if !request.prompt.ends_with('\n') {
        out.push('\n');
    }
    if let Some(interaction) = request
        .interaction_request
        .as_ref()
        .filter(|interaction| interaction.is_native() && !interaction.native_exchanges.is_empty())
    {
        out.push_str("-------------------- NATIVE EXCHANGES ----------------------\n");
        out.push_str(&render_native_exchanges_json(interaction));
        out.push('\n');
    }
    out.push_str("====================== END REQUEST =========================\n");
    out
}

fn render_native_exchanges_json(request: &agent_core::ModelInteractionRequest) -> String {
    let exchanges = request
        .native_exchanges
        .iter()
        .map(|exchange| {
            serde_json::json!({
                "assistant_text": exchange.assistant_text,
                "calls": exchange.calls,
                "results": exchange.results.iter().map(|result| serde_json::json!({
                    "call_id": result.call_id,
                    "name": result.name,
                    "content": result.content,
                    "is_error": result.is_error,
                })).collect::<Vec<_>>(),
            })
        })
        .collect::<Vec<_>>();
    serde_json::to_string_pretty(&serde_json::json!({"native_exchanges": exchanges}))
        .unwrap_or_else(|error| format!("{{\"dump_error\":{error:?}}}"))
}

fn render_tool_schema_dump(session_id: &str, request: Option<&LlmRequestDumpEntry>) -> String {
    let mut out = String::new();
    out.push_str("TIMEM TOOL SCHEMA DUMP\n");
    out.push_str(&format!("session_id: {session_id}\n"));
    out.push_str("scope: latest_request_only\n");
    let Some(request) = request else {
        out.push_str("\n(no model requests recorded)\n");
        return out;
    };
    out.push('\n');
    out.push_str("============================================================\n");
    out.push_str(&format!("request_sequence: {}\n", request.sequence));
    out.push_str(&format!("worker_id: {}\n", request.worker_id));
    out.push_str(&format!("round: {}\n", request.round));
    let Some(interaction) = request
        .interaction_request
        .as_ref()
        .filter(|interaction| interaction.is_native())
    else {
        out.push_str("tool_call_mode: inline\n");
        out.push_str("\n(no native API tools field for the latest request)\n");
        return out;
    };
    let static_tool_count = interaction.static_tool_count.min(interaction.tools.len());
    out.push_str("tool_call_mode: native\n");
    out.push_str(&format!(
        "tool_definitions: {}\nstatic_builtin_tool_definitions: {}\ndynamic_tool_definitions: {}\n",
        interaction.tools.len(),
        static_tool_count,
        interaction.tools.len().saturating_sub(static_tool_count),
    ));
    out.push_str("-------------------------- TOOLS ---------------------------\n");
    let tools = interaction
        .tools
        .iter()
        .map(|tool| {
            serde_json::json!({
                "name": tool.name,
                "description": tool.description,
                "input_schema": tool.input_schema,
            })
        })
        .collect::<Vec<_>>();
    out.push_str(
        &serde_json::to_string_pretty(&tools)
            .unwrap_or_else(|error| format!("{{\"dump_error\":{error:?}}}")),
    );
    out.push('\n');
    out.push_str("====================== END TOOL SCHEMA =====================\n");
    out
}

fn render_llm_response_dump(
    session_id: &str,
    responses: &VecDeque<LlmResponseDumpEntry>,
) -> String {
    let mut out = String::new();
    out.push_str("TIMEM LLM RESPONSE DUMP\n");
    out.push_str(&format!("session_id: {session_id}\n"));
    out.push_str(&format!("retained_responses: {}\n", responses.len()));
    out.push_str("order: newest_to_oldest\n");
    if responses.is_empty() {
        out.push_str("\n(no model responses recorded)\n");
        return out;
    }
    for response in responses {
        out.push('\n');
        out.push_str("============================================================\n");
        out.push_str(&format!("response_sequence: {}\n", response.sequence));
        out.push_str(&format!(
            "request_sequence: {}\n",
            response
                .request_sequence
                .map_or_else(|| "unknown".to_string(), |value| value.to_string())
        ));
        out.push_str(&format!("worker_id: {}\n", response.worker_id));
        out.push_str(&format!("round: {}\n", response.round));
        out.push_str(&format!(
            "received_at: {}\n",
            format_timestamp_ms(response.received_at_ms)
        ));
        out.push_str(&format!("content_bytes: {}\n", response.content.len()));
        out.push_str(&format!("tool_call_count: {}\n", response.tool_calls.len()));
        out.push_str("------------------------------------------------------------\n");
        if response.content.is_empty() {
            out.push_str("(empty assistant text)\n");
        } else {
            out.push_str(&response.content);
            if !response.content.ends_with('\n') {
                out.push('\n');
            }
        }
        if !response.tool_calls.is_empty() {
            out.push_str("-------------------- NATIVE TOOL CALLS ---------------------\n");
            out.push_str(
                &serde_json::to_string_pretty(&response.tool_calls)
                    .unwrap_or_else(|error| format!("{{\"dump_error\":{error:?}}}")),
            );
            out.push('\n');
        }
        out.push_str("====================== END RESPONSE ========================\n");
    }
    out
}

fn render_statistics_html(session_id: &str, stats: &SessionDebug) -> String {
    let total_requests = stats
        .endpoints
        .values()
        .map(|item| item.requests)
        .sum::<u64>();
    let total_successes = stats
        .endpoints
        .values()
        .map(|item| item.successes)
        .sum::<u64>();
    let total_failures = stats
        .endpoints
        .values()
        .map(|item| item.failures)
        .sum::<u64>();
    let total_repairs = stats
        .endpoints
        .values()
        .flat_map(|item| item.repairs.values())
        .copied()
        .sum::<u64>();
    let mut out = String::new();
    out.push_str("<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">");
    out.push_str("<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">");
    out.push_str("<title>Timem Session Statistics</title><style>");
    out.push_str(STATISTICS_CSS);
    out.push_str("</style></head><body><main class=\"shell\">");
    out.push_str("<header class=\"page-head\"><div><p class=\"eyebrow\">TIMEM DEBUG TELEMETRY</p><h1>Session Statistics</h1><p class=\"subtitle\">");
    html_text(&mut out, session_id);
    out.push_str(
        "</p></div><div class=\"freshness\"><span class=\"live-dot\"></span>Auto-refresh · ",
    );
    html_text(&mut out, &format_timestamp_ms(stats.updated_at_ms));
    out.push_str("</div></header><section class=\"summary-grid\" aria-label=\"Session overview\">");
    summary_card(
        &mut out,
        "Endpoints",
        stats.endpoints.len().to_string(),
        "distinct model routes",
    );
    summary_card(
        &mut out,
        "Requests",
        total_requests.to_string(),
        "formal model requests",
    );
    summary_card(
        &mut out,
        "Successful",
        total_successes.to_string(),
        "completed API calls",
    );
    summary_card(
        &mut out,
        "Failed",
        total_failures.to_string(),
        "terminal or superseded",
    );
    summary_card(
        &mut out,
        "Repairs",
        total_repairs.to_string(),
        "protocol repair events",
    );
    out.push_str("</section><section class=\"panel overview\"><div class=\"section-head\"><div><p class=\"eyebrow\">OVERVIEW</p><h2>Endpoint matrix</h2></div><div class=\"time-range\">Started ");
    html_text(&mut out, &format_timestamp_ms(stats.started_at_ms));
    out.push_str("</div></div>");
    if stats.endpoints.is_empty() {
        out.push_str("<div class=\"empty-state\">No model endpoint has been negotiated yet. This page refreshes when the first interaction profile arrives.</div>");
    } else {
        out.push_str("<div class=\"table-wrap\"><table><thead><tr><th>Model</th><th>Gateway</th><th>Tool call</th><th>Requests</th><th>Successful</th><th>Failed</th><th>Success rate</th></tr></thead><tbody>");
        for (key, endpoint) in &stats.endpoints {
            out.push_str("<tr><td class=\"strong\">");
            html_text(&mut out, &key.model);
            out.push_str("</td><td class=\"mono gateway\">");
            html_text(&mut out, &key.gateway);
            out.push_str("</td><td>");
            protocol_badge(&mut out, &key.tool_call_mode);
            out.push_str("</td><td>");
            out.push_str(&endpoint.requests.to_string());
            out.push_str("</td><td class=\"success\">");
            out.push_str(&endpoint.successes.to_string());
            out.push_str("</td><td class=\"failure\">");
            out.push_str(&endpoint.failures.to_string());
            out.push_str("</td><td>");
            out.push_str(&success_rate(endpoint.successes, endpoint.requests));
            out.push_str("</td></tr>");
        }
        out.push_str("</tbody></table></div>");
    }
    out.push_str("</section>");
    if !stats.endpoints.is_empty() {
        out.push_str("<nav class=\"tabs\" role=\"tablist\" aria-label=\"Endpoint details\">");
        for (index, (key, _)) in stats.endpoints.iter().enumerate() {
            let endpoint_id = endpoint_dom_id(key);
            out.push_str(&format!("<button type=\"button\" role=\"tab\" data-endpoint-tab=\"{endpoint_id}\" aria-controls=\"{endpoint_id}\" aria-selected=\"{}\">", index == 0));
            html_text(&mut out, &key.model);
            out.push_str(" <span>");
            html_text(&mut out, &key.tool_call_mode);
            out.push_str("</span></button>");
        }
        out.push_str("</nav>");
        for (index, (key, endpoint)) in stats.endpoints.iter().enumerate() {
            render_endpoint_panel(&mut out, key, endpoint, index == 0);
        }
    }
    out.push_str("<footer>Request counts cover logical model-turn requests; retry attempts are folded into their final outcome. Capability-probe traffic is reported in endpoint negotiation details.</footer></main><script>");
    out.push_str(STATISTICS_JS);
    out.push_str(&format!(
        "setTimeout(()=>location.reload(),{STATISTICS_REFRESH_MS});"
    ));
    out.push_str("</script></body></html>");
    out
}

fn render_endpoint_panel(
    out: &mut String,
    key: &EndpointKey,
    endpoint: &EndpointDebug,
    active: bool,
) {
    let endpoint_id = endpoint_dom_id(key);
    out.push_str(&format!(
        "<section id=\"{endpoint_id}\" class=\"endpoint-panel{}\" role=\"tabpanel\">",
        if active { " active" } else { "" }
    ));
    out.push_str("<div class=\"endpoint-title\"><div><p class=\"eyebrow\">ENDPOINT DETAIL</p><h2>");
    html_text(out, &key.model);
    out.push_str("</h2><p class=\"mono\">");
    html_text(out, &key.gateway);
    out.push_str("</p></div>");
    protocol_badge(out, &key.tool_call_mode);
    out.push_str("</div><div class=\"endpoint-kpis\">");
    summary_card(
        out,
        "Requests",
        endpoint.requests.to_string(),
        "model turns",
    );
    summary_card(
        out,
        "Successful",
        endpoint.successes.to_string(),
        "API responses",
    );
    summary_card(
        out,
        "Failed",
        endpoint.failures.to_string(),
        "terminal failures",
    );
    summary_card(
        out,
        "Success rate",
        success_rate(endpoint.successes, endpoint.requests),
        "completed / requested",
    );
    out.push_str("</div>");
    if let Some(profile) = endpoint.profile.as_ref() {
        out.push_str("<dl class=\"profile-grid\">");
        profile_item(out, "API protocol", &profile.api_protocol);
        profile_item(out, "Prompt protocol", &profile.active_prompt_protocol);
        profile_item(out, "Requested mode", profile.requested_mode.label());
        profile_item(out, "Resolved mode", profile.resolved_mode.label());
        profile_item(
            out,
            "Probe source",
            &format!("{:?}", profile.source).to_ascii_lowercase(),
        );
        profile_item(
            out,
            "Probe latency",
            &profile
                .probe_latency_ms
                .map(|value| format!("{value} ms"))
                .unwrap_or_else(|| "n/a".to_string()),
        );
        profile_item(
            out,
            "Parallel supported",
            &profile.parallel_supported.to_string(),
        );
        profile_item(
            out,
            "Parallel enabled",
            &profile.parallel_enabled.to_string(),
        );
        profile_item(
            out,
            "Observed probe calls",
            &profile.observed_tool_calls.to_string(),
        );
        profile_item(out, "Negotiation result", &profile.reason);
        out.push_str("</dl>");
    }
    let action_total_ns = endpoint.action_cpu_ns.iter().copied().sum::<u64>();
    let llm_total_ms = endpoint.llm_latency_ms.iter().copied().sum::<u64>();
    let action_ms = endpoint
        .action_cpu_ns
        .iter()
        .map(|value| value / 1_000_000)
        .collect::<Vec<_>>();
    let action_counts = fixed_histogram(&action_ms, ACTION_BUCKET_MS, ACTION_LAST_BUCKET_MS);
    let latency_counts =
        fixed_histogram(&endpoint.llm_latency_ms, LLM_BUCKET_MS, LLM_LAST_BUCKET_MS);
    out.push_str("<div class=\"metric-pair\"><article class=\"panel metric-panel\"><div class=\"section-head compact\"><div><p class=\"eyebrow\">LOCAL EXECUTION</p><h3>Action on-CPU time</h3></div></div><div class=\"mini-kpis\">");
    mini_kpi(out, "Total", &format_duration_ns(action_total_ns));
    mini_kpi(
        out,
        "Mean",
        &format_mean_ns(action_total_ns, endpoint.action_cpu_ns.len()),
    );
    mini_kpi(
        out,
        "Max",
        &endpoint
            .action_cpu_ns
            .iter()
            .copied()
            .max()
            .map(format_duration_ns)
            .unwrap_or_else(|| "n/a".to_string()),
    );
    mini_kpi(
        out,
        "Unavailable",
        &endpoint.action_cpu_unavailable.to_string(),
    );
    out.push_str("</div>");
    render_horizontal_histogram(out, &action_counts, |bucket| {
        if bucket + 1 == action_counts.len() {
            "1s+".to_string()
        } else {
            format!(
                "{}–{} ms",
                bucket as u64 * ACTION_BUCKET_MS,
                (bucket as u64 + 1) * ACTION_BUCKET_MS
            )
        }
    });
    out.push_str("</article><article class=\"panel metric-panel\"><div class=\"section-head compact\"><div><p class=\"eyebrow\">REMOTE SERVICE</p><h3>LLM API latency</h3></div></div><div class=\"mini-kpis\">");
    mini_kpi(out, "Total", &format_duration_ms(llm_total_ms));
    mini_kpi(
        out,
        "Mean",
        &format_mean_ms(llm_total_ms, endpoint.llm_latency_ms.len()),
    );
    mini_kpi(
        out,
        "Max",
        &endpoint
            .llm_latency_ms
            .iter()
            .copied()
            .max()
            .map(format_duration_ms)
            .unwrap_or_else(|| "n/a".to_string()),
    );
    mini_kpi(out, "Samples", &endpoint.llm_latency_ms.len().to_string());
    out.push_str("</div>");
    render_horizontal_histogram(out, &latency_counts, |bucket| {
        if bucket + 1 == latency_counts.len() {
            "30s+".to_string()
        } else {
            format!(
                "{}–{} ms",
                bucket as u64 * LLM_BUCKET_MS,
                (bucket as u64 + 1) * LLM_BUCKET_MS
            )
        }
    });
    out.push_str("</article></div><div class=\"metric-pair lower\"><article class=\"panel metric-panel\"><div class=\"section-head compact\"><div><p class=\"eyebrow\">PROTOCOL QUALITY</p><h3>Repair error categories</h3></div></div>");
    render_named_bars(out, &endpoint.repairs);
    if endpoint.runtime_root_repair_help > 0 {
        out.push_str("<p class=\"footnote\">Runtime root repair help: ");
        out.push_str(&endpoint.runtime_root_repair_help.to_string());
        out.push_str("</p>");
    }
    out.push_str("</article><article class=\"panel metric-panel\"><div class=\"section-head compact\"><div><p class=\"eyebrow\">TOOL DENSITY</p><h3>Tools per response</h3></div></div>");
    render_horizontal_histogram(out, &endpoint.tools_per_response, |bucket| {
        if bucket == 10 {
            "10+".to_string()
        } else {
            bucket.to_string()
        }
    });
    out.push_str("</article></div></section>");
}

fn fixed_histogram(values: &[u64], width: u64, last_start: u64) -> Vec<u64> {
    let normal = (last_start / width) as usize;
    let mut counts = vec![0_u64; normal + 1];
    for value in values {
        let index = if *value >= last_start {
            normal
        } else {
            (*value / width) as usize
        };
        counts[index] = counts[index].saturating_add(1);
    }
    counts
}

fn summary_card(out: &mut String, label: &str, value: String, note: &str) {
    out.push_str("<article class=\"summary-card\"><p>");
    html_text(out, label);
    out.push_str("</p><strong>");
    html_text(out, &value);
    out.push_str("</strong><span>");
    html_text(out, note);
    out.push_str("</span></article>");
}

fn mini_kpi(out: &mut String, label: &str, value: &str) {
    out.push_str("<div><span>");
    html_text(out, label);
    out.push_str("</span><strong>");
    html_text(out, value);
    out.push_str("</strong></div>");
}

fn profile_item(out: &mut String, label: &str, value: &str) {
    out.push_str("<div><dt>");
    html_text(out, label);
    out.push_str("</dt><dd>");
    html_text(out, value);
    out.push_str("</dd></div>");
}

fn protocol_badge(out: &mut String, mode: &str) {
    let class = match mode {
        "native" => "native",
        "inline" => "inline",
        _ => "pending",
    };
    out.push_str("<span class=\"badge badge-");
    out.push_str(class);
    out.push_str("\">");
    html_text(out, mode);
    out.push_str("</span>");
}

fn success_rate(successes: u64, requests: u64) -> String {
    if requests == 0 {
        "n/a".to_string()
    } else {
        format!("{:.1}%", successes as f64 * 100.0 / requests as f64)
    }
}

fn endpoint_dom_id(key: &EndpointKey) -> String {
    // Stable FNV-1a keeps the selected tab attached to the same endpoint when
    // another endpoint is inserted earlier in the sorted overview.
    const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

    let mut hash = FNV_OFFSET_BASIS;
    for part in [&key.model, &key.gateway, &key.tool_call_mode] {
        for byte in part.as_bytes().iter().copied().chain([0]) {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(FNV_PRIME);
        }
    }
    format!("endpoint-{hash:016x}")
}

fn render_horizontal_histogram(out: &mut String, counts: &[u64], label: impl Fn(usize) -> String) {
    let nonzero = counts
        .iter()
        .copied()
        .enumerate()
        .filter(|(_, count)| *count > 0)
        .collect::<Vec<_>>();
    if nonzero.is_empty() {
        out.push_str("<div class=\"empty-chart\">No samples yet</div>");
        return;
    }
    let max = nonzero.iter().map(|(_, count)| *count).max().unwrap_or(1);
    out.push_str("<div class=\"bar-chart\">");
    for (index, count) in nonzero {
        let width = count.saturating_mul(100).div_ceil(max).max(2);
        out.push_str("<div class=\"bar-row\"><span class=\"bar-label\">");
        html_text(out, &label(index));
        out.push_str("</span><span class=\"bar-track\"><i style=\"width:");
        out.push_str(&width.to_string());
        out.push_str("%\"></i></span><strong>");
        out.push_str(&count.to_string());
        out.push_str("</strong></div>");
    }
    out.push_str("</div>");
}

fn render_named_bars(out: &mut String, rows: &BTreeMap<String, u64>) {
    if rows.is_empty() {
        out.push_str("<div class=\"empty-chart success-empty\">No protocol repairs</div>");
        return;
    }
    let max = rows.values().copied().max().unwrap_or(1);
    out.push_str("<div class=\"bar-chart\">");
    for (name, count) in rows {
        let width = count.saturating_mul(100).div_ceil(max).max(2);
        out.push_str("<div class=\"bar-row\"><span class=\"bar-label\">");
        html_text(out, name);
        out.push_str("</span><span class=\"bar-track repair\"><i style=\"width:");
        out.push_str(&width.to_string());
        out.push_str("%\"></i></span><strong>");
        out.push_str(&count.to_string());
        out.push_str("</strong></div>");
    }
    out.push_str("</div>");
}

fn html_text(out: &mut String, value: &str) {
    for character in value.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(character),
        }
    }
}

fn normalize_repair_category(issue: &str) -> String {
    let lower = issue.trim().to_ascii_lowercase();
    if lower.contains("truncated") || lower.contains("empty") {
        "empty_or_truncated_response"
    } else if lower.contains("response_root") || lower.contains("root") {
        "missing_or_invalid_response_root"
    } else if lower.contains("xml") || lower.contains("close_tag") {
        "invalid_xml"
    } else if lower.contains("parallel") {
        "invalid_parallel"
    } else if lower.contains("action") || lower.contains("tool") {
        "invalid_action"
    } else if lower.contains("finish_confirm") {
        "invalid_finish_confirm"
    } else if lower.contains("branch")
        || lower.contains("status")
        || lower.contains("final_answer")
        || lower.contains("next_actions")
    {
        "invalid_branch"
    } else {
        "unknown_protocol_error"
    }
    .to_string()
}

fn format_duration_ns(ns: u64) -> String {
    if ns >= 1_000_000_000 {
        format!("{:.3} s", ns as f64 / 1_000_000_000.0)
    } else {
        format!("{:.3} ms", ns as f64 / 1_000_000.0)
    }
}

fn format_mean_ns(total: u64, count: usize) -> String {
    if count == 0 {
        "n/a".to_string()
    } else {
        format_duration_ns(total / count as u64)
    }
}

fn format_duration_ms(ms: u64) -> String {
    if ms >= 1_000 {
        format!("{:.3} s", ms as f64 / 1_000.0)
    } else {
        format!("{ms} ms")
    }
}

fn format_mean_ms(total: u64, count: usize) -> String {
    if count == 0 {
        "n/a".to_string()
    } else {
        format_duration_ms(total / count as u64)
    }
}

fn safe_session_component(session_id: &str) -> Result<&str, String> {
    if !session_id.is_empty()
        && session_id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
    {
        Ok(session_id)
    } else {
        Err("invalid_debug_session_id".to_string())
    }
}

fn create_private_dir(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path).map_err(|error| format!("debug_dir_create_failed:{error}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|error| format!("debug_dir_permissions_failed:{error}"))?;
    }
    Ok(())
}

fn atomic_private_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let suffix = now_ms();
    let temporary = path.with_extension(format!("tmp-{}-{suffix}", std::process::id()));
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&temporary)
        .map_err(|error| format!("debug_file_open_failed:{error}"))?;
    if let Err(error) = file.write_all(bytes).and_then(|_| file.sync_all()) {
        let _ = fs::remove_file(&temporary);
        return Err(format!("debug_file_write_failed:{error}"));
    }
    if let Err(error) = fs::rename(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(format!("debug_file_replace_failed:{error}"));
    }
    Ok(())
}

fn format_timestamp_ms(timestamp_ms: u128) -> String {
    use chrono::{DateTime, Local, SecondsFormat, Utc};

    let Ok(timestamp_ms) = i64::try_from(timestamp_ms) else {
        return "invalid timestamp".to_string();
    };
    DateTime::<Utc>::from_timestamp_millis(timestamp_ms)
        .map(|timestamp| {
            timestamp
                .with_timezone(&Local)
                .to_rfc3339_opts(SecondsFormat::Millis, false)
        })
        .unwrap_or_else(|| "invalid timestamp".to_string())
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

const STATISTICS_CSS: &str = r#"
:root{color-scheme:light dark;--bg:#f3f6fa;--panel:#fff;--panel-2:#f8fafc;--text:#172033;--muted:#667085;--line:#dfe5ec;--blue:#2563eb;--green:#16845b;--red:#c33b4a;--shadow:0 10px 30px rgba(27,39,58,.07)}
*{box-sizing:border-box}body{margin:0;background:var(--bg);color:var(--text);font:14px/1.5 Inter,ui-sans-serif,-apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif}.shell{width:min(1480px,calc(100% - 40px));margin:0 auto;padding:34px 0 52px}.page-head,.section-head,.endpoint-title{display:flex;align-items:flex-start;justify-content:space-between;gap:24px}.page-head{margin-bottom:24px}h1,h2,h3,p{margin:0}h1{font-size:30px;letter-spacing:-.03em}h2{font-size:20px}h3{font-size:17px}.subtitle,.mono{font-family:ui-monospace,SFMono-Regular,Menlo,monospace}.subtitle{margin-top:5px;color:var(--muted)}.eyebrow{color:var(--blue);font-size:11px;font-weight:750;letter-spacing:.13em;margin-bottom:6px}.freshness{display:flex;align-items:center;gap:8px;color:var(--muted);font-size:12px;padding-top:8px}.live-dot{width:8px;height:8px;border-radius:50%;background:#20b26b;box-shadow:0 0 0 4px rgba(32,178,107,.12)}.summary-grid,.endpoint-kpis{display:grid;grid-template-columns:repeat(5,minmax(0,1fr));gap:12px;margin-bottom:18px}.summary-card,.panel{background:var(--panel);border:1px solid var(--line);border-radius:12px;box-shadow:var(--shadow)}.summary-card{padding:16px 18px}.summary-card p,.summary-card span{color:var(--muted);font-size:12px}.summary-card strong{display:block;font-size:24px;line-height:1.2;margin:5px 0 2px}.panel{padding:20px}.overview{margin-bottom:18px}.section-head{align-items:center;margin-bottom:16px}.section-head.compact{margin-bottom:14px}.time-range{color:var(--muted);font-size:12px}.table-wrap{overflow:auto;border:1px solid var(--line);border-radius:9px}table{width:100%;border-collapse:collapse}th,td{padding:11px 13px;border-bottom:1px solid var(--line);text-align:right;white-space:nowrap}th:first-child,th:nth-child(2),th:nth-child(3),td:first-child,td:nth-child(2),td:nth-child(3){text-align:left}th{background:var(--panel-2);color:var(--muted);font-size:11px;text-transform:uppercase;letter-spacing:.06em}tbody tr:last-child td{border-bottom:0}.strong{font-weight:700}.gateway{max-width:440px;overflow:hidden;text-overflow:ellipsis}.success{color:var(--green);font-weight:700}.failure{color:var(--red);font-weight:700}.badge{display:inline-flex;padding:3px 9px;border-radius:999px;font-size:11px;font-weight:750;text-transform:uppercase;letter-spacing:.04em}.badge-native{background:#e7f7ef;color:#08764d}.badge-inline{background:#fff2d8;color:#955500}.badge-pending{background:#eef1f5;color:#667085}.tabs{display:flex;gap:8px;overflow:auto;padding:2px 0 12px}.tabs button{appearance:none;border:1px solid var(--line);background:var(--panel);color:var(--muted);border-radius:9px;padding:9px 13px;font:inherit;font-weight:650;cursor:pointer;white-space:nowrap}.tabs button span{font-size:10px;text-transform:uppercase;margin-left:5px}.tabs button[aria-selected=true]{background:var(--text);border-color:var(--text);color:var(--panel)}.endpoint-panel{display:none}.endpoint-panel.active{display:block}.endpoint-title{background:var(--panel);border:1px solid var(--line);border-radius:12px;padding:20px;margin-bottom:12px}.endpoint-title .mono{color:var(--muted);margin-top:4px;overflow-wrap:anywhere}.endpoint-kpis{grid-template-columns:repeat(4,minmax(0,1fr))}.profile-grid{display:grid;grid-template-columns:repeat(5,minmax(0,1fr));gap:1px;background:var(--line);border:1px solid var(--line);border-radius:11px;overflow:hidden;margin:0 0 12px}.profile-grid>div{background:var(--panel);padding:12px 14px;min-width:0}.profile-grid dt{color:var(--muted);font-size:11px;margin-bottom:3px}.profile-grid dd{margin:0;font-weight:650;overflow-wrap:anywhere}.metric-pair{display:grid;grid-template-columns:1fr 1fr;gap:12px}.metric-pair.lower{margin-top:12px}.metric-panel{min-width:0}.mini-kpis{display:grid;grid-template-columns:repeat(4,minmax(0,1fr));gap:8px;margin-bottom:16px}.mini-kpis div{background:var(--panel-2);border-radius:8px;padding:9px 10px}.mini-kpis span{display:block;color:var(--muted);font-size:10px;text-transform:uppercase;letter-spacing:.04em}.mini-kpis strong{display:block;margin-top:3px}.bar-chart{display:grid;gap:8px}.bar-row{display:grid;grid-template-columns:minmax(72px,130px) 1fr 38px;gap:9px;align-items:center}.bar-label{color:var(--muted);font-size:11px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}.bar-track{height:9px;background:var(--panel-2);border-radius:99px;overflow:hidden}.bar-track i{display:block;height:100%;border-radius:inherit;background:linear-gradient(90deg,#4b83ec,#2563eb)}.bar-track.repair i{background:linear-gradient(90deg,#f09a4b,#d45b37)}.bar-row strong{text-align:right;font-size:11px}.empty-chart,.empty-state{padding:24px;text-align:center;color:var(--muted);background:var(--panel-2);border-radius:9px}.success-empty{color:var(--green)}.footnote,footer{color:var(--muted);font-size:11px}.footnote{margin-top:12px}footer{text-align:center;padding:24px 10px 0}
@media(max-width:980px){.summary-grid{grid-template-columns:repeat(3,1fr)}.profile-grid{grid-template-columns:repeat(3,1fr)}.metric-pair{grid-template-columns:1fr}}
@media(max-width:640px){.shell{width:min(100% - 22px,1480px);padding-top:20px}.page-head{display:block}.freshness{margin-top:10px}.summary-grid,.endpoint-kpis{grid-template-columns:repeat(2,1fr)}.profile-grid{grid-template-columns:repeat(2,1fr)}.mini-kpis{grid-template-columns:repeat(2,1fr)}.summary-card strong{font-size:20px}.panel{padding:15px}}
@media(prefers-color-scheme:dark){:root{--bg:#0d1118;--panel:#151b24;--panel-2:#10161f;--text:#edf2f8;--muted:#99a6b7;--line:#2a3442;--blue:#72a4ff;--shadow:none}.badge-native{background:#11392b;color:#7ee2b8}.badge-inline{background:#402f10;color:#f0c36d}}
"#;

const STATISTICS_JS: &str = r#"
(()=>{const key=`timem-statistics-tab:${location.pathname}`;const buttons=[...document.querySelectorAll('[data-endpoint-tab]')];const panels=[...document.querySelectorAll('.endpoint-panel')];const load=()=>{try{return sessionStorage.getItem(key)}catch{return null}};const save=id=>{try{sessionStorage.setItem(key,id)}catch{}};function select(id){if(!panels.some(p=>p.id===id))id=panels[0]?.id;buttons.forEach(b=>b.setAttribute('aria-selected',String(b.dataset.endpointTab===id)));panels.forEach(p=>p.classList.toggle('active',p.id===id));if(id)save(id)}buttons.forEach(b=>b.addEventListener('click',()=>select(b.dataset.endpointTab)));select(load()||panels[0]?.id)})();
"#;

#[cfg(test)]
mod tests {
    use super::*;

    static DEBUG_STORE_TEST_LOCK: Mutex<()> = Mutex::new(());

    fn debug_store_test_guard() -> std::sync::MutexGuard<'static, ()> {
        DEBUG_STORE_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn profile(
        model: &str,
        gateway: &str,
        mode: agent_core::ToolCallMode,
    ) -> agent_core::InteractionProfile {
        agent_core::InteractionProfile {
            api_protocol: "openai_compatible".to_string(),
            model: model.to_string(),
            gateway: gateway.to_string(),
            requested_mode: agent_core::ToolCallMode::Auto,
            resolved_mode: mode,
            active_prompt_protocol: if mode == agent_core::ToolCallMode::Native {
                "json"
            } else {
                "xml"
            }
            .to_string(),
            parallel_supported: mode == agent_core::ToolCallMode::Native,
            parallel_enabled: mode == agent_core::ToolCallMode::Native,
            source: agent_core::CapabilityProbeSource::Probe,
            reason: "test_probe_result".to_string(),
            probe_latency_ms: Some(42),
            observed_tool_calls: if mode == agent_core::ToolCallMode::Native {
                2
            } else {
                0
            },
        }
    }

    #[test]
    fn timestamps_are_human_readable_rfc3339_with_milliseconds() {
        let formatted = format_timestamp_ms(1_787_066_216_855);
        let parsed = chrono::DateTime::parse_from_rfc3339(&formatted).unwrap();
        assert_eq!(parsed.timestamp_millis(), 1_787_066_216_855);
        assert!(formatted.contains(".855"));
    }

    #[test]
    fn histograms_keep_fixed_last_bucket() {
        assert_eq!(fixed_histogram(&[19, 20, 999, 1_000], 20, 1_000)[50], 1);
        assert_eq!(
            fixed_histogram(&[199, 200, 29_999, 30_000], 200, 30_000)[150],
            1
        );
    }

    #[test]
    fn debug_store_initializes_private_artifacts_and_rejects_unsafe_session_ids() {
        let _guard = debug_store_test_guard();
        let store = DebugStore::create().unwrap();
        let root = store.root().to_path_buf();
        let session_dir = store.session_dir("safe-session_1").unwrap();

        let prompt = fs::read_to_string(session_dir.join("llm_prompt.dump")).unwrap();
        let tool_schema = fs::read_to_string(session_dir.join("tool_schema.dump")).unwrap();
        let response = fs::read_to_string(session_dir.join("llm_response.dump")).unwrap();
        let statistics = fs::read_to_string(session_dir.join("statistics.html")).unwrap();
        assert!(prompt.contains("(no model requests recorded)"));
        assert!(tool_schema.contains("(no model requests recorded)"));
        assert!(response.contains("(no model responses recorded)"));
        assert!(statistics.contains("No model endpoint has been negotiated yet"));
        assert_eq!(fs::read_dir(&session_dir).unwrap().count(), 4);

        for unsafe_id in ["", ".", "../escape", "nested/session", "session space"] {
            assert_eq!(
                store.session_dir(unsafe_id).unwrap_err(),
                "invalid_debug_session_id"
            );
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&root).unwrap().permissions().mode() & 0o777,
                0o700
            );
            assert_eq!(
                fs::metadata(&session_dir).unwrap().permissions().mode() & 0o777,
                0o700
            );
            for name in [
                "statistics.html",
                "llm_prompt.dump",
                "llm_response.dump",
                "tool_schema.dump",
            ] {
                assert_eq!(
                    fs::metadata(session_dir.join(name))
                        .unwrap()
                        .permissions()
                        .mode()
                        & 0o777,
                    0o600
                );
            }
        }
        store.cleanup().unwrap();
        assert!(!root.exists());
    }

    #[test]
    fn debug_statistics_accounts_for_every_metric_and_terminal_outcome() {
        let _guard = debug_store_test_guard();
        let store = DebugStore::create().unwrap();
        let session = "session_complete_metrics";
        let native_profile = profile(
            "qwen-plus",
            "https://gateway.example/v1",
            agent_core::ToolCallMode::Native,
        );
        store
            .record_interaction_profile(session, "worker_0", &native_profile)
            .unwrap();
        store
            .record_prompt(session, "worker_0", 1, "successful", None)
            .unwrap();
        store
            .record_llm_latency(session, "worker_0", Duration::from_millis(450))
            .unwrap();
        store
            .record_tools_per_response(session, "worker_0", 2)
            .unwrap();
        store
            .record_action_cpu(session, "worker_0", Some(Duration::from_millis(30)))
            .unwrap();
        store.record_action_cpu(session, "worker_0", None).unwrap();
        store
            .record_repair(session, "worker_0", "xml_response_root_missing")
            .unwrap();
        store
            .record_repair(session, "worker_0", "unknown-new-issue")
            .unwrap();
        store
            .record_runtime_root_repair_help(session, "worker_0")
            .unwrap();
        store
            .record_prompt(session, "worker_0", 2, "failed", None)
            .unwrap();
        store.record_model_failure(session, "worker_0").unwrap();
        store
            .record_llm_latency(session, "worker_0", Duration::from_millis(999))
            .unwrap();
        store.record_model_failure(session, "worker_0").unwrap();

        {
            let sessions = store.sessions.lock().unwrap();
            let endpoint =
                &sessions[session].endpoints[&EndpointKey::from_profile(&native_profile)];
            assert_eq!(
                (endpoint.requests, endpoint.successes, endpoint.failures),
                (2, 1, 1)
            );
            assert_eq!(endpoint.llm_latency_ms, vec![450]);
            assert_eq!(endpoint.action_cpu_ns, vec![30_000_000]);
            assert_eq!(endpoint.action_cpu_unavailable, 1);
            assert_eq!(endpoint.tools_per_response[2], 1);
            assert_eq!(endpoint.repairs["missing_or_invalid_response_root"], 1);
            assert_eq!(endpoint.repairs["unknown_protocol_error"], 1);
            assert_eq!(endpoint.runtime_root_repair_help, 1);
        }
        let html = fs::read_to_string(store.root().join(session).join("statistics.html")).unwrap();
        for marker in [
            "qwen-plus",
            "50.0%",
            "Action on-CPU time",
            "LLM API latency",
            "<span>Unavailable</span><strong>1</strong>",
            "missing_or_invalid_response_root",
            "unknown_protocol_error",
            "Runtime root repair help: 1",
        ] {
            assert!(html.contains(marker), "missing {marker:?} in {html}");
        }
        store.cleanup().unwrap();
    }

    #[test]
    fn concurrent_debug_updates_are_atomic_and_worker_isolated() {
        let _guard = debug_store_test_guard();
        let store = std::sync::Arc::new(DebugStore::create().unwrap());
        let session = "session_concurrent";
        const WORKERS: usize = 4;
        const REQUESTS_PER_WORKER: u32 = 3;
        let start = std::sync::Arc::new(std::sync::Barrier::new(WORKERS));
        let threads = (0..WORKERS)
            .map(|worker| {
                let store = std::sync::Arc::clone(&store);
                let start = std::sync::Arc::clone(&start);
                std::thread::spawn(move || {
                    let worker_id = format!("worker_{worker}");
                    store
                        .record_interaction_profile(
                            session,
                            &worker_id,
                            &profile(
                                "shared-model",
                                "https://shared.example/v1",
                                agent_core::ToolCallMode::Native,
                            ),
                        )
                        .unwrap();
                    start.wait();
                    for round in 1..=REQUESTS_PER_WORKER {
                        store
                            .record_prompt(session, &worker_id, round, &worker_id, None)
                            .unwrap();
                        store
                            .record_llm_latency(
                                session,
                                &worker_id,
                                Duration::from_millis(u64::from(round)),
                            )
                            .unwrap();
                        store
                            .record_llm_response(session, &worker_id, round, "ok", &[])
                            .unwrap();
                    }
                })
            })
            .collect::<Vec<_>>();
        for thread in threads {
            thread.join().unwrap();
        }

        let session_dir = store.root().join(session);
        let html = fs::read_to_string(session_dir.join("statistics.html")).unwrap();
        let prompt = fs::read_to_string(session_dir.join("llm_prompt.dump")).unwrap();
        let response = fs::read_to_string(session_dir.join("llm_response.dump")).unwrap();
        assert!(html.ends_with("</html>"));
        assert!(html.contains("<strong>12</strong>"));
        assert!(prompt.contains("scope: latest_request_only"));
        assert_eq!(prompt.matches("request_sequence:").count(), 1);
        assert!(response.contains("retained_responses: 10"));
        let artifact_names = fs::read_dir(&session_dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            artifact_names,
            [
                "llm_prompt.dump",
                "llm_response.dump",
                "statistics.html",
                "tool_schema.dump",
            ]
            .into_iter()
            .map(std::ffi::OsString::from)
            .collect()
        );
        store.cleanup().unwrap();
    }

    #[test]
    fn html_groups_multiple_endpoints_and_escapes_dynamic_values() {
        let mut stats = SessionDebug {
            started_at_ms: 1,
            updated_at_ms: 2,
            ..SessionDebug::default()
        };
        let first_profile = profile(
            "qwen-plus<script>",
            "https://one.example/v1?a=<x>",
            agent_core::ToolCallMode::Native,
        );
        let second_profile = profile(
            "other-model",
            "https://two.example/v1",
            agent_core::ToolCallMode::Inline,
        );
        let first_key = EndpointKey::from_profile(&first_profile);
        let second_key = EndpointKey::from_profile(&second_profile);
        stats.endpoints.insert(
            first_key,
            EndpointDebug {
                profile: Some(first_profile),
                requests: 3,
                successes: 2,
                failures: 1,
                action_cpu_ns: vec![1_000_000, 2_000_000],
                llm_latency_ms: vec![100, 250],
                repairs: BTreeMap::from([("invalid_action<script>".to_string(), 2)]),
                ..EndpointDebug::default()
            },
        );
        stats.endpoints.insert(
            second_key,
            EndpointDebug {
                profile: Some(second_profile),
                requests: 1,
                successes: 1,
                ..EndpointDebug::default()
            },
        );
        let html = render_statistics_html("session_native", &stats);
        assert!(html.starts_with("<!doctype html>"));
        assert!(html.contains("Endpoint matrix"));
        assert_eq!(html.matches("role=\"tabpanel\"").count(), 2);
        assert!(html.contains("qwen-plus&lt;script&gt;"));
        assert!(html.contains("invalid_action&lt;script&gt;"));
        assert!(!html.contains("qwen-plus<script>"));
        assert!(html.contains("Action on-CPU time"));
        assert!(html.contains("LLM API latency"));
        assert!(html.contains("Repair error categories"));
    }

    #[test]
    fn store_refreshes_profile_and_request_outcomes_per_worker_endpoint() {
        let _guard = debug_store_test_guard();
        let store = DebugStore::create().unwrap();
        let session = "session_metrics";
        store
            .record_interaction_profile(
                session,
                "worker_0",
                &profile(
                    "qwen",
                    "https://one.example/v1",
                    agent_core::ToolCallMode::Native,
                ),
            )
            .unwrap();
        store
            .record_prompt(session, "worker_0", 1, "prompt", None)
            .unwrap();
        store
            .record_llm_latency(session, "worker_0", Duration::from_millis(125))
            .unwrap();
        store
            .record_interaction_profile(
                session,
                "worker_0",
                &profile(
                    "gpt",
                    "https://two.example/v1",
                    agent_core::ToolCallMode::Inline,
                ),
            )
            .unwrap();
        store
            .record_prompt(session, "worker_0", 2, "prompt 2", None)
            .unwrap();
        store.record_model_failure(session, "worker_0").unwrap();
        let html = fs::read_to_string(store.root().join(session).join("statistics.html")).unwrap();
        assert!(html.contains("qwen"));
        assert!(html.contains("gpt"));
        assert_eq!(html.matches("role=\"tabpanel\"").count(), 2);
        assert!(!html.contains("tool_call: pending"));
        store.cleanup().unwrap();
    }

    #[test]
    fn prompt_dump_keeps_only_latest_request_and_statistics_is_private_html() {
        let _guard = debug_store_test_guard();
        let store = DebugStore::create().unwrap();
        let root = store.root().to_path_buf();
        let session_dir = store.session_dir("session_test").unwrap();
        store
            .record_interaction_profile(
                "session_test",
                "worker_0",
                &profile(
                    "qwen",
                    "https://example.test/v1",
                    agent_core::ToolCallMode::Native,
                ),
            )
            .unwrap();
        store
            .record_prompt("session_test", "worker_0", 1, "first prompt", None)
            .unwrap();
        let native_request = agent_core::ModelInteractionRequest {
            rendered_prompt: "second prompt".to_string(),
            static_tool_count: 1,
            tools: vec![
                agent_core::ToolDefinition {
                    name: "self_tool".to_string(),
                    description: "Inspect runtime state".to_string(),
                    input_schema: serde_json::json!({
                        "type": "object",
                        "properties": {"type": {"enum": ["cwd"]}},
                        "additionalProperties": false
                    }),
                },
                agent_core::ToolDefinition {
                    name: "mcp.demo.echo".to_string(),
                    description: "Dynamic MCP echo".to_string(),
                    input_schema: serde_json::json!({"type": "object"}),
                },
            ],
            native_exchanges: vec![agent_core::NativeExchange {
                assistant_text: "checking".to_string(),
                calls: vec![agent_core::NativeToolCall {
                    id: "call_previous".to_string(),
                    name: "self_tool".to_string(),
                    arguments: serde_json::json!({"type": "cwd"}),
                    raw_arguments: r#"{"type":"cwd"}"#.to_string(),
                }],
                results: vec![agent_core::NativeToolResult {
                    call_id: "call_previous".to_string(),
                    name: "self_tool".to_string(),
                    content: "CWD: /workspace".to_string(),
                    is_error: false,
                }],
            }],
            resolved_mode: agent_core::ToolCallMode::Native,
            parallel_tool_calls: true,
            tool_choice: agent_core::NativeToolChoice::Auto,
        };
        store
            .record_prompt(
                "session_test",
                "worker_0",
                2,
                "second prompt",
                Some(&native_request),
            )
            .unwrap();
        let dump = fs::read_to_string(session_dir.join("llm_prompt.dump")).unwrap();
        let tool_schema = fs::read_to_string(session_dir.join("tool_schema.dump")).unwrap();
        assert!(dump.contains("scope: latest_request_only"));
        assert!(dump.contains("request_sequence: 2"));
        assert!(dump.contains("second prompt"));
        assert!(!dump.contains("first prompt"));
        assert_eq!(dump.matches("request_sequence:").count(), 1);
        assert!(dump.contains("tool_call_mode: native"));
        assert!(dump.contains("static_builtin_tool_definitions: 1"));
        assert!(dump.contains("dynamic_tool_definitions: 1"));
        assert!(!dump.contains("mcp.demo.echo"));
        assert!(dump.contains("call_previous"));
        assert!(dump.contains("CWD: /workspace"));
        assert!(tool_schema.contains("scope: latest_request_only"));
        assert!(tool_schema.contains("request_sequence: 2"));
        assert!(tool_schema.contains("tool_call_mode: native"));
        assert!(tool_schema.contains("tool_definitions: 2"));
        assert!(tool_schema.contains("mcp.demo.echo"));
        assert!(tool_schema.contains("Dynamic MCP echo"));
        assert!(tool_schema.contains("\"additionalProperties\": false"));
        assert!(tool_schema.contains("\"enum\": ["));
        assert!(!tool_schema.contains("call_previous"));
        assert!(session_dir.join("statistics.html").is_file());
        assert!(!session_dir.join("statistics.md").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(session_dir.join("statistics.html"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        store.cleanup().unwrap();
        assert!(!root.exists());
    }

    #[test]
    fn inline_prompt_dump_records_mode_without_inventing_native_payload() {
        let _guard = debug_store_test_guard();
        let store = DebugStore::create().unwrap();
        let session_dir = store.session_dir("session_inline_dump").unwrap();
        let request = agent_core::ModelInteractionRequest::inline("inline prompt");
        store
            .record_prompt(
                "session_inline_dump",
                "worker_inline",
                1,
                "inline prompt",
                Some(&request),
            )
            .unwrap();

        let dump = fs::read_to_string(session_dir.join("llm_prompt.dump")).unwrap();
        let tool_schema = fs::read_to_string(session_dir.join("tool_schema.dump")).unwrap();
        assert!(dump.contains("tool_call_mode: inline"));
        assert!(dump.contains("inline prompt"));
        assert!(!dump.contains("NATIVE STRUCTURED INPUT"));
        assert!(tool_schema.contains("tool_call_mode: inline"));
        assert!(tool_schema.contains("no native API tools field"));
        store.cleanup().unwrap();
    }

    #[test]
    fn llm_response_dump_keeps_newest_ten_in_reverse_chronological_order() {
        let _guard = debug_store_test_guard();
        let store = DebugStore::create().unwrap();
        let session_dir = store.session_dir("session_responses").unwrap();
        for index in 1..=12 {
            let tool_calls = if index == 12 {
                vec![agent_core::NativeToolCall {
                    id: "call_latest".to_string(),
                    name: "self_tool".to_string(),
                    arguments: serde_json::json!({"type": "cwd"}),
                    raw_arguments: r#"{"type":"cwd"}"#.to_string(),
                }]
            } else {
                Vec::new()
            };
            store
                .record_llm_response(
                    "session_responses",
                    "worker_0",
                    index,
                    &format!("response-{index}"),
                    &tool_calls,
                )
                .unwrap();
        }
        let dump = fs::read_to_string(session_dir.join("llm_response.dump")).unwrap();
        assert!(dump.contains("retained_responses: 10"));
        assert!(dump.find("response-12").unwrap() < dump.find("response-11").unwrap());
        assert!(!dump.contains("response-1\n"));
        assert!(!dump.contains("response-2\n"));
        assert!(dump.contains("tool_call_count: 1"));
        assert!(dump.contains("NATIVE TOOL CALLS"));
        assert!(dump.contains("call_latest"));
        assert!(dump.contains("raw_arguments"));
        store.cleanup().unwrap();
    }
}
