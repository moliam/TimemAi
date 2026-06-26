use rusqlite::{params_from_iter, types::ValueRef, Connection};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::ffi::{CStr, CString};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::os::raw::c_char;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

static ID_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CoreProfile {
    pub name: String,
    pub provider: String,
    pub model: String,
}
impl CoreProfile {
    pub fn label(&self) -> String {
        format!("{}:{}:{}", self.name, self.provider, self.model)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UsageStats {
    pub llm_calls: u32,
    pub tool_calls: u32,
    pub mem_reads: u32,
    pub mem_writes: u32,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
    pub cached_tokens: u32,
    pub shrunk_tokens: u32,
}
impl UsageStats {
    pub fn zero() -> Self {
        Self {
            llm_calls: 0,
            tool_calls: 0,
            mem_reads: 0,
            mem_writes: 0,
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: 0,
            cached_tokens: 0,
            shrunk_tokens: 0,
        }
    }
    pub fn add(&mut self, other: &UsageStats) {
        self.llm_calls += other.llm_calls;
        self.tool_calls += other.tool_calls;
        self.mem_reads += other.mem_reads;
        self.mem_writes += other.mem_writes;
        self.prompt_tokens += other.prompt_tokens;
        self.completion_tokens += other.completion_tokens;
        self.total_tokens += other.total_tokens;
        self.cached_tokens += other.cached_tokens;
        self.shrunk_tokens += other.shrunk_tokens;
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LlmResponse {
    pub content: String,
    pub model_name: String,
    pub usage: UsageStats,
    pub truncated: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TurnFinal {
    pub response_to_user: String,
    pub stats: UsageStats,
    pub profile_label: String,
    pub repair_issue: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ApprovalRequest {
    pub approval_id: String,
    pub action: String,
    pub command: String,
    pub read_back_command: String,
    pub reason: String,
    pub risk: String,
    pub intent: String,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum BashApprovalMode {
    Ask,
    Approve,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum CoreStep {
    NeedModel {
        prompt: String,
        rounds_remaining: u32,
    },
    NeedsUserApproval {
        request: ApprovalRequest,
    },
    RoundLimitReached {
        max_rounds: u32,
    },
    Final(TurnFinal),
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PromptDelta {
    pub delta_id: String,
    pub time_ms: i64,
    #[serde(default)]
    pub durable_ctx_score: Option<u8>,
    slices: Vec<PromptSlice>,
    #[serde(default)]
    pub hidden_slice_ids: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct PromptSlice {
    delta_id: String,
    slice_id: String,
    durable_ctx_score: Option<u8>,
    prompt_type: String,
    time_ms: i64,
    text: String,
    slice_index: usize,
    slice_count: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryRecord {
    pub id: String,
    pub created_at_ms: i64,
    pub content: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScratchNoteRecord {
    pub id: String,
    pub created_at_ms: i64,
    pub content: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatHistoryRecord {
    pub session: String,
    pub turn_id: String,
    pub started_at_ms: i64,
    pub user_input: String,
    pub assistant_output: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ShellJobRecord {
    pub id: String,
    pub created_at_ms: i64,
    pub pid: u32,
    pub command: String,
    pub output_file: String,
    pub status_file: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedAction {
    action: String,
    intent: String,
    query: String,
    content: String,
    sql: String,
    params: Vec<String>,
    operation: String,
    id: String,
    command: String,
    read_back_command: String,
    large_readback_opt_in: bool,
    background: bool,
    job_id: String,
    delta_ids: Vec<String>,
    slice_ids: Vec<String>,
    timeout_ms: u64,
    limit: usize,
    after_ms: Option<i64>,
    before_ms: Option<i64>,
}
impl ParsedAction {
    fn audit_input(&self) -> Value {
        match self.action.as_str() {
            "run_bash" => json!({
                "command": self.command,
                "read_back_command": self.read_back_command,
                "large_readback_opt_in": self.large_readback_opt_in,
                "background": self.background,
                "timeout_ms": self.timeout_ms,
            }),
            "shell_job_status" => json!({
                "job_id": self.job_id,
                "timeout_ms": self.timeout_ms,
            }),
            "memory_sql_query" | "sql_read" => json!({
                "sql": self.sql,
                "params": self.params,
                "limit": self.limit,
            }),
            "memory_update" => json!({
                "operation": self.operation,
                "id": self.id,
                "content": self.content,
            }),
            "memory_write" | "write_memory" | "scratch_write" => json!({
                "content": self.content,
                "query": self.query,
            }),
            "scratch_delete" => json!({
                "id": self.id,
            }),
            "prompt_shrink" => json!({
                "delta_ids": self.delta_ids,
                "slice_ids": self.slice_ids,
            }),
            "chat_history_delete" => json!({
                "id": self.id,
                "query": self.query,
                "limit": self.limit,
                "after_ms": self.after_ms,
                "before_ms": self.before_ms,
            }),
            _ => json!({
                "query": self.query,
                "id": self.id,
                "limit": self.limit,
                "after_ms": self.after_ms,
                "before_ms": self.before_ms,
            }),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedEnvelope {
    response_to_user: String,
    thought: String,
    next_actions: Vec<ParsedAction>,
    memory_candidates: Vec<String>,
    delta_scores: Vec<DeltaScore>,
    repair_issue: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DeltaScore {
    delta_id: Option<String>,
    durable_ctx_score: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingApproval {
    request: ApprovalRequest,
    command: String,
    read_back_command: String,
    large_readback_opt_in: bool,
    background: bool,
    timeout_ms: u64,
    intent: String,
}

const PROMPT_SLICE_TEXT_LIMIT: usize = 12_000;
const DEFAULT_ROUND_BUDGET: u32 = 20;

#[derive(Debug, Clone, PartialEq, Eq)]
enum ActionExecution {
    Completed(String),
    NeedsApproval(PendingApproval),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ActionAuditDocument {
    version: u32,
    turns: Vec<ActionAuditTurn>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ActionAuditTurn {
    turn_id: String,
    started_at_ms: i64,
    user_question: String,
    interactions: Vec<ActionAuditInteraction>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ActionAuditInteraction {
    round: u32,
    actions: Vec<ActionAuditEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ActionAuditEntry {
    time_ms: i64,
    round: u32,
    action: String,
    intent: String,
    status: String,
    input: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result_summary: Option<String>,
}

#[derive(Debug, Clone)]
struct FileActionAuditStore {
    file: PathBuf,
}

impl FileActionAuditStore {
    fn new(memory_dir: &Path) -> Self {
        let space_dir = if memory_dir.file_name().and_then(|name| name.to_str()) == Some("memory") {
            memory_dir.parent().unwrap_or(memory_dir)
        } else {
            memory_dir
        };
        let file = space_dir.join("audit").join("action_audit.json");
        Self { file }
    }

    fn begin_turn(&self, turn_id: &str, started_at_ms: i64, user_question: &str) {
        let mut doc = self.read_doc();
        if doc.turns.iter().any(|turn| turn.turn_id == turn_id) {
            return;
        }
        doc.turns.push(ActionAuditTurn {
            turn_id: turn_id.to_string(),
            started_at_ms,
            user_question: user_question.to_string(),
            interactions: Vec::new(),
        });
        self.write_doc(&doc);
    }

    fn record_action(&self, entry: ActionAuditEntry, turn_id: &str, user_question: &str) {
        let mut doc = self.read_doc();
        let turn_index =
            if let Some(index) = doc.turns.iter().position(|turn| turn.turn_id == turn_id) {
                index
            } else {
                doc.turns.push(ActionAuditTurn {
                    turn_id: turn_id.to_string(),
                    started_at_ms: now_ms(),
                    user_question: user_question.to_string(),
                    interactions: Vec::new(),
                });
                doc.turns.len().saturating_sub(1)
            };
        let turn = &mut doc.turns[turn_index];
        let interaction_index = if let Some(index) = turn
            .interactions
            .iter()
            .position(|interaction| interaction.round == entry.round)
        {
            index
        } else {
            turn.interactions.push(ActionAuditInteraction {
                round: entry.round,
                actions: Vec::new(),
            });
            turn.interactions.len().saturating_sub(1)
        };
        turn.interactions[interaction_index].actions.push(entry);
        self.write_doc(&doc);
    }

    fn read_doc(&self) -> ActionAuditDocument {
        let Ok(text) = fs::read_to_string(&self.file) else {
            return Self::empty_doc();
        };
        serde_json::from_str(&text).unwrap_or_else(|_| Self::empty_doc())
    }

    fn write_doc(&self, doc: &ActionAuditDocument) {
        if let Some(parent) = self.file.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let Ok(text) = serde_json::to_string_pretty(doc) else {
            return;
        };
        let _ = fs::write(&self.file, format!("{text}\n"));
    }

    fn empty_doc() -> ActionAuditDocument {
        ActionAuditDocument {
            version: 1,
            turns: Vec::new(),
        }
    }
}

#[derive(Debug)]
pub struct AgentCore {
    static_prompt: String,
    profile: CoreProfile,
    memory: FileMemoryStore,
    scratch: FileScratchStore,
    chat_history: FileChatHistoryStore,
    shell_jobs: FileShellJobStore,
    action_audit: FileActionAuditStore,
    deltas: Vec<PromptDelta>,
    max_llm_input_tokens: u32,
    next_shrink_review_prompt_tokens: u32,
    last_observed_prompt_tokens: u32,
    configured_round_budget: u32,
    round_budget: u32,
    current_round: u32,
    current_stats: UsageStats,
    repair_attempted: bool,
    last_repair_issue: Option<String>,
    pending_approval: Option<PendingApproval>,
    bash_approval_mode: BashApprovalMode,
    current_action_turn_id: Option<String>,
    current_action_user_question: String,
}
impl AgentCore {
    pub fn new(
        static_prompt: impl Into<String>,
        profile: CoreProfile,
        memory_dir: impl AsRef<Path>,
    ) -> Self {
        let memory_dir = memory_dir.as_ref();
        Self {
            static_prompt: static_prompt.into(),
            profile,
            memory: FileMemoryStore::new(memory_dir),
            scratch: FileScratchStore::new(memory_dir),
            chat_history: FileChatHistoryStore::new(memory_dir),
            shell_jobs: FileShellJobStore::new(memory_dir),
            action_audit: FileActionAuditStore::new(memory_dir),
            deltas: Vec::new(),
            max_llm_input_tokens: 100_000,
            next_shrink_review_prompt_tokens: 0,
            last_observed_prompt_tokens: 0,
            configured_round_budget: DEFAULT_ROUND_BUDGET,
            round_budget: DEFAULT_ROUND_BUDGET,
            current_round: 0,
            current_stats: UsageStats::zero(),
            repair_attempted: false,
            last_repair_issue: None,
            pending_approval: None,
            bash_approval_mode: BashApprovalMode::Ask,
            current_action_turn_id: None,
            current_action_user_question: String::new(),
        }
    }
    pub fn set_bash_approval_mode(&mut self, mode: BashApprovalMode) {
        self.bash_approval_mode = mode;
    }
    pub fn set_max_llm_input_tokens(&mut self, max_llm_input_tokens: u32) {
        self.max_llm_input_tokens = max_llm_input_tokens.max(3_000);
    }
    pub fn set_max_rounds(&mut self, max_rounds: u32) {
        self.configured_round_budget = max_rounds.max(1);
        self.round_budget = self.configured_round_budget;
    }
    pub fn profile(&self) -> &CoreProfile {
        &self.profile
    }
    pub fn memory_file(&self) -> PathBuf {
        self.memory.file.clone()
    }
    pub fn scratch_file(&self) -> PathBuf {
        self.scratch.file.clone()
    }
    pub fn current_stats(&self) -> &UsageStats {
        &self.current_stats
    }
    pub fn dynamic_context_estimated_tokens(&self) -> u32 {
        self.render_prompt_slices()
            .iter()
            .map(|slice| estimate_prompt_tokens(&slice.text))
            .sum()
    }
    pub fn clear_dynamic_context(&mut self) {
        self.deltas.clear();
        self.next_shrink_review_prompt_tokens = 0;
        self.last_observed_prompt_tokens = 0;
        self.current_round = 0;
        self.current_stats = UsageStats::zero();
        self.repair_attempted = false;
        self.last_repair_issue = None;
        self.pending_approval = None;
        self.current_action_turn_id = None;
        self.current_action_user_question.clear();
    }
    pub fn memory_git_commit_count(&self) -> usize {
        self.memory.git_commit_count()
    }
    pub fn begin_turn(&mut self, user_input: &str, supporting_context: Option<&str>) -> CoreStep {
        self.current_round = 0;
        self.round_budget = self.configured_round_budget;
        self.current_stats = UsageStats::zero();
        self.repair_attempted = false;
        self.last_repair_issue = None;
        self.pending_approval = None;
        let action_turn_id = unique_id("action_turn");
        self.current_action_turn_id = Some(action_turn_id.clone());
        self.current_action_user_question = user_input.trim().to_string();
        self.action_audit.begin_turn(
            &action_turn_id,
            now_ms(),
            &self.current_action_user_question,
        );
        let should_memory_precheck = supporting_context
            .map(should_run_memory_precheck)
            .unwrap_or(false);
        let mut text = format!("User question:\n{}", user_input.trim());
        if let Some(ctx) = supporting_context.map(str::trim).filter(|x| !x.is_empty()) {
            text.push_str("\n\nSupporting context:\n");
            text.push_str(ctx);
        }
        let incoming_prompt_tokens = estimate_prompt_tokens(&text);
        if let Some(shrink_review) = self.consume_shrink_review_if_needed(incoming_prompt_tokens) {
            text.push_str("\n\nLong-context maintenance:\n");
            text.push_str(&shrink_review);
        }
        text.push_str(&format!("\nrounds_remaining: {}", self.round_budget));
        let mut slices = vec![("user_question".to_string(), text)];
        if should_memory_precheck {
            let result = self.runtime_memory_precheck(user_input, 5);
            slices.push(("result_of_llm_action".to_string(), result));
        }
        self.append_delta(slices);
        CoreStep::NeedModel {
            prompt: self.render_prompt(),
            rounds_remaining: self.round_budget,
        }
    }
    pub fn apply_model_response(&mut self, response: LlmResponse) -> CoreStep {
        self.current_round += 1;
        self.current_stats.add(&response.usage);
        self.last_observed_prompt_tokens = self
            .last_observed_prompt_tokens
            .max(response.usage.prompt_tokens);
        if response.truncated && !self.repair_attempted {
            return self.request_protocol_repair(
                "truncated_model_output",
                "The previous model output was cut off by the max output token limit before a complete JSON object was produced. Return one short valid JSON object only. If the task needs a long report, use run_bash to write the full report to a file and keep response_to_user concise.",
            );
        }
        let parsed = parse_envelope(&response.content);
        if parsed.repair_issue.is_none()
            && parsed.delta_scores.is_empty()
            && self.has_unscored_prompt_delta()
        {
            if !self.repair_attempted {
                return self.request_protocol_repair(
                    "durable_ctx_score_required_for_unscored_delta",
                    "Return exactly one valid JSON object and include durable_ctx_score for the most recent unscored prompt_delta, or delta_scores with explicit delta_id values.",
                );
            }
        }
        self.apply_delta_scores(&parsed.delta_scores);
        let mut slices = Vec::new();
        if !parsed.thought.is_empty() {
            slices.push((
                "llm_thought".to_string(),
                format!("Thought:\n{}", parsed.thought),
            ));
        }
        if let Some(issue) = parsed.repair_issue.clone() {
            if !self.repair_attempted {
                return self.request_protocol_repair(
                    &issue,
                    "Return exactly one valid JSON object with response_to_user. Do not use markdown fences.",
                );
            }
            let final_text = if parsed.response_to_user.trim().is_empty() {
                repair_failure_message(self.last_repair_issue.as_deref().unwrap_or(&issue), &issue)
            } else {
                parsed.response_to_user
            };
            slices.push((
                "llm_response".to_string(),
                format!("Response shown to user:\n{}", final_text),
            ));
            self.append_delta(slices);
            return CoreStep::Final(TurnFinal {
                response_to_user: final_text,
                stats: self.current_stats.clone(),
                profile_label: self.profile.label(),
                repair_issue: Some(issue),
            });
        }
        if !parsed.next_actions.is_empty() {
            let mut result_lines = Vec::new();
            for action in parsed.next_actions {
                match self.execute_action(action) {
                    ActionExecution::Completed(result) => result_lines.push(result),
                    ActionExecution::NeedsApproval(pending) => {
                        if !result_lines.is_empty() {
                            slices.push((
                                "result_of_llm_action".to_string(),
                                result_lines.join("\n\n"),
                            ));
                        }
                        self.append_delta(slices);
                        let request = pending.request.clone();
                        self.pending_approval = Some(pending);
                        return CoreStep::NeedsUserApproval { request };
                    }
                }
            }
            if !result_lines.is_empty() {
                slices.push((
                    "result_of_llm_action".to_string(),
                    result_lines.join("\n\n"),
                ));
            }
            self.append_delta(slices);
            self.append_in_turn_shrink_review_if_needed();
            if self.remaining_rounds() == 0 {
                return CoreStep::RoundLimitReached {
                    max_rounds: self.round_budget,
                };
            }
            return CoreStep::NeedModel {
                prompt: self.render_prompt(),
                rounds_remaining: self.remaining_rounds(),
            };
        }
        for candidate in parsed.memory_candidates {
            if self.memory.write(&candidate).is_ok() {
                self.current_stats.tool_calls += 1;
                self.current_stats.mem_writes += 1;
            }
        }
        let final_text = parsed.response_to_user.trim().to_string();
        let final_text = if final_text.is_empty() {
            response.content
        } else {
            final_text
        };
        slices.push((
            "llm_response".to_string(),
            format!("Response shown to user:\n{}", final_text),
        ));
        self.append_delta(slices);
        CoreStep::Final(TurnFinal {
            response_to_user: final_text,
            stats: self.current_stats.clone(),
            profile_label: self.profile.label(),
            repair_issue: None,
        })
    }
    pub fn resolve_user_approval(&mut self, approval_id: &str, approved: bool) -> CoreStep {
        let Some(pending) = self.pending_approval.take() else {
            self.append_delta(vec![(
                "result_of_llm_action".to_string(),
                format!(
                    "Action result: user_approval\napproval_id: {}\nerror: no_pending_approval",
                    approval_id
                ),
            )]);
            return CoreStep::NeedModel {
                prompt: self.render_prompt(),
                rounds_remaining: self.remaining_rounds(),
            };
        };
        if pending.request.approval_id != approval_id {
            let request = pending.request.clone();
            self.pending_approval = Some(pending);
            return CoreStep::NeedsUserApproval { request };
        }
        let result = if approved {
            execute_approved_bash(
                &pending.command,
                &pending.read_back_command,
                pending.large_readback_opt_in,
                pending.background,
                pending.timeout_ms,
                &pending.request,
                &self.shell_jobs,
            )
        } else {
            format!(
                "Action result: run_bash\ncommand: {}\napproval_id: {}\nstatus: denied_by_user\nreason: {}",
                pending.command, pending.request.approval_id, pending.request.reason
            )
        };
        self.record_pending_approval_audit(&pending, approved, &result);
        self.append_delta(vec![("result_of_llm_action".to_string(), result)]);
        self.append_in_turn_shrink_review_if_needed();
        if self.remaining_rounds() == 0 {
            return CoreStep::RoundLimitReached {
                max_rounds: self.round_budget,
            };
        }
        CoreStep::NeedModel {
            prompt: self.render_prompt(),
            rounds_remaining: self.remaining_rounds(),
        }
    }
    pub fn continue_after_round_limit(&mut self) -> CoreStep {
        self.current_round = 0;
        self.round_budget = DEFAULT_ROUND_BUDGET;
        self.append_delta(vec![(
            "result_of_llm_action".to_string(),
            format!(
                "Runtime round budget continued by user.\nrounds_remaining: {}",
                self.round_budget
            ),
        )]);
        CoreStep::NeedModel {
            prompt: self.render_prompt(),
            rounds_remaining: self.remaining_rounds(),
        }
    }
    pub fn render_prompt(&self) -> String {
        let mut out = format!(
            "[BEGIN SEGMENT 0: prompt_0]\n{}\n[END SEGMENT 0: prompt_0]",
            self.static_prompt
        );
        let slices = self.render_prompt_slices();
        for (idx, slice) in slices.iter().enumerate() {
            out.push('\n');
            out.push_str(&format!("[BEGIN SEGMENT {}: prompt_delta]\n", idx + 1));
            out.push_str(&format!(
                "delta_id: {}\ndurable_ctx_score: {}\nslice_id: {}\nslice: {}/{}\n",
                slice.delta_id,
                durable_score_label(slice.durable_ctx_score),
                slice.slice_id,
                slice.slice_index,
                slice.slice_count
            ));
            out.push_str(&format!(
                "prompt_type: {}\n{}\ntime: {}\n",
                slice.prompt_type, slice.text, slice.time_ms
            ));
            out.push_str(&format!("[END SEGMENT {}: prompt_delta]", idx + 1));
        }
        out
    }
    fn render_prompt_slices(&self) -> Vec<PromptSlice> {
        self.deltas
            .iter()
            .flat_map(render_delta_slices)
            .collect::<Vec<_>>()
    }
    fn remaining_rounds(&self) -> u32 {
        self.round_budget.saturating_sub(self.current_round)
    }

    fn has_unscored_prompt_delta(&self) -> bool {
        self.deltas
            .iter()
            .any(|delta| delta.durable_ctx_score.is_none())
    }

    fn request_protocol_repair(&mut self, issue: &str, instruction: &str) -> CoreStep {
        self.repair_attempted = true;
        self.last_repair_issue = Some(issue.to_string());
        self.append_delta(vec![(
            "result_of_llm_action".to_string(),
            format!("Protocol repair request\nissue: {}\n{}", issue, instruction),
        )]);
        CoreStep::NeedModel {
            prompt: self.render_prompt(),
            rounds_remaining: self.remaining_rounds(),
        }
    }

    fn append_in_turn_shrink_review_if_needed(&mut self) {
        if let Some(shrink_review) = self.consume_shrink_review_if_needed(0) {
            self.append_delta(vec![(
                "result_of_llm_action".to_string(),
                format!("Long-context maintenance:\n{shrink_review}"),
            )]);
        }
    }

    fn apply_delta_scores(&mut self, delta_scores: &[DeltaScore]) {
        for score in delta_scores {
            if let Some(delta_id) = score
                .delta_id
                .as_deref()
                .map(str::trim)
                .filter(|id| !id.is_empty())
            {
                if let Some(delta) = self
                    .deltas
                    .iter_mut()
                    .find(|delta| delta.delta_id == delta_id)
                {
                    delta.durable_ctx_score = Some(score.durable_ctx_score);
                    for slice in &mut delta.slices {
                        slice.durable_ctx_score = Some(score.durable_ctx_score);
                    }
                }
                continue;
            }
            if let Some(delta) = self
                .deltas
                .iter_mut()
                .rev()
                .find(|delta| delta.durable_ctx_score.is_none())
            {
                delta.durable_ctx_score = Some(score.durable_ctx_score);
                for slice in &mut delta.slices {
                    slice.durable_ctx_score = Some(score.durable_ctx_score);
                }
            }
        }
    }

    fn append_delta(&mut self, slice_texts: Vec<(String, String)>) {
        if slice_texts.is_empty() {
            return;
        }
        let timestamp = now_ms();
        let delta_id = format!("pd_{}_{}", timestamp, self.deltas.len() + 1);
        let chunks = slice_texts
            .into_iter()
            .flat_map(|(prompt_type, text)| {
                let slice_time_ms = now_ms();
                split_text_for_prompt_slices(&text, PROMPT_SLICE_TEXT_LIMIT)
                    .into_iter()
                    .map(move |chunk| (prompt_type.clone(), slice_time_ms, chunk))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let slice_count = chunks.len();
        let slices = chunks
            .into_iter()
            .enumerate()
            .map(|(idx, (prompt_type, time_ms, text))| {
                let slice_index = idx + 1;
                PromptSlice {
                    delta_id: delta_id.clone(),
                    slice_id: format!(
                        "ps_{}_s{:03}",
                        delta_id.trim_start_matches("pd_"),
                        slice_index
                    ),
                    durable_ctx_score: None,
                    prompt_type,
                    time_ms,
                    text,
                    slice_index,
                    slice_count,
                }
            })
            .collect::<Vec<_>>();
        self.deltas.push(PromptDelta {
            delta_id,
            time_ms: timestamp,
            durable_ctx_score: None,
            slices,
            hidden_slice_ids: Vec::new(),
        });
    }
    fn consume_shrink_review_if_needed(&mut self, incoming_prompt_tokens: u32) -> Option<String> {
        let estimated_prompt_tokens = self.estimate_rendered_prompt_tokens(incoming_prompt_tokens);
        let first_threshold = self.max_llm_input_tokens / 3;
        let followup_step = self.max_llm_input_tokens / 5;
        let force_threshold = self.max_llm_input_tokens.saturating_mul(95) / 100;
        let threshold = if self.next_shrink_review_prompt_tokens == 0 {
            first_threshold
        } else {
            self.next_shrink_review_prompt_tokens
        };
        let slices = self.render_prompt_slices();
        if slices.is_empty() {
            return None;
        }
        let force_shrink = estimated_prompt_tokens >= force_threshold;
        if estimated_prompt_tokens < threshold && !force_shrink {
            return None;
        }
        let current_count = self.deltas.len();
        let slice_count = slices.len();
        let delta_refs = self
            .deltas
            .iter()
            .filter(|delta| !render_delta_slices(delta).is_empty())
            .rev()
            .take(12)
            .map(|delta| {
                let token_estimate = render_delta_slices(delta)
                    .iter()
                    .map(|slice| estimate_prompt_tokens(&slice.text))
                    .sum::<u32>();
                format!(
                    "- delta_id={} durable_ctx_score={} time_ms={} visible_slices={} estimated_tokens={}",
                    delta.delta_id,
                    durable_score_label(delta.durable_ctx_score),
                    delta.time_ms,
                    render_delta_slices(delta).len(),
                    token_estimate
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        let recent_refs = slices
            .iter()
            .rev()
            .take(8)
            .map(|slice| {
                format!(
                    "- slice_id={} delta_id={} durable_ctx_score={} slice={}/{} prompt_type={} time_ms={}",
                    slice.slice_id,
                    slice.delta_id,
                    durable_score_label(slice.durable_ctx_score),
                    slice.slice_index,
                    slice.slice_count,
                    slice.prompt_type,
                    slice.time_ms
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        self.next_shrink_review_prompt_tokens =
            estimated_prompt_tokens.saturating_add(followup_step);
        let mode = if force_shrink {
            "force_shrink_required"
        } else {
            "shrink_review"
        };
        let instruction = if force_shrink {
            "Context is above 95% of the configured window. You must use prompt_shrink before continuing: remove low durable_ctx_score or stale dynamic deltas/slices, and rewrite a compact summary for only the current work-relevant and high durable_ctx_score knowledge in response_to_user or scratch/memory actions as appropriate. Do not target prompt_0."
        } else {
            "Decide whether stale or irrelevant dynamic prompt deltas or rendered slices should be compacted with prompt_shrink. Prefer shrinking low durable_ctx_score content first. If suggesting shrink, refer to delta_id for whole logical deltas or slice_id for specific rendered slices; if not, continue normally."
        };
        Some(format!(
            "mode={mode}\nestimated_prompt_tokens={estimated_prompt_tokens}\nmax_llm_input_tokens={}\nshrink_review_threshold_tokens={threshold}\nfirst_shrink_review_threshold_tokens={first_threshold}\nnext_shrink_review_step_tokens={followup_step}\nforce_shrink_threshold_tokens={force_threshold}\nprompt_delta_count={current_count}\nprompt_slice_count={slice_count}\nrecent_prompt_delta_refs:\n{delta_refs}\nrecent_prompt_slice_refs:\n{recent_refs}\n{instruction}",
            self.max_llm_input_tokens
        ))
    }
    fn estimate_rendered_prompt_tokens(&self, incoming_prompt_tokens: u32) -> u32 {
        self.last_observed_prompt_tokens
            .saturating_add(incoming_prompt_tokens)
            .max(estimate_prompt_tokens(&self.render_prompt()))
    }
    fn runtime_memory_precheck(&mut self, query: &str, limit: usize) -> String {
        self.current_stats.tool_calls += 1;
        self.current_stats.mem_reads += 1;
        match self.memory.query(query, limit) {
            Ok(rows) if rows.is_empty() => match self.memory.recent(limit) {
                Ok(recent) if recent.is_empty() => format!(
                    "Action result: runtime_memory_precheck\nquery: {}\nresults: none",
                    query.trim()
                ),
                Ok(recent) => {
                    let lines = recent
                        .into_iter()
                        .map(|r| format!("- {} @ {}", r.content, r.created_at_ms))
                        .collect::<Vec<_>>()
                        .join("\n");
                    format!(
                        "Action result: runtime_memory_precheck\nquery: {}\nlexical_results: none\nrecent_memory_evidence:\n{}",
                        query.trim(),
                        lines
                    )
                }
                Err(_) => format!(
                    "Action result: runtime_memory_precheck\nquery: {}\nerror: memory_read_failed",
                    query.trim()
                ),
            },
            Ok(rows) => {
                let lines = rows
                    .into_iter()
                    .map(|r| format!("- {} @ {}", r.content, r.created_at_ms))
                    .collect::<Vec<_>>()
                    .join("\n");
                format!(
                    "Action result: runtime_memory_precheck\nquery: {}\nresults:\n{}",
                    query.trim(),
                    lines
                )
            }
            Err(_) => format!(
                "Action result: runtime_memory_precheck\nquery: {}\nerror: memory_read_failed",
                query.trim()
            ),
        }
    }
    fn query_prompt_slices(
        &self,
        query: &str,
        limit: usize,
        after_ms: Option<i64>,
        before_ms: Option<i64>,
    ) -> Vec<PromptSlice> {
        let terms = search_terms(query);
        let mut rows = self
            .render_prompt_slices()
            .into_iter()
            .filter(|slice| {
                if !time_in_window(slice.time_ms, after_ms, before_ms) {
                    return false;
                }
                if terms.is_empty() {
                    return true;
                }
                let text = slice.text.to_lowercase();
                terms.iter().any(|term| text.contains(term))
            })
            .collect::<Vec<_>>();
        rows.sort_by(|a, b| b.time_ms.cmp(&a.time_ms));
        rows.truncate(limit.max(1).min(50));
        rows
    }

    fn execute_action(&mut self, action: ParsedAction) -> ActionExecution {
        let action_for_audit = action.clone();
        let result = match action.action.as_str() {
            "chat_history_query" => {
                self.current_stats.tool_calls += 1;
                let rows = self
                    .chat_history
                    .query(
                        &action.query,
                        action.limit,
                        action.after_ms,
                        action.before_ms,
                    )
                    .unwrap_or_default();
                let delta_rows = self.query_prompt_slices(
                    &action.query,
                    action.limit,
                    action.after_ms,
                    action.before_ms,
                );
                if rows.is_empty() && delta_rows.is_empty() {
                    format!(
                        "Action result: chat_history_query\nquery: {}\nresults: none",
                        action.query
                    )
                } else {
                    let mut sections = Vec::new();
                    if !rows.is_empty() {
                        let lines = rows
                            .into_iter()
                            .map(|record| {
                                format!(
                                    "- source=chat_record time_ms={} session={} turn_id={} user={} assistant={}",
                                    record.started_at_ms,
                                    record.session,
                                    record.turn_id,
                                    compact_text(&record.user_input, 160),
                                    compact_text(&record.assistant_output, 220)
                                )
                            })
                            .collect::<Vec<_>>()
                            .join("\n");
                        sections.push(format!("chat_records:\n{}", lines));
                    }
                    if !delta_rows.is_empty() {
                        let lines = delta_rows
                            .into_iter()
                            .map(|slice| {
                                format!(
                                    "- source=prompt_delta delta_id={} slice_id={} slice={}/{} prompt_type={} time_ms={} text={}",
                                    slice.delta_id,
                                    slice.slice_id,
                                    slice.slice_index,
                                    slice.slice_count,
                                    slice.prompt_type,
                                    slice.time_ms,
                                    compact_text(&slice.text, 240)
                                )
                            })
                            .collect::<Vec<_>>()
                            .join("\n");
                        sections.push(format!("current_prompt_deltas:\n{}", lines));
                    }
                    format!(
                        "Action result: chat_history_query\nquery: {}\nresults:\n{}",
                        action.query,
                        sections.join("\n")
                    )
                }
            }
            "chat_history_delete" => {
                self.current_stats.tool_calls += 1;
                match self.chat_history.delete(
                    &action.id,
                    &action.query,
                    action.limit,
                    action.after_ms,
                    action.before_ms,
                ) {
                    Ok(deleted) => format!(
                        "Action result: chat_history_delete\nid: {}\nquery: {}\ndeleted_count: {}",
                        action.id, action.query, deleted
                    ),
                    Err(err) => format!("Action result: chat_history_delete\nerror: {}", err),
                }
            }
            "query_memory" | "memory_query" => {
                self.current_stats.tool_calls += 1;
                self.current_stats.mem_reads += 1;
                let rows = self
                    .memory
                    .query(&action.query, action.limit)
                    .unwrap_or_default();
                if rows.is_empty() {
                    format!(
                        "Action result: query_memory\nquery: {}\nresults: none",
                        action.query
                    )
                } else {
                    let lines = rows
                        .into_iter()
                        .map(|r| format!("- {} @ {}", r.content, r.created_at_ms))
                        .collect::<Vec<_>>()
                        .join("\n");
                    format!(
                        "Action result: query_memory\nquery: {}\nresults:\n{}",
                        action.query, lines
                    )
                }
            }
            "memory_schema" => {
                self.current_stats.tool_calls += 1;
                self.current_stats.mem_reads += 1;
                self.memory.schema_text(&self.chat_history)
            }
            "sql_read" | "memory_sql_query" => {
                self.current_stats.tool_calls += 1;
                self.current_stats.mem_reads += 1;
                match self.memory.sql_read(
                    &self.chat_history,
                    &action.sql,
                    &action.params,
                    action.limit,
                ) {
                    Ok(rows) if rows.is_empty() => {
                        format!(
                            "Action result: {}\nsql: {}\nresults: none",
                            action.action, action.sql
                        )
                    }
                    Ok(rows) => {
                        let lines = rows
                            .into_iter()
                            .map(|row| {
                                let cells = row
                                    .into_iter()
                                    .map(|(column, value)| format!("{}={}", column, value))
                                    .collect::<Vec<_>>()
                                    .join(", ");
                                format!("- {}", cells)
                            })
                            .collect::<Vec<_>>()
                            .join("\n");
                        format!(
                            "Action result: {}\nsql: {}\nresults:\n{}",
                            action.action, action.sql, lines
                        )
                    }
                    Err(err) => format!(
                        "Action result: {}\nsql: {}\nerror: {}",
                        action.action, action.sql, err
                    ),
                }
            }
            "memory_write" | "write_memory" => {
                self.current_stats.tool_calls += 1;
                let content = if action.content.trim().is_empty() {
                    action.query
                } else {
                    action.content
                };
                if content.trim().is_empty() {
                    "Action result: memory_write\nskipped: empty content".to_string()
                } else if self.memory.write(&content).is_ok() {
                    self.current_stats.mem_writes += 1;
                    format!("Action result: memory_write\nstored: {}", content)
                } else {
                    "Action result: memory_write\nerror: write_failed".to_string()
                }
            }
            "memory_update" => {
                self.current_stats.tool_calls += 1;
                match self
                    .memory
                    .update(&action.operation, &action.id, &action.content)
                {
                    Ok(result) => {
                        self.current_stats.mem_writes += 1;
                        result
                    }
                    Err(err) => format!("Action result: memory_update\nerror: {}", err),
                }
            }
            "scratch_write" => {
                self.current_stats.tool_calls += 1;
                match self.scratch.write(&action.content) {
                    Ok(record) => format!(
                        "Action result: scratch_write\nid: {}\nstored: {}",
                        record.id, record.content
                    ),
                    Err(err) => format!("Action result: scratch_write\nerror: {}", err),
                }
            }
            "scratch_query" => {
                self.current_stats.tool_calls += 1;
                match self.scratch.query(&action.query, action.limit) {
                    Ok(rows) if rows.is_empty() => format!(
                        "Action result: scratch_query\nquery: {}\nresults: none",
                        action.query
                    ),
                    Ok(rows) => {
                        let lines = rows
                            .into_iter()
                            .map(|row| {
                                format!(
                                    "- id={} time_ms={} content={}",
                                    row.id,
                                    row.created_at_ms,
                                    compact_text(&row.content, 240)
                                )
                            })
                            .collect::<Vec<_>>()
                            .join("\n");
                        format!(
                            "Action result: scratch_query\nquery: {}\nresults:\n{}",
                            action.query, lines
                        )
                    }
                    Err(err) => format!("Action result: scratch_query\nerror: {}", err),
                }
            }
            "scratch_delete" => {
                self.current_stats.tool_calls += 1;
                match self.scratch.delete(&action.id) {
                    Ok(true) => format!(
                        "Action result: scratch_delete\nid: {}\ndeleted: true",
                        action.id
                    ),
                    Ok(false) => format!(
                        "Action result: scratch_delete\nid: {}\ndeleted: false",
                        action.id
                    ),
                    Err(err) => format!("Action result: scratch_delete\nerror: {}", err),
                }
            }
            "prompt_shrink" => {
                self.current_stats.tool_calls += 1;
                self.apply_prompt_shrink(&action.delta_ids, &action.slice_ids)
            }
            "shell_job_status" => {
                self.current_stats.tool_calls += 1;
                self.shell_jobs.status(&action.job_id, action.timeout_ms)
            }
            "run_bash" => {
                self.current_stats.tool_calls += 1;
                let execution = execute_guarded_bash(
                    &action.command,
                    &action.read_back_command,
                    action.large_readback_opt_in,
                    action.background,
                    action.timeout_ms,
                    self.bash_approval_mode,
                    &action.intent,
                    &self.shell_jobs,
                );
                match &execution {
                    ActionExecution::Completed(result) => {
                        self.record_action_audit(&action_for_audit, "completed", Some(result));
                    }
                    ActionExecution::NeedsApproval(pending) => {
                        let result = format!(
                            "Action result: run_bash\ncommand: {}\napproval_id: {}\nstatus: needs_user_approval\nrisk: {}\nreason: {}",
                            pending.command,
                            pending.request.approval_id,
                            pending.request.risk,
                            pending.request.reason
                        );
                        self.record_action_audit(
                            &action_for_audit,
                            "needs_user_approval",
                            Some(&result),
                        );
                    }
                }
                return execution;
            }
            other => format!("Action result: {}\nunsupported native action", other),
        };
        self.record_action_audit(&action_for_audit, "completed", Some(&result));
        ActionExecution::Completed(result)
    }

    fn record_action_audit(&self, action: &ParsedAction, status: &str, result: Option<&str>) {
        let turn_id = self
            .current_action_turn_id
            .as_deref()
            .unwrap_or("unknown_turn");
        self.action_audit.record_action(
            ActionAuditEntry {
                time_ms: now_ms(),
                round: self.current_round.max(1),
                action: action.action.clone(),
                intent: action.intent.clone(),
                status: status.to_string(),
                input: action.audit_input(),
                result_summary: result.map(|text| compact_text(text, 2_000)),
            },
            turn_id,
            &self.current_action_user_question,
        );
    }

    fn record_pending_approval_audit(
        &self,
        pending: &PendingApproval,
        approved: bool,
        result: &str,
    ) {
        let turn_id = self
            .current_action_turn_id
            .as_deref()
            .unwrap_or("unknown_turn");
        self.action_audit.record_action(
            ActionAuditEntry {
                time_ms: now_ms(),
                round: self.current_round.max(1),
                action: pending.request.action.clone(),
                intent: pending.intent.clone(),
                status: if approved {
                    "approved_completed".to_string()
                } else {
                    "denied_by_user".to_string()
                },
                input: json!({
                    "command": pending.command,
                    "read_back_command": pending.read_back_command,
                    "background": pending.background,
                    "timeout_ms": pending.timeout_ms,
                    "approval_id": pending.request.approval_id,
                    "risk": pending.request.risk,
                    "reason": pending.request.reason,
                }),
                result_summary: Some(compact_text(result, 2_000)),
            },
            turn_id,
            &self.current_action_user_question,
        );
    }

    fn apply_prompt_shrink(&mut self, delta_ids: &[String], slice_ids: &[String]) -> String {
        let delta_id_set = delta_ids
            .iter()
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty())
            .collect::<HashSet<_>>();
        let slice_id_set = slice_ids
            .iter()
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty())
            .collect::<HashSet<_>>();
        let existing_delta_ids = self
            .deltas
            .iter()
            .map(|delta| delta.delta_id.clone())
            .collect::<HashSet<_>>();
        let mut shrunk_tokens_estimate = 0u32;
        for delta in &self.deltas {
            if delta_id_set.contains(&delta.delta_id) {
                for slice in render_delta_slices(delta) {
                    shrunk_tokens_estimate =
                        shrunk_tokens_estimate.saturating_add(estimate_prompt_tokens(&slice.text));
                }
            }
        }
        let before_delta_count = self.deltas.len();
        if !delta_id_set.is_empty() {
            self.deltas
                .retain(|delta| !delta_id_set.contains(&delta.delta_id));
        }
        let removed_delta_count = before_delta_count.saturating_sub(self.deltas.len());

        let mut hidden_slice_count = 0usize;
        let mut matched_slice_ids = HashSet::new();
        if !slice_id_set.is_empty() {
            for delta in &mut self.deltas {
                let slices = render_delta_slices(delta);
                for slice in slices {
                    if slice_id_set.contains(&slice.slice_id) {
                        matched_slice_ids.insert(slice.slice_id.clone());
                        if !delta.hidden_slice_ids.contains(&slice.slice_id) {
                            shrunk_tokens_estimate = shrunk_tokens_estimate
                                .saturating_add(estimate_prompt_tokens(&slice.text));
                            delta.hidden_slice_ids.push(slice.slice_id);
                            hidden_slice_count += 1;
                        }
                    }
                }
            }
        }
        let mut missing = delta_id_set
            .into_iter()
            .filter(|id| !existing_delta_ids.contains(id))
            .collect::<Vec<_>>();
        for id in slice_id_set {
            if !matched_slice_ids.contains(&id) {
                missing.push(id);
            }
        }
        missing.sort();
        missing.dedup();

        self.current_stats.shrunk_tokens = self
            .current_stats
            .shrunk_tokens
            .saturating_add(shrunk_tokens_estimate);
        if removed_delta_count > 0 || hidden_slice_count > 0 {
            self.next_shrink_review_prompt_tokens = 0;
        }

        let missing_text = if missing.is_empty() {
            "none".to_string()
        } else {
            missing.join(", ")
        };
        format!(
            "Action result: prompt_shrink\nremoved_delta_count: {}\nhidden_slice_count: {}\nshrunk_tokens_estimate: {}\nmissing_ids: {}",
            removed_delta_count, hidden_slice_count, shrunk_tokens_estimate, missing_text
        )
    }
}

#[derive(Debug, Clone)]
struct FileMemoryStore {
    dir: PathBuf,
    file: PathBuf,
}
impl FileMemoryStore {
    fn new(dir: &Path) -> Self {
        let _ = fs::create_dir_all(dir);
        Self {
            dir: dir.to_path_buf(),
            file: dir.join("memory.jsonl"),
        }
    }
    fn write(&self, content: &str) -> std::io::Result<()> {
        let clean = content.trim();
        if clean.is_empty() {
            return Ok(());
        }
        let record = MemoryRecord {
            id: unique_id("mem"),
            created_at_ms: now_ms(),
            content: clean.to_string(),
        };
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.file)?;
        writeln!(
            file,
            "{}",
            serde_json::to_string(&record).unwrap_or_default()
        )?;
        self.snapshot_with_git("memory write");
        Ok(())
    }
    fn query(&self, query: &str, limit: usize) -> std::io::Result<Vec<MemoryRecord>> {
        let terms = search_terms(query);
        if terms.is_empty() {
            return Ok(Vec::new());
        }
        let file = match OpenOptions::new().read(true).open(&self.file) {
            Ok(file) => file,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => return Err(err),
        };
        let mut rows = Vec::new();
        for line in BufReader::new(file).lines().map_while(Result::ok) {
            if let Ok(record) = serde_json::from_str::<MemoryRecord>(&line) {
                let normalized = record.content.to_lowercase();
                if terms.iter().any(|term| normalized.contains(term)) {
                    rows.push(record);
                }
            }
        }
        rows.sort_by(|a, b| b.created_at_ms.cmp(&a.created_at_ms));
        rows.truncate(limit.max(1).min(50));
        Ok(rows)
    }
    fn recent(&self, limit: usize) -> std::io::Result<Vec<MemoryRecord>> {
        let mut rows = self.read_all()?;
        rows.sort_by(|a, b| b.created_at_ms.cmp(&a.created_at_ms));
        rows.truncate(limit.max(1).min(50));
        Ok(rows)
    }

    fn update(&self, operation: &str, id: &str, content: &str) -> Result<String, String> {
        let op = operation.trim().to_lowercase();
        match op.as_str() {
            "insert" | "upsert" if id.trim().is_empty() => {
                let clean = content.trim();
                if clean.is_empty() {
                    return Err("content_required".to_string());
                }
                self.write(clean).map_err(|_| "write_failed".to_string())?;
                Ok(format!(
                    "Action result: memory_update\noperation: insert\nstored: {}",
                    clean
                ))
            }
            "update" | "upsert" => {
                let clean_id = id.trim();
                let clean = content.trim();
                if clean_id.is_empty() {
                    return Err("id_required".to_string());
                }
                if clean.is_empty() {
                    return Err("content_required".to_string());
                }
                let mut rows = self
                    .read_all()
                    .map_err(|_| "memory_read_failed".to_string())?;
                let mut found = false;
                for row in &mut rows {
                    if row.id == clean_id {
                        row.content = clean.to_string();
                        found = true;
                        break;
                    }
                }
                if !found {
                    rows.push(MemoryRecord {
                        id: clean_id.to_string(),
                        created_at_ms: now_ms(),
                        content: clean.to_string(),
                    });
                }
                self.write_all(&rows)
                    .map_err(|_| "write_failed".to_string())?;
                Ok(format!(
                    "Action result: memory_update\noperation: {}\nid: {}\nstored: {}",
                    if found { "update" } else { "insert" },
                    clean_id,
                    clean
                ))
            }
            "delete" => {
                let clean_id = id.trim();
                if clean_id.is_empty() {
                    return Err("id_required".to_string());
                }
                let mut rows = self
                    .read_all()
                    .map_err(|_| "memory_read_failed".to_string())?;
                let before = rows.len();
                rows.retain(|row| row.id != clean_id);
                if rows.len() == before {
                    return Err("id_not_found".to_string());
                }
                self.write_all(&rows)
                    .map_err(|_| "write_failed".to_string())?;
                Ok(format!(
                    "Action result: memory_update\noperation: delete\nid: {}\ndeleted: true",
                    clean_id
                ))
            }
            _ => Err("operation_must_be_insert_update_upsert_or_delete".to_string()),
        }
    }

    fn write_all(&self, rows: &[MemoryRecord]) -> std::io::Result<()> {
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&self.file)?;
        for row in rows {
            writeln!(file, "{}", serde_json::to_string(row).unwrap_or_default())?;
        }
        self.snapshot_with_git("memory update");
        Ok(())
    }

    fn snapshot_with_git(&self, message: &str) {
        if !self.file.exists() {
            return;
        }
        if Command::new("git")
            .arg("-C")
            .arg(&self.dir)
            .arg("init")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| !status.success())
            .unwrap_or(true)
        {
            return;
        }
        let _ = Command::new("git")
            .arg("-C")
            .arg(&self.dir)
            .args(["config", "user.name", "timem-memory"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = Command::new("git")
            .arg("-C")
            .arg(&self.dir)
            .args(["config", "user.email", "timem-memory@example.invalid"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        if Command::new("git")
            .arg("-C")
            .arg(&self.dir)
            .args(["add", "memory.jsonl"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| !status.success())
            .unwrap_or(true)
        {
            return;
        }
        let _ = Command::new("git")
            .arg("-C")
            .arg(&self.dir)
            .args(["commit", "-m", message])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }

    fn read_all(&self) -> std::io::Result<Vec<MemoryRecord>> {
        let file = match OpenOptions::new().read(true).open(&self.file) {
            Ok(file) => file,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => return Err(err),
        };
        let mut rows = Vec::new();
        for line in BufReader::new(file).lines().map_while(Result::ok) {
            if let Ok(record) = serde_json::from_str::<MemoryRecord>(&line) {
                rows.push(record);
            }
        }
        Ok(rows)
    }

    fn git_commit_count(&self) -> usize {
        Command::new("git")
            .arg("-C")
            .arg(&self.dir)
            .args(["rev-list", "--count", "HEAD"])
            .output()
            .ok()
            .and_then(|output| {
                if output.status.success() {
                    String::from_utf8(output.stdout).ok()
                } else {
                    None
                }
            })
            .and_then(|text| text.trim().parse::<usize>().ok())
            .unwrap_or_default()
    }

    fn schema_text(&self, chat_history: &FileChatHistoryStore) -> String {
        format!(
            "Action result: memory_schema\ntables:\n- memories(id TEXT, created_at_ms INTEGER, content TEXT)\n- chat_messages(id TEXT, session_id TEXT, turn_id TEXT, role TEXT, content TEXT, created_at_ms INTEGER, source TEXT, profile_name TEXT, model_name TEXT, source_message_id TEXT)\n- scratch_notes(id TEXT, created_at_ms INTEGER, content TEXT)\nsafe_interfaces: memory_schema, memory_sql_query, memory_update, memory_write, chat_history_query, chat_history_delete, scratch_write, scratch_query, scratch_delete\nrules: memory_sql_query accepts SELECT, WITH ... SELECT, or PRAGMA table_info(memories/chat_messages); SQL writes are forbidden; use memory_update for durable memory insert/update/delete; use chat_history_delete for explicit chat transcript deletion; use scratch_* for temporary task checkpoints. Empty chat_history_query.query lists recent chat records. loaded_chat_records={}.",
            chat_history.read_all().map(|rows| rows.len()).unwrap_or_default()
        )
    }

    fn sql_read(
        &self,
        chat_history: &FileChatHistoryStore,
        sql: &str,
        params: &[String],
        limit: usize,
    ) -> Result<Vec<Vec<(String, String)>>, String> {
        validate_memory_sql(sql)?;
        let conn = Connection::open_in_memory().map_err(|_| "sqlite_open_failed".to_string())?;
        conn.execute(
            "CREATE TABLE memories(id TEXT NOT NULL, created_at_ms INTEGER NOT NULL, content TEXT NOT NULL)",
            [],
        )
        .map_err(|_| "sqlite_schema_failed".to_string())?;
        conn.execute(
            "CREATE TABLE chat_messages(id TEXT NOT NULL, session_id TEXT NOT NULL, turn_id TEXT NOT NULL, role TEXT NOT NULL, content TEXT NOT NULL, created_at_ms INTEGER NOT NULL, source TEXT NOT NULL, profile_name TEXT, model_name TEXT, source_message_id TEXT)",
            [],
        )
        .map_err(|_| "sqlite_schema_failed".to_string())?;
        for record in self
            .read_all()
            .map_err(|_| "memory_read_failed".to_string())?
        {
            conn.execute(
                "INSERT INTO memories(id, created_at_ms, content) VALUES (?1, ?2, ?3)",
                (&record.id, record.created_at_ms, &record.content),
            )
            .map_err(|_| "sqlite_load_failed".to_string())?;
        }
        for record in chat_history
            .read_all()
            .map_err(|_| "chat_history_read_failed".to_string())?
        {
            if !record.user_input.trim().is_empty() {
                conn.execute(
                    "INSERT INTO chat_messages(id, session_id, turn_id, role, content, created_at_ms, source, profile_name, model_name, source_message_id) VALUES (?1, ?2, ?3, 'user', ?4, ?5, 'shell_audit', NULL, NULL, NULL)",
                    (
                        format!("{}_user", record.turn_id),
                        &record.session,
                        &record.turn_id,
                        &record.user_input,
                        record.started_at_ms,
                    ),
                )
                .map_err(|_| "sqlite_load_failed".to_string())?;
            }
            if !record.assistant_output.trim().is_empty() {
                conn.execute(
                    "INSERT INTO chat_messages(id, session_id, turn_id, role, content, created_at_ms, source, profile_name, model_name, source_message_id) VALUES (?1, ?2, ?3, 'assistant', ?4, ?5, 'shell_audit', NULL, NULL, ?6)",
                    (
                        format!("{}_assistant", record.turn_id),
                        &record.session,
                        &record.turn_id,
                        &record.assistant_output,
                        record.started_at_ms,
                        format!("{}_user", record.turn_id),
                    ),
                )
                .map_err(|_| "sqlite_load_failed".to_string())?;
            }
        }
        let mut stmt = conn
            .prepare(sql)
            .map_err(|_| "sql_prepare_failed".to_string())?;
        let column_names = stmt
            .column_names()
            .into_iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        let column_count = column_names.len();
        let mut rows = stmt
            .query(params_from_iter(params.iter().map(String::as_str)))
            .map_err(|_| "sql_query_failed".to_string())?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().map_err(|_| "sql_row_failed".to_string())? {
            let mut cells = Vec::new();
            for idx in 0..column_count {
                let value = match row
                    .get_ref(idx)
                    .map_err(|_| "sql_value_failed".to_string())?
                {
                    ValueRef::Null => "NULL".to_string(),
                    ValueRef::Integer(v) => v.to_string(),
                    ValueRef::Real(v) => v.to_string(),
                    ValueRef::Text(v) => String::from_utf8_lossy(v).to_string(),
                    ValueRef::Blob(_) => "<blob>".to_string(),
                };
                cells.push((column_names[idx].clone(), value));
            }
            out.push(cells);
            if out.len() >= limit.max(1).min(200) {
                break;
            }
        }
        Ok(out)
    }
}

#[derive(Debug, Clone)]
struct FileScratchStore {
    file: PathBuf,
}

impl FileScratchStore {
    fn new(dir: &Path) -> Self {
        let _ = fs::create_dir_all(dir);
        Self {
            file: dir.join("scratch_notes.jsonl"),
        }
    }

    fn write(&self, content: &str) -> Result<ScratchNoteRecord, String> {
        let clean = content.trim();
        if clean.is_empty() {
            return Err("content_required".to_string());
        }
        let record = ScratchNoteRecord {
            id: unique_id("scratch"),
            created_at_ms: now_ms(),
            content: clean.to_string(),
        };
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.file)
            .map_err(|_| "scratch_open_failed".to_string())?;
        writeln!(
            file,
            "{}",
            serde_json::to_string(&record).unwrap_or_default()
        )
        .map_err(|_| "scratch_write_failed".to_string())?;
        Ok(record)
    }

    fn query(&self, query: &str, limit: usize) -> Result<Vec<ScratchNoteRecord>, String> {
        let terms = search_terms(query);
        let mut rows = self.read_all()?;
        if !terms.is_empty() {
            rows.retain(|record| {
                let normalized = record.content.to_lowercase();
                terms.iter().any(|term| normalized.contains(term))
            });
        }
        rows.sort_by(|a, b| b.created_at_ms.cmp(&a.created_at_ms));
        rows.truncate(limit.max(1).min(50));
        Ok(rows)
    }

    fn delete(&self, id: &str) -> Result<bool, String> {
        let clean_id = id.trim();
        if clean_id.is_empty() {
            return Err("id_required".to_string());
        }
        let mut rows = self.read_all()?;
        let before = rows.len();
        rows.retain(|record| record.id != clean_id);
        if rows.len() == before {
            return Ok(false);
        }
        self.write_all(&rows)?;
        Ok(true)
    }

    fn read_all(&self) -> Result<Vec<ScratchNoteRecord>, String> {
        let file = match OpenOptions::new().read(true).open(&self.file) {
            Ok(file) => file,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(_) => return Err("scratch_read_failed".to_string()),
        };
        let mut rows = Vec::new();
        for line in BufReader::new(file).lines().map_while(Result::ok) {
            if let Ok(record) = serde_json::from_str::<ScratchNoteRecord>(&line) {
                rows.push(record);
            }
        }
        Ok(rows)
    }

    fn write_all(&self, rows: &[ScratchNoteRecord]) -> Result<(), String> {
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&self.file)
            .map_err(|_| "scratch_open_failed".to_string())?;
        for row in rows {
            writeln!(file, "{}", serde_json::to_string(row).unwrap_or_default())
                .map_err(|_| "scratch_write_failed".to_string())?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
struct FileShellJobStore {
    dir: PathBuf,
    index_file: PathBuf,
}

impl FileShellJobStore {
    fn new(memory_dir: &Path) -> Self {
        let dir = memory_dir.join("shell_jobs");
        let _ = fs::create_dir_all(&dir);
        Self {
            index_file: dir.join("jobs.jsonl"),
            dir,
        }
    }

    fn spawn(&self, command: &str) -> String {
        let clean = command.trim();
        if clean.is_empty() {
            return "Action result: run_bash\nerror: command_required".to_string();
        }
        let _ = fs::create_dir_all(&self.dir);
        let id = unique_id("job");
        let output_file = self.dir.join(format!("{id}.out"));
        let status_file = self.dir.join(format!("{id}.status"));
        let script = format!(
            "({}) > {} 2>&1; printf '%s' \"$?\" > {}",
            clean,
            shell_quote_path(&output_file),
            shell_quote_path(&status_file)
        );
        let spawn = Command::new("/bin/sh")
            .arg("-lc")
            .arg(script)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        let child = match spawn {
            Ok(child) => child,
            Err(_) => {
                return format!(
                    "Action result: run_bash\ncommand: {}\nerror: background_spawn_failed",
                    clean
                )
            }
        };
        let record = ShellJobRecord {
            id: id.clone(),
            created_at_ms: now_ms(),
            pid: child.id(),
            command: clean.to_string(),
            output_file: output_file.to_string_lossy().to_string(),
            status_file: status_file.to_string_lossy().to_string(),
        };
        let _ = self.append(&record);
        format!(
            "Action result: run_bash\ncommand: {}\nstatus: background_started\njob_id: {}\npid: {}\noutput_file: {}\nstatus_file: {}\nnext_action: shell_job_status",
            clean, record.id, record.pid, record.output_file, record.status_file
        )
    }

    fn status(&self, job_id: &str, wait_ms: u64) -> String {
        let clean_id = job_id.trim();
        if clean_id.is_empty() {
            return "Action result: shell_job_status\nerror: job_id_required".to_string();
        }
        let Some(record) = self.find(clean_id) else {
            return format!(
                "Action result: shell_job_status\njob_id: {}\nerror: job_not_found",
                clean_id
            );
        };
        let wait = Duration::from_millis(wait_ms.min(15000));
        let started = Instant::now();
        loop {
            if let Some(code) = fs::read_to_string(&record.status_file)
                .ok()
                .map(|text| text.trim().to_string())
                .filter(|text| !text.is_empty())
            {
                let output = fs::read_to_string(&record.output_file).unwrap_or_default();
                return format!(
                    "Action result: shell_job_status\njob_id: {}\nstate: finished\nexit_code: {}\nwaited_ms: {}\noutput_file: {}\noutput:\n{}",
                    record.id,
                    code,
                    started.elapsed().as_millis(),
                    record.output_file,
                    compact_text(&output, 4000)
                );
            }
            if started.elapsed() >= wait {
                let output = fs::read_to_string(&record.output_file).unwrap_or_default();
                return format!(
                    "Action result: shell_job_status\njob_id: {}\nstate: running\npid: {}\nwaited_ms: {}\noutput_file: {}\npartial_output:\n{}",
                    record.id,
                    record.pid,
                    started.elapsed().as_millis(),
                    record.output_file,
                    compact_text(&output, 2000)
                );
            }
            thread::sleep(Duration::from_millis(200));
        }
    }

    fn append(&self, record: &ShellJobRecord) -> std::io::Result<()> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.index_file)?;
        writeln!(
            file,
            "{}",
            serde_json::to_string(record).unwrap_or_default()
        )
    }

    fn find(&self, job_id: &str) -> Option<ShellJobRecord> {
        let file = OpenOptions::new().read(true).open(&self.index_file).ok()?;
        let mut found = None;
        for line in BufReader::new(file).lines().map_while(Result::ok) {
            let Ok(record) = serde_json::from_str::<ShellJobRecord>(&line) else {
                continue;
            };
            if record.id == job_id {
                found = Some(record);
            }
        }
        found
    }
}

#[derive(Debug, Clone)]
struct FileChatHistoryStore {
    audit_file: PathBuf,
}
impl FileChatHistoryStore {
    fn new(memory_dir: &Path) -> Self {
        let audit_file = memory_dir
            .parent()
            .unwrap_or(memory_dir)
            .join("api_audit.jsonl");
        Self { audit_file }
    }

    fn query(
        &self,
        query: &str,
        limit: usize,
        after_ms: Option<i64>,
        before_ms: Option<i64>,
    ) -> std::io::Result<Vec<ChatHistoryRecord>> {
        let terms = search_terms(query);
        let mut rows = self.read_all()?;
        rows.retain(|record| time_in_window(record.started_at_ms, after_ms, before_ms));
        if !terms.is_empty() {
            rows.retain(|record| chat_record_matches(record, &terms));
        }
        rows.sort_by(|a, b| b.started_at_ms.cmp(&a.started_at_ms));
        rows.truncate(limit.max(1).min(50));
        Ok(rows)
    }

    fn delete(
        &self,
        id: &str,
        query: &str,
        limit: usize,
        after_ms: Option<i64>,
        before_ms: Option<i64>,
    ) -> Result<usize, String> {
        let clean_id = id.trim();
        let targets = if clean_id.is_empty() {
            self.query(query, limit, after_ms, before_ms)
                .map_err(|_| "chat_history_read_failed".to_string())?
                .into_iter()
                .map(|record| record.turn_id)
                .collect::<HashSet<_>>()
        } else {
            let mut ids = HashSet::new();
            ids.insert(clean_id.to_string());
            ids
        };
        if targets.is_empty() {
            return Ok(0);
        }
        let file = match OpenOptions::new().read(true).open(&self.audit_file) {
            Ok(file) => file,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(_) => return Err("chat_history_read_failed".to_string()),
        };
        let mut retained = Vec::new();
        let mut deleted_turn_ids = HashSet::new();
        for line in BufReader::new(file).lines().map_while(Result::ok) {
            let turn_id = serde_json::from_str::<Value>(&line)
                .ok()
                .and_then(|value| {
                    value
                        .get("turn_id")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .unwrap_or_default();
            if !turn_id.is_empty() && targets.contains(&turn_id) {
                deleted_turn_ids.insert(turn_id);
                continue;
            }
            retained.push(line);
        }
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&self.audit_file)
            .map_err(|_| "chat_history_write_failed".to_string())?;
        for line in retained {
            writeln!(file, "{}", line).map_err(|_| "chat_history_write_failed".to_string())?;
        }
        Ok(deleted_turn_ids.len())
    }

    fn read_all(&self) -> std::io::Result<Vec<ChatHistoryRecord>> {
        let file = match OpenOptions::new().read(true).open(&self.audit_file) {
            Ok(file) => file,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => return Err(err),
        };
        let mut rows = Vec::<ChatHistoryRecord>::new();
        for line in BufReader::new(file).lines().map_while(Result::ok) {
            let Ok(value) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            let event_type = value.get("type").and_then(Value::as_str).unwrap_or("");
            let turn_id = value
                .get("turn_id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim();
            if turn_id.is_empty() {
                continue;
            }
            match event_type {
                "turn_start" => {
                    let user_input = value
                        .get("user_input")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .trim();
                    if user_input.is_empty() {
                        continue;
                    }
                    rows.push(ChatHistoryRecord {
                        session: value
                            .get("session")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        turn_id: turn_id.to_string(),
                        started_at_ms: turn_id_millis(turn_id)
                            .or_else(|| value.get("created_at").and_then(Value::as_i64))
                            .unwrap_or_default(),
                        user_input: user_input.to_string(),
                        assistant_output: String::new(),
                    });
                }
                "turn_final" => {
                    let assistant_output = value
                        .get("assistant_output")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .trim();
                    if assistant_output.is_empty() {
                        continue;
                    }
                    if let Some(existing) = rows.iter_mut().rev().find(|row| row.turn_id == turn_id)
                    {
                        existing.assistant_output = assistant_output.to_string();
                    } else {
                        rows.push(ChatHistoryRecord {
                            session: value
                                .get("session")
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .to_string(),
                            turn_id: turn_id.to_string(),
                            started_at_ms: turn_id_millis(turn_id)
                                .or_else(|| value.get("created_at").and_then(Value::as_i64))
                                .unwrap_or_default(),
                            user_input: String::new(),
                            assistant_output: assistant_output.to_string(),
                        });
                    }
                }
                _ => {}
            }
        }
        Ok(rows
            .into_iter()
            .filter(|row| {
                !row.user_input.trim().is_empty() || !row.assistant_output.trim().is_empty()
            })
            .collect())
    }
}

fn validate_memory_sql(sql: &str) -> Result<(), String> {
    let trimmed = sql.trim();
    if trimmed.is_empty() {
        return Err("empty_sql".to_string());
    }
    let lowered = trimmed.to_lowercase();
    let lowered = lowered.trim_end_matches(';').trim().to_string();
    let first_keyword = lowered.split_whitespace().next().unwrap_or("");
    if !matches!(first_keyword, "select" | "with" | "pragma") {
        return Err("read_only_sql_required".to_string());
    }
    if lowered.contains(';') {
        return Err("semicolon_not_allowed".to_string());
    }
    if lowered.contains("sqlite_") || lowered.contains("sqlite_master") {
        return Err("only_declared_tables_are_allowed".to_string());
    }
    let tokens = lowered
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();
    let forbidden = [
        "insert", "update", "delete", "alter", "drop", "attach", "detach", "replace", "create",
        "vacuum", "reindex", "analyze", "truncate",
    ];
    if tokens.iter().any(|token| forbidden.contains(token)) {
        return Err("write_or_ddl_not_allowed".to_string());
    }
    if first_keyword == "pragma" {
        let compact = lowered.split_whitespace().collect::<String>();
        if compact == "pragmatable_info(memories)"
            || compact == "pragmatable_info('memories')"
            || compact == "pragmatable_info(\"memories\")"
            || compact == "pragmatable_info(chat_messages)"
            || compact == "pragmatable_info('chat_messages')"
            || compact == "pragmatable_info(\"chat_messages\")"
        {
            return Ok(());
        }
        return Err("only_declared_tables_are_allowed".to_string());
    }
    let allowed_read = lowered.contains(" from memories")
        || lowered.contains(" from chat_messages")
        || lowered.contains(" join memories")
        || lowered.contains(" join chat_messages")
        || lowered.contains(" from (select");
    if !allowed_read {
        return Err("only_declared_tables_are_allowed".to_string());
    }
    Ok(())
}

fn render_delta_slices(delta: &PromptDelta) -> Vec<PromptSlice> {
    delta
        .slices
        .iter()
        .filter(|slice| !delta.hidden_slice_ids.contains(&slice.slice_id))
        .cloned()
        .collect()
}

fn split_text_for_prompt_slices(text: &str, limit: usize) -> Vec<String> {
    let safe_limit = limit.max(1);
    if text.len() <= safe_limit {
        return vec![text.to_string()];
    }
    let mut chunks = Vec::new();
    let mut start = 0;
    while start < text.len() {
        let mut end = (start + safe_limit).min(text.len());
        while end > start && !text.is_char_boundary(end) {
            end -= 1;
        }
        if end == start {
            end = text[start..]
                .char_indices()
                .nth(1)
                .map(|(idx, _)| start + idx)
                .unwrap_or(text.len());
        }
        chunks.push(text[start..end].to_string());
        start = end;
    }
    chunks
}

fn estimate_prompt_tokens(text: &str) -> u32 {
    text.chars().count().div_ceil(4).min(u32::MAX as usize) as u32
}

fn clamp_durable_ctx_score(raw: u64) -> u8 {
    raw.clamp(1, 10) as u8
}

fn durable_score_label(score: Option<u8>) -> String {
    score
        .map(|value| value.to_string())
        .unwrap_or_else(|| "unscored".to_string())
}

fn repair_failure_message(first_issue: &str, final_issue: &str) -> String {
    if first_issue == "truncated_model_output" || final_issue == "truncated_model_output" {
        return "模型回复被 API 提供商按最大输出 token 限制截断（例如 stop_reason=max_tokens），导致返回的 JSON 协议不完整。请调大 TIMEM_MAX_LLM_OUTPUT，或在交互提示中选择增加 10K 后重试。".to_string();
    }
    format!(
        "模型的回复不符合本地协议，已拦截原始报文展示。原因：{final_issue}。请重试或换一个更具体的问题。"
    )
}

fn shell_quote_path(path: &Path) -> String {
    let raw = path.to_string_lossy();
    format!("'{}'", raw.replace('\'', "'\\''"))
}

fn parse_envelope(content: &str) -> ParsedEnvelope {
    let value: Value = match parse_json_value_from_model_text(content) {
        Ok(value) => value,
        Err(_) => {
            return ParsedEnvelope {
                response_to_user: String::new(),
                thought: String::new(),
                next_actions: vec![],
                memory_candidates: vec![],
                delta_scores: Vec::new(),
                repair_issue: Some("invalid_json".to_string()),
            }
        }
    };
    if !value.is_object() {
        return ParsedEnvelope {
            response_to_user: String::new(),
            thought: String::new(),
            next_actions: vec![],
            memory_candidates: vec![],
            delta_scores: Vec::new(),
            repair_issue: Some("root_must_be_json_object".to_string()),
        };
    }
    let mut repair_issue: Option<String> = None;
    let response_to_user = match value.get("response_to_user").and_then(Value::as_str) {
        Some(text) => text.to_string(),
        None => {
            repair_issue = Some("response_to_user_required".to_string());
            String::new()
        }
    };
    let thought = value
        .get("thought")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(ToString::to_string)
        .unwrap_or_default();
    let mut delta_scores = parse_delta_scores(value.get("delta_scores"));
    if let Some(score) = value
        .get("durable_ctx_score")
        .and_then(Value::as_u64)
        .map(clamp_durable_ctx_score)
    {
        delta_scores.push(DeltaScore {
            delta_id: value
                .get("scored_delta_id")
                .or_else(|| value.get("delta_id"))
                .and_then(Value::as_str)
                .map(|id| id.trim().to_string())
                .filter(|id| !id.is_empty()),
            durable_ctx_score: score,
        });
    }
    let acceptance_satisfied = value
        .get("acceptance_check")
        .and_then(|check| check.get("is_satisfied"))
        .and_then(Value::as_bool);
    let mut next_actions = Vec::new();
    if let Some(next_actions_value) = value.get("next_actions") {
        if let Some(actions) = next_actions_value.as_array() {
            for (idx, action) in actions.iter().enumerate() {
                let name = action
                    .get("action")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if name.is_empty() {
                    repair_issue = Some(format!("next_actions[{idx}].action_missing"));
                    break;
                }
                let input = action.get("input").unwrap_or(action);
                let intent = action
                    .get("intent")
                    .or_else(|| input.get("intent"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim();
                if intent.is_empty() {
                    repair_issue = Some(format!("next_actions[{idx}].intent_required"));
                    break;
                }
                let query = input
                    .get("query")
                    .or_else(|| action.get("query"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_string();
                let content = input
                    .get("content")
                    .or_else(|| action.get("content"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_string();
                let sql = input
                    .get("sql")
                    .or_else(|| action.get("sql"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_string();
                let params = input
                    .get("params")
                    .or_else(|| action.get("params"))
                    .and_then(Value::as_array)
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(json_sql_param_to_string)
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                let operation = input
                    .get("operation")
                    .or_else(|| action.get("operation"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_string();
                let id = input
                    .get("id")
                    .or_else(|| action.get("id"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_string();
                let command = input
                    .get("command")
                    .or_else(|| action.get("command"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_string();
                let read_back_command = input
                    .get("read_back_command")
                    .or_else(|| action.get("read_back_command"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_string();
                let large_readback_opt_in = input
                    .get("large_readback_opt_in")
                    .or_else(|| action.get("large_readback_opt_in"))
                    .is_some();
                let background = input
                    .get("background")
                    .or_else(|| action.get("background"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                    || input
                        .get("mode")
                        .or_else(|| action.get("mode"))
                        .and_then(Value::as_str)
                        .is_some_and(|mode| mode.trim().eq_ignore_ascii_case("background"));
                let job_id = input
                    .get("job_id")
                    .or_else(|| action.get("job_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_string();
                let delta_ids = input
                    .get("delta_ids")
                    .or_else(|| action.get("delta_ids"))
                    .and_then(Value::as_array)
                    .map(|items| json_string_array(items))
                    .unwrap_or_default();
                let slice_ids = input
                    .get("slice_ids")
                    .or_else(|| action.get("slice_ids"))
                    .and_then(Value::as_array)
                    .map(|items| json_string_array(items))
                    .unwrap_or_default();
                let timeout_ms_raw = input
                    .get("timeout_ms")
                    .or_else(|| action.get("timeout_ms"))
                    .and_then(Value::as_u64);
                let timeout_sec_raw = input
                    .get("timeout_sec")
                    .or_else(|| action.get("timeout_sec"))
                    .and_then(Value::as_u64);
                let timeout_ms_provided = timeout_ms_raw.is_some() || timeout_sec_raw.is_some();
                let after_ms = input
                    .get("after_ms")
                    .or_else(|| action.get("after_ms"))
                    .and_then(json_i64);
                let before_ms = input
                    .get("before_ms")
                    .or_else(|| action.get("before_ms"))
                    .and_then(json_i64);
                let normalized_name = name.as_str();
                match normalized_name {
                    "chat_history_query" => {}
                    "chat_history_delete" => {
                        if id.is_empty() && query.is_empty() {
                            repair_issue =
                                Some(format!("next_actions[{idx}].input.id_or_query_required"));
                            break;
                        }
                    }
                    "query_memory" | "memory_query" => {
                        if query.is_empty() {
                            repair_issue =
                                Some(format!("next_actions[{idx}].input.query_required"));
                            break;
                        }
                    }
                    "memory_schema" => {}
                    "memory_write" | "write_memory" => {
                        if content.is_empty() && query.is_empty() {
                            repair_issue =
                                Some(format!("next_actions[{idx}].input.content_required"));
                            break;
                        }
                    }
                    "memory_update" => {
                        if operation.trim().is_empty() {
                            repair_issue =
                                Some(format!("next_actions[{idx}].input.operation_required"));
                            break;
                        }
                        if matches!(operation.as_str(), "insert" | "upsert" | "update")
                            && content.is_empty()
                        {
                            repair_issue =
                                Some(format!("next_actions[{idx}].input.content_required"));
                            break;
                        }
                        if matches!(operation.as_str(), "delete" | "update") && id.is_empty() {
                            repair_issue = Some(format!("next_actions[{idx}].input.id_required"));
                            break;
                        }
                    }
                    "scratch_write" => {
                        if content.is_empty() {
                            repair_issue =
                                Some(format!("next_actions[{idx}].input.content_required"));
                            break;
                        }
                    }
                    "scratch_query" => {}
                    "scratch_delete" => {
                        if id.is_empty() {
                            repair_issue = Some(format!("next_actions[{idx}].input.id_required"));
                            break;
                        }
                    }
                    "prompt_shrink" => {
                        if delta_ids.is_empty() && slice_ids.is_empty() {
                            repair_issue = Some(format!("next_actions[{idx}].input.ids_required"));
                            break;
                        }
                    }
                    "shell_job_status" => {
                        if job_id.is_empty() {
                            repair_issue =
                                Some(format!("next_actions[{idx}].input.job_id_required"));
                            break;
                        }
                        if !timeout_ms_provided {
                            repair_issue =
                                Some(format!("next_actions[{idx}].input.timeout_ms_required"));
                            break;
                        }
                    }
                    "run_bash" => {
                        if command.is_empty() && read_back_command.is_empty() {
                            repair_issue =
                                Some(format!("next_actions[{idx}].input.command_required"));
                            break;
                        }
                    }
                    "sql_read" | "memory_sql_query" => {
                        if sql.is_empty() {
                            repair_issue = Some(format!("next_actions[{idx}].input.sql_required"));
                            break;
                        }
                        let placeholder_count = sql.matches('?').count();
                        if params.len() != placeholder_count {
                            repair_issue = Some(format!(
                                "next_actions[{idx}].input.params_count_mismatch expected={placeholder_count} actual={}",
                                params.len()
                            ));
                            break;
                        }
                    }
                    _ => {
                        repair_issue =
                            Some(format!("next_actions[{idx}].unsupported_action:{name}"));
                        break;
                    }
                }
                let parsed_timeout_ms = timeout_ms_raw
                    .or_else(|| timeout_sec_raw.map(|seconds| seconds.saturating_mul(1000)));
                let timeout_ms = if normalized_name == "shell_job_status" {
                    parsed_timeout_ms.unwrap_or(0).min(15000)
                } else {
                    parsed_timeout_ms.unwrap_or(5000).clamp(1000, 15000)
                };
                next_actions.push(ParsedAction {
                    action: name,
                    intent: intent.to_string(),
                    query,
                    content,
                    sql,
                    params,
                    operation,
                    id,
                    command,
                    read_back_command,
                    large_readback_opt_in,
                    background,
                    job_id,
                    delta_ids,
                    slice_ids,
                    timeout_ms,
                    limit: input
                        .get("limit")
                        .or_else(|| action.get("limit"))
                        .and_then(Value::as_u64)
                        .unwrap_or(5) as usize,
                    after_ms,
                    before_ms,
                });
            }
        } else if !next_actions_value.is_null() {
            repair_issue = Some("next_actions_must_be_array".to_string());
        }
    }
    let mut memory_candidates = Vec::new();
    if let Some(candidates_value) = value.get("memory_candidates") {
        if let Some(candidates) = candidates_value.as_array() {
            for candidate in candidates {
                if let Some(text) = candidate.as_str().map(str::trim).filter(|x| !x.is_empty()) {
                    memory_candidates.push(text.to_string());
                    continue;
                }
                for key in ["content", "fact", "summary", "memory", "text", "title"] {
                    if let Some(text) = candidate
                        .get(key)
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|x| !x.is_empty())
                    {
                        memory_candidates.push(text.to_string());
                        break;
                    }
                }
            }
        } else if !candidates_value.is_null() {
            repair_issue =
                repair_issue.or_else(|| Some("memory_candidates_must_be_array".to_string()));
        }
    }
    if repair_issue.is_none() && response_to_user.trim().is_empty() && next_actions.is_empty() {
        repair_issue = Some("empty_response_to_user_and_no_next_actions".to_string());
    }
    if repair_issue.is_none() && next_actions.is_empty() && acceptance_satisfied.is_none() {
        repair_issue = Some("acceptance_check.is_satisfied_required".to_string());
    }
    if repair_issue.is_none() && acceptance_satisfied == Some(false) {
        let missing_info_ok = value
            .get("acceptance_check")
            .and_then(|check| check.get("missing_info"))
            .and_then(Value::as_array)
            .is_some_and(|items| !items.is_empty());
        if !missing_info_ok {
            repair_issue = Some("acceptance_check.missing_info_required".to_string());
        } else if next_actions.is_empty() {
            repair_issue = Some("next_actions_required_when_unsatisfied".to_string());
        }
    }
    ParsedEnvelope {
        response_to_user,
        thought,
        next_actions,
        memory_candidates,
        delta_scores,
        repair_issue,
    }
}

fn parse_delta_scores(value: Option<&Value>) -> Vec<DeltaScore> {
    let Some(value) = value else {
        return Vec::new();
    };
    let Some(items) = value.as_array() else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let score = item
                .get("durable_ctx_score")
                .or_else(|| item.get("score"))
                .and_then(Value::as_u64)
                .map(clamp_durable_ctx_score)?;
            Some(DeltaScore {
                delta_id: item
                    .get("delta_id")
                    .and_then(Value::as_str)
                    .map(|id| id.trim().to_string())
                    .filter(|id| !id.is_empty()),
                durable_ctx_score: score,
            })
        })
        .collect()
}

fn parse_json_value_from_model_text(content: &str) -> Result<Value, serde_json::Error> {
    let trimmed = content.trim();
    if let Ok(value) = serde_json::from_str(trimmed) {
        return Ok(value);
    }
    if let Some(repaired) = repair_known_string_field_quotes(trimmed) {
        if let Ok(value) = serde_json::from_str(&repaired) {
            return Ok(value);
        }
    }
    let mut last_ok = None;
    for (idx, ch) in trimmed.char_indices() {
        if ch != '{' {
            continue;
        }
        let candidate = &trimmed[idx..];
        if let Ok(value) = serde_json::from_str(candidate) {
            if is_likely_response_envelope(&value) {
                last_ok = Some(value);
            }
        }
        if let Some(repaired) = repair_known_string_field_quotes(candidate) {
            if let Ok(value) = serde_json::from_str(&repaired) {
                if is_likely_response_envelope(&value) {
                    last_ok = Some(value);
                }
            }
        }
        if let Some(object_text) = extract_balanced_json_object(candidate) {
            if let Ok(value) = serde_json::from_str(&object_text) {
                if is_likely_response_envelope(&value) {
                    last_ok = Some(value);
                }
            }
            if let Some(repaired) = repair_known_string_field_quotes(&object_text) {
                if let Ok(value) = serde_json::from_str(&repaired) {
                    if is_likely_response_envelope(&value) {
                        last_ok = Some(value);
                    }
                }
            }
        }
    }
    if let Some(value) = last_ok {
        Ok(value)
    } else {
        serde_json::from_str(trimmed)
    }
}

fn is_likely_response_envelope(value: &Value) -> bool {
    value.as_object().is_some_and(|object| {
        object.contains_key("response_to_user") || object.contains_key("next_actions")
    })
}

fn extract_balanced_json_object(input: &str) -> Option<String> {
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (idx, ch) in input.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
                continue;
            }
            match ch {
                '\\' => escaped = true,
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '{' => depth = depth.saturating_add(1),
            '}' => {
                if depth == 0 {
                    return None;
                }
                depth -= 1;
                if depth == 0 {
                    return Some(input[..idx + ch.len_utf8()].to_string());
                }
            }
            _ => {}
        }
    }
    None
}

fn repair_known_string_field_quotes(input: &str) -> Option<String> {
    let mut output = input.to_string();
    let mut changed = false;
    for key in [
        "response_to_user",
        "thought",
        "intent",
        "query",
        "content",
        "command",
        "sql",
    ] {
        let (next, key_changed) = repair_unescaped_quotes_for_key(&output, key);
        output = next;
        changed |= key_changed;
    }
    changed.then_some(output)
}

fn repair_unescaped_quotes_for_key(input: &str, key: &str) -> (String, bool) {
    let marker = format!("\"{key}\"");
    let bytes = input.as_bytes();
    let mut output = String::with_capacity(input.len());
    let mut pos = 0;
    let mut changed = false;
    while let Some(rel) = input[pos..].find(&marker) {
        let marker_start = pos + rel;
        output.push_str(&input[pos..marker_start]);
        output.push_str(&marker);
        let mut cursor = marker_start + marker.len();
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            output.push(bytes[cursor] as char);
            cursor += 1;
        }
        if cursor >= bytes.len() || bytes[cursor] != b':' {
            pos = cursor;
            continue;
        }
        output.push(':');
        cursor += 1;
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            output.push(bytes[cursor] as char);
            cursor += 1;
        }
        if cursor >= bytes.len() || bytes[cursor] != b'"' {
            pos = cursor;
            continue;
        }
        output.push('"');
        cursor += 1;
        let value_start = cursor;
        let mut segment = String::new();
        let mut ended = false;
        while cursor < input.len() {
            let Some(ch) = input[cursor..].chars().next() else {
                break;
            };
            let ch_len = ch.len_utf8();
            if ch == '\\' {
                segment.push(ch);
                cursor += ch_len;
                if cursor < input.len() {
                    if let Some(next_ch) = input[cursor..].chars().next() {
                        segment.push(next_ch);
                        cursor += next_ch.len_utf8();
                    }
                }
                continue;
            }
            if ch == '"' {
                let next = next_non_ws_char(input, cursor + ch_len);
                if matches!(next, Some(',') | Some('}') | Some(']') | None) {
                    output.push_str(&segment);
                    output.push('"');
                    cursor += ch_len;
                    ended = true;
                    break;
                }
                output.push_str(&segment);
                output.push('\\');
                output.push('"');
                segment.clear();
                cursor += ch_len;
                changed = true;
                continue;
            }
            segment.push(ch);
            cursor += ch_len;
        }
        if !ended {
            output.push_str(&input[value_start..cursor]);
        }
        pos = cursor;
    }
    output.push_str(&input[pos..]);
    (output, changed)
}

fn next_non_ws_char(input: &str, mut pos: usize) -> Option<char> {
    while pos < input.len() {
        let ch = input[pos..].chars().next()?;
        if !ch.is_whitespace() {
            return Some(ch);
        }
        pos += ch.len_utf8();
    }
    None
}

fn search_terms(query: &str) -> Vec<String> {
    let lowered = query.to_lowercase();
    let mut seen = HashSet::new();
    let mut terms = Vec::new();
    for token in lowered.split(|c: char| !c.is_alphanumeric()) {
        push_search_term(token.trim(), &mut seen, &mut terms);
    }
    terms
}

fn push_search_term(token: &str, seen: &mut HashSet<String>, terms: &mut Vec<String>) {
    if token.is_empty() || !seen.insert(token.to_string()) {
        return;
    }
    terms.push(token.to_string());
    if token
        .chars()
        .all(|c| ('\u{4e00}'..='\u{9fff}').contains(&c))
        && token.chars().count() >= 4
    {
        let chars: Vec<char> = token.chars().collect();
        for pair in chars.windows(2) {
            let gram = pair.iter().collect::<String>();
            if seen.insert(gram.clone()) {
                terms.push(gram);
            }
        }
    }
}

fn turn_id_millis(turn_id: &str) -> Option<i64> {
    turn_id
        .strip_prefix("turn_")
        .and_then(|value| value.parse::<i64>().ok())
}

fn chat_record_matches(record: &ChatHistoryRecord, terms: &[String]) -> bool {
    let haystack = format!(
        "{} {} {} {}",
        record.session, record.turn_id, record.user_input, record.assistant_output
    )
    .to_lowercase();
    terms.iter().any(|term| haystack.contains(term))
}

fn time_in_window(time_ms: i64, after_ms: Option<i64>, before_ms: Option<i64>) -> bool {
    after_ms.is_none_or(|after| time_ms >= after) && before_ms.is_none_or(|before| time_ms < before)
}

fn json_i64(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_u64().and_then(|raw| i64::try_from(raw).ok()))
}

fn json_string_array(items: &[Value]) -> Vec<String> {
    items
        .iter()
        .filter_map(|value| {
            value
                .as_str()
                .map(ToString::to_string)
                .or_else(|| value.as_i64().map(|raw| raw.to_string()))
                .or_else(|| value.as_u64().map(|raw| raw.to_string()))
        })
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
        .collect()
}

fn json_sql_param_to_string(value: &Value) -> Option<String> {
    if let Some(text) = value.as_str() {
        return Some(text.to_string());
    }
    if let Some(num) = value.as_i64() {
        return Some(num.to_string());
    }
    if let Some(num) = value.as_u64() {
        return Some(num.to_string());
    }
    if let Some(num) = value.as_f64() {
        return Some(num.to_string());
    }
    value.as_bool().map(|flag| flag.to_string())
}

fn should_run_memory_precheck(supporting_context: &str) -> bool {
    supporting_context.contains("memory_lookup_hint:")
}
fn execute_guarded_bash(
    command: &str,
    read_back_command: &str,
    large_readback_opt_in: bool,
    background: bool,
    timeout_ms: u64,
    approval_mode: BashApprovalMode,
    intent: &str,
    shell_jobs: &FileShellJobStore,
) -> ActionExecution {
    let primary_command = command.trim();
    let read_back = read_back_command.trim();
    let command_to_run = if primary_command.is_empty() {
        read_back
    } else {
        primary_command
    };
    if let Err(reason) = validate_bash_request(command_to_run) {
        return ActionExecution::Completed(format!(
            "Action result: run_bash\ncommand: {}\nerror: {}",
            command_to_run, reason
        ));
    }
    if !background && !read_back.is_empty() && read_back != command_to_run {
        if let Err(reason) = validate_bash_request(read_back) {
            return ActionExecution::Completed(format!(
                "Action result: run_bash\ncommand: {}\nerror: {}",
                read_back, reason
            ));
        }
    }
    if approval_mode == BashApprovalMode::Ask {
        return ActionExecution::NeedsApproval(PendingApproval {
            request: ApprovalRequest {
                approval_id: format!("approval_{}", now_ms()),
                action: "run_bash".to_string(),
                command: command_to_run.to_string(),
                read_back_command: read_back.to_string(),
                reason: "run_bash_requires_user_approval".to_string(),
                risk: "local_shell_command".to_string(),
                intent: intent.to_string(),
            },
            command: command_to_run.to_string(),
            read_back_command: read_back.to_string(),
            large_readback_opt_in,
            background,
            timeout_ms,
            intent: intent.to_string(),
        });
    }
    if background {
        return ActionExecution::Completed(shell_jobs.spawn(command_to_run));
    }
    let mut result = execute_one_bash(command_to_run, timeout_ms);
    if !background && !read_back.is_empty() && read_back != command_to_run {
        result.push_str("\n\n");
        result.push_str("Read-back result:\n");
        result.push_str(&execute_one_bash(read_back, timeout_ms));
    }
    if large_readback_opt_in {
        result.push_str(
            "\nread_back_policy: unbounded_v1_requested_but_native_output_is_still_bounded",
        );
    }
    ActionExecution::Completed(result)
}

fn execute_approved_bash(
    command: &str,
    read_back_command: &str,
    large_readback_opt_in: bool,
    background: bool,
    timeout_ms: u64,
    request: &ApprovalRequest,
    shell_jobs: &FileShellJobStore,
) -> String {
    let primary_command = command.trim();
    let read_back = read_back_command.trim();
    let command_to_run = if primary_command.is_empty() {
        read_back
    } else {
        primary_command
    };
    let mut result = if background {
        shell_jobs.spawn(command_to_run)
    } else {
        execute_one_bash(command_to_run, timeout_ms)
    };
    if !read_back.is_empty() && read_back != command_to_run {
        result.push_str("\n\n");
        result.push_str("Read-back result:\n");
        result.push_str(&execute_one_bash(read_back, timeout_ms));
    }
    if large_readback_opt_in {
        result.push_str(
            "\nread_back_policy: unbounded_v1_requested_but_native_output_is_still_bounded",
        );
    }
    result.push_str(&format!(
        "\napproval_id: {}\napproval_status: approved_by_user",
        request.approval_id
    ));
    result
}

fn execute_one_bash(command: &str, timeout_ms: u64) -> String {
    let spawn = Command::new("/bin/sh")
        .arg("-lc")
        .arg(command)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match spawn {
        Ok(child) => child,
        Err(_) => {
            return format!(
                "Action result: run_bash\ncommand: {}\nerror: command_failed",
                command
            )
        }
    };
    let started = std::time::Instant::now();
    let timeout = Duration::from_millis(timeout_ms.max(1000).min(15000));
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return format!(
                    "Action result: run_bash\ncommand: {}\nerror: timeout",
                    command
                );
            }
            Ok(None) => thread::sleep(Duration::from_millis(50)),
            Err(_) => {
                return format!(
                    "Action result: run_bash\ncommand: {}\nerror: command_failed",
                    command
                )
            }
        }
    }
    match child.wait_with_output() {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let mut combined = String::new();
            if !stdout.trim().is_empty() {
                combined.push_str(stdout.trim_end());
            }
            if !stderr.trim().is_empty() {
                if !combined.is_empty() {
                    combined.push('\n');
                }
                combined.push_str("stderr: ");
                combined.push_str(stderr.trim_end());
            }
            if combined.is_empty() {
                combined = "<no output>".to_string();
            }
            format!(
                "Action result: run_bash\ncommand: {}\nstatus: {}\noutput:\n{}",
                command,
                output.status.code().unwrap_or(-1),
                compact_text(&combined, 4000)
            )
        }
        Err(_) => format!(
            "Action result: run_bash\ncommand: {}\nerror: command_failed",
            command
        ),
    }
}
fn validate_bash_request(command: &str) -> Result<(), String> {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return Err("command_required".to_string());
    }
    if trimmed.len() > 2000 {
        return Err("command_too_long".to_string());
    }
    Ok(())
}

fn compact_text(text: &str, max_chars: usize) -> String {
    let mut out = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if out.chars().count() > max_chars {
        out = out.chars().take(max_chars).collect::<String>();
        out.push('…');
    }
    out
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn unique_id(prefix: &str) -> String {
    let seq = ID_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{}_{}_{}", prefix, now_ms(), seq)
}

#[derive(Debug, Deserialize)]
struct FfiCoreConfig {
    static_prompt: String,
    memory_dir: String,
    profile: CoreProfile,
}

#[derive(Debug, Deserialize)]
struct FfiLlmResponse {
    content: String,
    model_name: Option<String>,
    usage: Option<UsageStats>,
}

pub struct AgentCoreHandle {
    core: AgentCore,
}

#[no_mangle]
pub extern "C" fn timem_core_new(config_json: *const c_char) -> *mut AgentCoreHandle {
    let Some(config_text) = read_c_string(config_json) else {
        return std::ptr::null_mut();
    };
    let Ok(config) = serde_json::from_str::<FfiCoreConfig>(&config_text) else {
        return std::ptr::null_mut();
    };
    Box::into_raw(Box::new(AgentCoreHandle {
        core: AgentCore::new(config.static_prompt, config.profile, config.memory_dir),
    }))
}

#[no_mangle]
pub extern "C" fn timem_core_begin_turn(
    handle: *mut AgentCoreHandle,
    user_input: *const c_char,
    supporting_context: *const c_char,
) -> *mut c_char {
    let Some(handle) = handle_mut(handle) else {
        return json_string(json_error("null_handle"));
    };
    let Some(input) = read_c_string(user_input) else {
        return json_string(json_error("null_user_input"));
    };
    let context = read_c_string(supporting_context);
    json_string(step_to_json(
        handle.core.begin_turn(&input, context.as_deref()),
    ))
}

#[no_mangle]
pub extern "C" fn timem_core_apply_model_response(
    handle: *mut AgentCoreHandle,
    response_json: *const c_char,
) -> *mut c_char {
    let Some(handle) = handle_mut(handle) else {
        return json_string(json_error("null_handle"));
    };
    let Some(response_text) = read_c_string(response_json) else {
        return json_string(json_error("null_response"));
    };
    let response = match serde_json::from_str::<FfiLlmResponse>(&response_text) {
        Ok(value) => LlmResponse {
            content: value.content,
            model_name: value
                .model_name
                .unwrap_or_else(|| handle.core.profile.model.clone()),
            usage: value.usage.unwrap_or_else(UsageStats::zero),
            truncated: false,
        },
        Err(err) => return json_string(json_error(&format!("invalid_response_json:{err}"))),
    };
    json_string(step_to_json(handle.core.apply_model_response(response)))
}

#[no_mangle]
pub extern "C" fn timem_core_resolve_user_approval(
    handle: *mut AgentCoreHandle,
    approval_id: *const c_char,
    approved: bool,
) -> *mut c_char {
    let Some(handle) = handle_mut(handle) else {
        return json_string(json_error("null_handle"));
    };
    let Some(approval_id) = read_c_string(approval_id) else {
        return json_string(json_error("null_approval_id"));
    };
    json_string(step_to_json(
        handle
            .core
            .resolve_user_approval(approval_id.trim(), approved),
    ))
}

#[no_mangle]
pub extern "C" fn timem_core_continue_after_round_limit(
    handle: *mut AgentCoreHandle,
) -> *mut c_char {
    let Some(handle) = handle_mut(handle) else {
        return json_string(json_error("null_handle"));
    };
    json_string(step_to_json(handle.core.continue_after_round_limit()))
}

#[no_mangle]
pub extern "C" fn timem_core_free(handle: *mut AgentCoreHandle) {
    if !handle.is_null() {
        unsafe {
            drop(Box::from_raw(handle));
        }
    }
}

#[no_mangle]
pub extern "C" fn timem_core_free_string(value: *mut c_char) {
    if !value.is_null() {
        unsafe {
            drop(CString::from_raw(value));
        }
    }
}

#[no_mangle]
pub extern "C" fn timem_core_version() -> *mut c_char {
    json_string(serde_json::json!({"agent_core":"rust","version":env!("CARGO_PKG_VERSION")}))
}

fn read_c_string(value: *const c_char) -> Option<String> {
    if value.is_null() {
        return None;
    }
    unsafe { CStr::from_ptr(value).to_str().ok().map(ToString::to_string) }
}

fn handle_mut<'a>(handle: *mut AgentCoreHandle) -> Option<&'a mut AgentCoreHandle> {
    if handle.is_null() {
        None
    } else {
        unsafe { handle.as_mut() }
    }
}

fn json_string(value: serde_json::Value) -> *mut c_char {
    let text = serde_json::to_string(&value)
        .unwrap_or_else(|_| "{\"ok\":false,\"error\":\"encode_failed\"}".to_string());
    CString::new(text)
        .unwrap_or_else(|_| CString::new("{\"ok\":false,\"error\":\"nul_byte\"}").unwrap())
        .into_raw()
}

fn json_error(error: &str) -> serde_json::Value {
    serde_json::json!({"ok":false,"error":error})
}

fn step_to_json(step: CoreStep) -> serde_json::Value {
    match step {
        CoreStep::NeedModel {
            prompt,
            rounds_remaining,
        } => serde_json::json!({
            "ok": true,
            "step": "need_model",
            "prompt": prompt,
            "rounds_remaining": rounds_remaining
        }),
        CoreStep::NeedsUserApproval { request } => serde_json::json!({
            "ok": true,
            "step": "needs_user_approval",
            "approval": request
        }),
        CoreStep::RoundLimitReached { max_rounds } => serde_json::json!({
            "ok": true,
            "step": "round_limit_reached",
            "max_rounds": max_rounds
        }),
        CoreStep::Final(turn) => serde_json::json!({
            "ok": true,
            "step": "final",
            "response_to_user": turn.response_to_user,
            "stats": turn.stats,
            "profile_label": turn.profile_label
        }),
    }
}
