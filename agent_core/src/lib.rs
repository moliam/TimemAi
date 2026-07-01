use rusqlite::{params_from_iter, types::ValueRef, Connection};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::ffi::{CStr, CString};
use std::fs::{self, OpenOptions};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::{BufRead, BufReader, Write};
use std::os::raw::c_char;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub mod capability;
pub mod capmgr;
use capability::CapabilityRegistry;
pub mod executor;
pub mod memmgr;
pub mod prompt_spec;
pub mod shell_exec;
use shell_exec::FileShellJobStore;
pub use shell_exec::ShellJobRecord;

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
    slices: Vec<PromptSlice>,
    #[serde(default)]
    pub hidden_slice_ids: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct PromptSlice {
    delta_id: String,
    slice_id: String,
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
    #[serde(default)]
    pub updated_at_ms: i64,
    #[serde(default)]
    pub version: u64,
    pub content: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScratchNoteRecord {
    pub id: String,
    pub created_at_ms: i64,
    pub scratch_type: String,
    pub label: String,
    pub content: String,
    pub prompt_delta_ids: Vec<String>,
    pub prompt_slice_ids: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
struct ScratchContextOffload {
    content: String,
    delta_ids: Vec<String>,
    slice_ids: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatHistoryRecord {
    pub session: String,
    pub turn_id: String,
    pub started_at_ms: i64,
    pub user_input: String,
    pub assistant_output: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedAction {
    action: String,
    intent: String,
    raw_input: Value,
    mem_type: String,
    op: String,
    query: String,
    content: String,
    scratch_type: String,
    label: String,
    sql: String,
    params: Vec<String>,
    operation: String,
    expected_version: Option<u64>,
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
    expect: String,
    expect_timeout_ms: u64,
}
impl ParsedAction {
    fn audit_input(&self) -> Value {
        let mut input = match self.action.as_str() {
            "capmgr" => json!({
                "op": self.op,
                "kind": self.scratch_type,
                "id": self.id,
            }),
            "memmgr" => json!({
                "type": self.mem_type,
                "op": self.op,
                "query": self.query,
                "content": self.content,
                "kind": self.scratch_type,
                "label": self.label,
                "sql": self.sql,
                "params": self.params,
                "operation": self.operation,
                "expected_version": self.expected_version,
                "id": self.id,
                "delta_ids": self.delta_ids,
                "slice_ids": self.slice_ids,
                "limit": self.limit,
                "after_ms": self.after_ms,
                "before_ms": self.before_ms,
            }),
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
                "expected_version": self.expected_version,
                "content": self.content,
            }),
            "memory_write" | "write_memory" => json!({
                "content": self.content,
                "query": self.query,
            }),
            "scratch_write" => json!({
                "type": self.scratch_type,
                "label": self.label,
                "content": self.content,
                "delta_ids": self.delta_ids,
                "slice_ids": self.slice_ids,
            }),
            "scratch_read" => json!({
                "id": self.id,
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
        };
        if !self.expect.is_empty() {
            if let Some(object) = input.as_object_mut() {
                object.insert("expect".to_string(), json!(self.expect));
                object.insert(
                    "expect_timeout_ms".to_string(),
                    json!(self.expect_timeout_ms),
                );
            }
        }
        input
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedEnvelope {
    report_job_progress: String,
    continue_work: bool,
    continue_was_implicit: bool,
    thought: String,
    thought_durable: bool,
    next_actions: Vec<ParsedAction>,
    memory_candidates: Vec<String>,
    repair_issue: Option<String>,
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
const DEFAULT_ROUND_BUDGET: u32 = 50;
const MEM_GUARD_WAIT_STEP: Duration = Duration::from_millis(25);
const MEM_GUARD_TIMEOUT: Duration = Duration::from_secs(30);
const MEM_GUARD_STALE_AFTER: Duration = Duration::from_secs(60 * 60 * 6);

#[derive(Debug, Clone)]
pub struct MemGuard {
    lock_dir: PathBuf,
}

impl MemGuard {
    pub fn for_memory_dir(memory_dir: impl AsRef<Path>) -> Self {
        let space_dir = space_dir_for_memory_dir(memory_dir.as_ref()).to_path_buf();
        Self::for_space_dir(space_dir)
    }

    pub fn for_space_dir(space_dir: impl AsRef<Path>) -> Self {
        let space_dir = fs::canonicalize(space_dir.as_ref())
            .unwrap_or_else(|_| space_dir.as_ref().to_path_buf());
        Self {
            lock_dir: space_dir.join(".guard").join("mem.lock.d"),
        }
    }

    pub fn for_audit_file(path: impl AsRef<Path>) -> Self {
        let path = path.as_ref();
        let space_dir = path
            .parent()
            .and_then(|parent| {
                if parent.file_name().and_then(|name| name.to_str()) == Some("audit") {
                    parent.parent()
                } else {
                    Some(parent)
                }
            })
            .unwrap_or_else(|| Path::new("."));
        Self::for_space_dir(space_dir)
    }

    pub fn with_read<T>(&self, f: impl FnOnce() -> T) -> Result<T, String> {
        self.with_lock(f)
    }

    pub fn with_write<T>(&self, f: impl FnOnce() -> T) -> Result<T, String> {
        self.with_lock(f)
    }

    fn with_lock<T>(&self, f: impl FnOnce() -> T) -> Result<T, String> {
        let _lock = self.acquire()?;
        Ok(f())
    }

    fn acquire(&self) -> Result<MemGuardLock, String> {
        if let Some(parent) = self.lock_dir.parent() {
            fs::create_dir_all(parent).map_err(|_| "mem_guard_create_failed".to_string())?;
        }
        let started = Instant::now();
        loop {
            match fs::create_dir(&self.lock_dir) {
                Ok(()) => {
                    let owner = json!({
                        "pid": std::process::id(),
                        "created_at_ms": now_ms(),
                    });
                    let _ = fs::write(
                        self.lock_dir.join("owner.json"),
                        serde_json::to_string_pretty(&owner).unwrap_or_default(),
                    );
                    return Ok(MemGuardLock {
                        lock_dir: self.lock_dir.clone(),
                    });
                }
                Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                    if self.is_stale_lock() {
                        let _ = fs::remove_dir_all(&self.lock_dir);
                        continue;
                    }
                    if started.elapsed() >= MEM_GUARD_TIMEOUT {
                        return Err("mem_guard_timeout".to_string());
                    }
                    thread::sleep(MEM_GUARD_WAIT_STEP);
                }
                Err(_) => return Err("mem_guard_lock_failed".to_string()),
            }
        }
    }

    fn is_stale_lock(&self) -> bool {
        fs::metadata(&self.lock_dir)
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .map(|age| age >= MEM_GUARD_STALE_AFTER)
            .unwrap_or(false)
    }
}

#[derive(Debug)]
struct MemGuardLock {
    lock_dir: PathBuf,
}

impl Drop for MemGuardLock {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.lock_dir);
    }
}

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
    guard: MemGuard,
}

impl FileActionAuditStore {
    fn new(memory_dir: &Path) -> Self {
        let space_dir = space_dir_for_memory_dir(memory_dir);
        let file = space_dir.join("audit").join("action_audit.json");
        Self {
            file,
            guard: MemGuard::for_memory_dir(memory_dir),
        }
    }

    fn begin_turn(&self, turn_id: &str, started_at_ms: i64, user_question: &str) {
        let _ = self.guard.with_write(|| {
            let mut doc = self.read_doc_unlocked();
            if doc.turns.iter().any(|turn| turn.turn_id == turn_id) {
                return;
            }
            doc.turns.push(ActionAuditTurn {
                turn_id: turn_id.to_string(),
                started_at_ms,
                user_question: user_question.to_string(),
                interactions: Vec::new(),
            });
            self.write_doc_unlocked(&doc);
        });
    }

    fn record_action(&self, entry: ActionAuditEntry, turn_id: &str, user_question: &str) {
        let _ = self.guard.with_write(|| {
            let mut doc = self.read_doc_unlocked();
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
            self.write_doc_unlocked(&doc);
        });
    }

    fn read_doc_unlocked(&self) -> ActionAuditDocument {
        let Ok(text) = fs::read_to_string(&self.file) else {
            return Self::empty_doc();
        };
        serde_json::from_str(&text).unwrap_or_else(|_| Self::empty_doc())
    }

    fn write_doc_unlocked(&self, doc: &ActionAuditDocument) {
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

fn space_dir_for_memory_dir(memory_dir: &Path) -> &Path {
    if memory_dir.file_name().and_then(|name| name.to_str()) == Some("memory") {
        memory_dir.parent().unwrap_or(memory_dir)
    } else {
        memory_dir
    }
}

#[derive(Debug)]
pub struct AgentCore {
    static_prompt: String,
    profile: CoreProfile,
    capabilities: CapabilityRegistry,
    memory: FileMemoryStore,
    scratch: FileScratchStore,
    chat_history: FileChatHistoryStore,
    shell_jobs: FileShellJobStore,
    action_audit: FileActionAuditStore,
    deltas: Vec<PromptDelta>,
    max_llm_input_tokens: u32,
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
            capabilities: CapabilityRegistry::builtin(),
            memory: FileMemoryStore::new(memory_dir),
            scratch: FileScratchStore::new(memory_dir),
            chat_history: FileChatHistoryStore::new(memory_dir),
            shell_jobs: FileShellJobStore::new(memory_dir),
            action_audit: FileActionAuditStore::new(memory_dir),
            deltas: Vec::new(),
            max_llm_input_tokens: 100_000,
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
    pub fn set_capability_registry(&mut self, capabilities: CapabilityRegistry) {
        self.capabilities = capabilities;
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
                "The previous model output was cut off by the max output token limit before a complete JSON object was produced. Return one short valid JSON object only. If the task needs a long report, use run_bash to write the full report to a file and keep report_job_progress concise.",
            );
        }
        let parsed = parse_envelope(&response.content, &self.capabilities);
        let mut slices = Vec::new();
        if !parsed.thought.is_empty() && parsed.thought_durable {
            slices.push((
                "llm_thought".to_string(),
                format!("Thought:\n{}", parsed.thought),
            ));
        }
        if let Some(issue) = parsed.repair_issue.clone() {
            if !self.repair_attempted {
                return self.request_protocol_repair(
                    &issue,
                    "Return exactly one valid JSON object with report_job_progress and continue. Do not use markdown fences.",
                );
            }
            if issue == "invalid_json"
                && can_show_plain_text_after_repair_failure(&response.content)
            {
                let final_text = response.content.trim().to_string();
                slices.push((
                    "llm_response".to_string(),
                    format!("Response shown to user:\n{}", final_text),
                ));
                self.append_delta(slices);
                return CoreStep::Final(TurnFinal {
                    response_to_user: final_text,
                    stats: self.current_stats.clone(),
                    profile_label: self.profile.label(),
                    repair_issue: Some("invalid_json_plain_text_fallback".to_string()),
                });
            }
            let final_text = if parsed.report_job_progress.trim().is_empty() {
                repair_failure_message(self.last_repair_issue.as_deref().unwrap_or(&issue), &issue)
            } else {
                parsed.report_job_progress
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
        if !parsed.continue_work {
            if parsed.next_actions.is_empty() {
                for candidate in parsed.memory_candidates {
                    if self.memory.write(&candidate).is_ok() {
                        self.current_stats.tool_calls += 1;
                        self.current_stats.mem_writes += 1;
                    }
                }
                let final_text = parsed.report_job_progress.trim().to_string();
                slices.push((
                    "llm_response".to_string(),
                    format!("Response shown to user:\n{}", final_text),
                ));
                self.append_delta(slices);
                return CoreStep::Final(TurnFinal {
                    response_to_user: final_text,
                    stats: self.current_stats.clone(),
                    profile_label: self.profile.label(),
                    repair_issue: None,
                });
            }
            let pending_final_text = parsed.report_job_progress.trim().to_string();
            let last_idx = parsed.next_actions.len() - 1;
            let expect_cmd = parsed.next_actions[last_idx].expect.clone();
            let expect_timeout = parsed.next_actions[last_idx].expect_timeout_ms;
            if !pending_final_text.is_empty() {
                slices.push((
                    "llm_progress".to_string(),
                    format!("Job progress shown to user:\n{}", pending_final_text),
                ));
            }
            let mut result_lines: Vec<String> = Vec::new();
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
            let expect_body = match self.run_guarded_finalize_expect(&expect_cmd, expect_timeout) {
                ActionExecution::Completed(result) => result,
                ActionExecution::NeedsApproval(pending) => {
                    self.append_delta(slices);
                    let request = pending.request.clone();
                    self.pending_approval = Some(pending);
                    return CoreStep::NeedsUserApproval { request };
                }
            };
            let pass = expect_check_passed(&expect_body);
            slices.push(("result_of_llm_action".to_string(), expect_body));
            if pass {
                for candidate in parsed.memory_candidates {
                    if self.memory.write(&candidate).is_ok() {
                        self.current_stats.tool_calls += 1;
                        self.current_stats.mem_writes += 1;
                    }
                }
                slices.push((
                    "llm_response".to_string(),
                    format!("Response shown to user:\n{}", pending_final_text),
                ));
                self.append_delta(slices);
                return CoreStep::Final(TurnFinal {
                    response_to_user: pending_final_text,
                    stats: self.current_stats.clone(),
                    profile_label: self.profile.label(),
                    repair_issue: None,
                });
            }
            slices.push((
                "runtime_note".to_string(),
                "Note: 你上轮用 continue:false + expect 声明完成，但 expect 命令 exit!=0。请根据以上证据修正后再回复。".to_string(),
            ));
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

        if !parsed.report_job_progress.trim().is_empty() {
            slices.push((
                "llm_progress".to_string(),
                format!(
                    "Job progress shown to user:\n{}",
                    parsed.report_job_progress.trim()
                ),
            ));
        }
        if parsed.continue_was_implicit {
            slices.push((
                "runtime_note".to_string(),
                "Note: 上轮回复没有写 continue，runtime 已默认按 continue=true 处理。以后最好明确给出 continue。"
                    .to_string(),
            ));
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
        let final_text = parsed.report_job_progress.trim().to_string();
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
        let static_prompt = prompt_spec::enrich_static_prompt_with_response_schema(
            &self.capabilities.enrich_static_prompt(&self.static_prompt),
        );
        let mut out = format!(
            "[BEGIN SEGMENT 0: prompt_0]\n{}\n[END SEGMENT 0: prompt_0]",
            static_prompt
        );
        let slices = self.render_prompt_slices();
        for (idx, slice) in slices.iter().enumerate() {
            out.push('\n');
            out.push_str(&format!("[BEGIN SEGMENT {}: prompt_delta]\n", idx + 1));
            out.push_str(&format!(
                "delta_id: {}\nslice_id: {}\nslice: {}/{}\n",
                slice.delta_id, slice.slice_id, slice.slice_index, slice.slice_count
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
            slices,
            hidden_slice_ids: Vec::new(),
        });
    }
    fn consume_shrink_review_if_needed(&mut self, incoming_prompt_tokens: u32) -> Option<String> {
        let estimated_prompt_tokens = self.estimate_rendered_prompt_tokens(incoming_prompt_tokens);
        let force_threshold = self.max_llm_input_tokens.saturating_mul(90) / 100;
        let slices = self.render_prompt_slices();
        if slices.is_empty() {
            return None;
        }
        let dynamic_tokens = slices
            .iter()
            .map(|slice| estimate_prompt_tokens(&slice.text))
            .sum::<u32>();
        if estimated_prompt_tokens < force_threshold {
            return None;
        }
        let excess_tokens = estimated_prompt_tokens.saturating_sub(force_threshold);
        let practical_shrink_capacity = dynamic_tokens.saturating_mul(8) / 10;
        if practical_shrink_capacity < excess_tokens {
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
                    "- delta_id={} time_ms={} visible_slices={} estimated_tokens={}",
                    delta.delta_id,
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
                    "- slice_id={} delta_id={} slice={}/{} prompt_type={} time_ms={}",
                    slice.slice_id,
                    slice.delta_id,
                    slice.slice_index,
                    slice.slice_count,
                    slice.prompt_type,
                    slice.time_ms
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        let instruction = "Context is above 90% of the configured input window. You must compact before continuing: summarize all dynamic prompt deltas into about 10%-20% of their current token footprint, discard useless/stale details, preserve only active work-relevant state, use memmgr type=scratch op=write kind=context_offload for important but lengthy existing delta/slice content or kind=notes for compact checkpoints, then use memmgr type=context op=shrink on covered delta_id/slice_id ranges. Do not target prompt_0.";
        Some(format!(
            "mode=force_shrink_required\nestimated_prompt_tokens={estimated_prompt_tokens}\nmax_llm_input_tokens={}\nforce_shrink_threshold_tokens={force_threshold}\ntarget_dynamic_context_ratio=10%-20%\nprompt_delta_count={current_count}\nprompt_slice_count={slice_count}\nrecent_prompt_delta_refs:\n{delta_refs}\nrecent_prompt_slice_refs:\n{recent_refs}\n{instruction}",
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
        let executor_target = match executor::resolve_action(&self.capabilities, &action.action) {
            Ok(target) => target,
            Err(err) => {
                let result = format!("Action result: {}\nerror: {}", action.action, err);
                self.record_action_audit(&action_for_audit, "completed", Some(&result));
                return ActionExecution::Completed(result);
            }
        };
        if let executor::ExecutorTarget::Command { path, .. } = &executor_target {
            let result = self.execute_command_capability(&action, path);
            self.record_action_audit(&action_for_audit, "completed", Some(&result));
            return ActionExecution::Completed(result);
        }
        let dispatch_name = match &executor_target {
            executor::ExecutorTarget::Builtin { binding_name } => binding_name.as_str(),
            executor::ExecutorTarget::Legacy { action } => action.as_str(),
            executor::ExecutorTarget::Command { .. } => {
                unreachable!("command target returned early")
            }
        };
        if !matches!(
            dispatch_name,
            "capmgr" | "memmgr" | "run_bash" | "shell_job_status"
        ) {
            if let Some(result) = self.execute_legacy_memmgr_action(&action, dispatch_name) {
                self.record_action_audit(&action_for_audit, "completed", Some(&result));
                return ActionExecution::Completed(result);
            }
        }
        let result = match dispatch_name {
            "capmgr" => self.execute_capmgr_action(&action),
            "memmgr" => self.execute_memmgr_action(&action),
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

    fn execute_legacy_memmgr_action(
        &mut self,
        action: &ParsedAction,
        dispatch_name: &str,
    ) -> Option<String> {
        let mut canonical = action.clone();
        canonical.action = "memmgr".to_string();
        match dispatch_name {
            "chat_history_query" => {
                canonical.mem_type = "raw_chat".to_string();
                canonical.op = "query".to_string();
                Some(rewrite_memmgr_result_header(
                    self.execute_memmgr_action(&canonical),
                    "chat_history_query",
                ))
            }
            "chat_history_delete" => {
                canonical.mem_type = "raw_chat".to_string();
                canonical.op = "delete".to_string();
                Some(rewrite_memmgr_result_header(
                    self.execute_memmgr_action(&canonical),
                    "chat_history_delete",
                ))
            }
            "query_memory" | "memory_query" => {
                canonical.mem_type = "durable".to_string();
                canonical.op = "query".to_string();
                Some(rewrite_memmgr_result_header(
                    self.execute_memmgr_action(&canonical),
                    "query_memory",
                ))
            }
            "memory_schema" => {
                canonical.mem_type = "durable".to_string();
                canonical.op = "schema".to_string();
                Some(rewrite_memmgr_result_header(
                    self.execute_memmgr_action(&canonical),
                    "memory_schema",
                ))
            }
            "memory_write" | "write_memory" => {
                self.current_stats.tool_calls += 1;
                let content = if action.content.trim().is_empty() {
                    action.query.clone()
                } else {
                    action.content.clone()
                };
                if content.trim().is_empty() {
                    Some("Action result: memory_write\nskipped: empty content".to_string())
                } else if self.memory.write(&content).is_ok() {
                    self.current_stats.mem_writes += 1;
                    Some(format!("Action result: memory_write\nstored: {}", content))
                } else {
                    Some("Action result: memory_write\nerror: write_failed".to_string())
                }
            }
            "memory_update" => {
                self.current_stats.tool_calls += 1;
                Some(
                    match self.memory.update(
                        &action.operation,
                        &action.id,
                        &action.content,
                        action.expected_version,
                    ) {
                        Ok(result) => {
                            self.current_stats.mem_writes += 1;
                            result
                        }
                        Err(err) => format!("Action result: memory_update\nerror: {}", err),
                    },
                )
            }
            "sql_read" | "memory_sql_query" => {
                canonical.mem_type = "durable".to_string();
                canonical.op = "sql".to_string();
                Some(rewrite_memmgr_result_header(
                    self.execute_memmgr_action(&canonical),
                    dispatch_name,
                ))
            }
            "scratch_write" => {
                canonical.mem_type = "scratch".to_string();
                canonical.op = "write".to_string();
                Some(rewrite_memmgr_result_header(
                    self.execute_memmgr_action(&canonical),
                    "scratch_write",
                ))
            }
            "scratch_read" => {
                canonical.mem_type = "scratch".to_string();
                canonical.op = "read".to_string();
                Some(rewrite_memmgr_result_header(
                    self.execute_memmgr_action(&canonical),
                    "scratch_read",
                ))
            }
            "scratch_query" => {
                canonical.mem_type = "scratch".to_string();
                canonical.op = "query".to_string();
                Some(rewrite_memmgr_result_header(
                    self.execute_memmgr_action(&canonical),
                    "scratch_query",
                ))
            }
            "scratch_delete" => {
                canonical.mem_type = "scratch".to_string();
                canonical.op = "delete".to_string();
                Some(rewrite_memmgr_result_header(
                    self.execute_memmgr_action(&canonical),
                    "scratch_delete",
                ))
            }
            "prompt_shrink" => {
                canonical.mem_type = "context".to_string();
                canonical.op = "shrink".to_string();
                Some(rewrite_memmgr_result_header(
                    self.execute_memmgr_action(&canonical),
                    "prompt_shrink",
                ))
            }
            _ => None,
        }
    }

    fn execute_memmgr_action(&mut self, action: &ParsedAction) -> String {
        self.current_stats.tool_calls += 1;
        match (action.mem_type.as_str(), action.op.as_str()) {
            ("durable", "query") => {
                self.current_stats.mem_reads += 1;
                let rows = self
                    .memory
                    .query(&action.query, action.limit)
                    .unwrap_or_default();
                if rows.is_empty() {
                    format!(
                        "Action result: memmgr\ntype: durable\nop: query\nquery: {}\nresults: none",
                        action.query
                    )
                } else {
                    let lines = rows
                        .into_iter()
                        .map(|r| {
                            format!(
                                "- id={} version={} created_at_ms={} updated_at_ms={} content={}",
                                r.id, r.version, r.created_at_ms, r.updated_at_ms, r.content
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    format!(
                        "Action result: memmgr\ntype: durable\nop: query\nquery: {}\nresults:\n{}",
                        action.query, lines
                    )
                }
            }
            ("durable", "schema") => {
                self.current_stats.mem_reads += 1;
                self.memory.schema_text(&self.chat_history)
                    .replacen("Action result: memory_schema", "Action result: memmgr\ntype: durable\nop: schema", 1)
            }
            ("durable", "sql") | ("raw_chat", "sql") => {
                self.current_stats.mem_reads += 1;
                match self.memory.sql_read(
                    &self.chat_history,
                    &action.sql,
                    &action.params,
                    action.limit,
                ) {
                    Ok(rows) if rows.is_empty() => {
                        format!(
                            "Action result: memmgr\ntype: {}\nop: sql\nsql: {}\nresults: none",
                            action.mem_type, action.sql
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
                            "Action result: memmgr\ntype: {}\nop: sql\nsql: {}\nresults:\n{}",
                            action.mem_type, action.sql, lines
                        )
                    }
                    Err(err) => format!(
                        "Action result: memmgr\ntype: {}\nop: sql\nsql: {}\nerror: {}",
                        action.mem_type, action.sql, err
                    ),
                }
            }
            ("durable", "insert" | "update" | "upsert" | "delete") => {
                match self.memory.update(
                    &action.op,
                    &action.id,
                    &action.content,
                    action.expected_version,
                ) {
                    Ok(result) => {
                        self.current_stats.mem_writes += 1;
                        result
                            .replacen("Action result: memory_update", "Action result: memmgr\ntype: durable", 1)
                    }
                    Err(err) => format!("Action result: memmgr\ntype: durable\nop: {}\nerror: {}", action.op, err),
                }
            }
            ("raw_chat", "query") => {
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
                        "Action result: memmgr\ntype: raw_chat\nop: query\nquery: {}\nresults: none",
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
                        "Action result: memmgr\ntype: raw_chat\nop: query\nquery: {}\nresults:\n{}",
                        action.query,
                        sections.join("\n")
                    )
                }
            }
            ("raw_chat", "delete") => match self.chat_history.delete(
                &action.id,
                &action.query,
                action.limit,
                action.after_ms,
                action.before_ms,
            ) {
                Ok(deleted) => format!(
                    "Action result: memmgr\ntype: raw_chat\nop: delete\nid: {}\nquery: {}\ndeleted_count: {}",
                    action.id, action.query, deleted
                ),
                Err(err) => format!("Action result: memmgr\ntype: raw_chat\nop: delete\nerror: {}", err),
            },
            ("scratch", "write") => {
                let scratch_type = memmgr::normalize_scratch_kind(&action.scratch_type);
                let write_result = if scratch_type == "context_offload" {
                    self.collect_prompt_context_for_scratch(&action.delta_ids, &action.slice_ids)
                        .and_then(|offload| {
                            self.scratch.write_record(
                                &scratch_type,
                                &action.label,
                                &offload.content,
                                &offload.delta_ids,
                                &offload.slice_ids,
                            )
                        })
                } else {
                    self.scratch.write_record(
                        &scratch_type,
                        &action.label,
                        &action.content,
                        &[],
                        &[],
                    )
                };
                match write_result {
                    Ok(record) => format_scratch_write_result(&record)
                        .replacen("Action result: scratch_write", "Action result: memmgr\ntype: scratch\nop: write", 1),
                    Err(err) => format!("Action result: memmgr\ntype: scratch\nop: write\nerror: {}", err),
                }
            }
            ("scratch", "read") => match self.scratch.read(&action.id) {
                Ok(Some(record)) => format_scratch_read_result(&record)
                    .replacen("Action result: scratch_read", "Action result: memmgr\ntype: scratch\nop: read", 1),
                Ok(None) => format!(
                    "Action result: memmgr\ntype: scratch\nop: read\nid: {}\nfound: false",
                    action.id
                ),
                Err(err) => format!("Action result: memmgr\ntype: scratch\nop: read\nerror: {}", err),
            },
            ("scratch", "query") => match self.scratch.query(&action.query, action.limit) {
                Ok(rows) if rows.is_empty() => format!(
                    "Action result: memmgr\ntype: scratch\nop: query\nquery: {}\nresults: none",
                    action.query
                ),
                Ok(rows) => {
                    let lines = rows
                        .into_iter()
                        .map(|row| {
                            format!(
                                "- id={} label={} type={} time_ms={} content_preview={}",
                                row.id,
                                scratch_label_for_display(&row),
                                memmgr::normalize_scratch_kind(&row.scratch_type),
                                row.created_at_ms,
                                compact_text(&row.content, 240)
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    format!(
                        "Action result: memmgr\ntype: scratch\nop: query\nquery: {}\nresults:\n{}",
                        action.query, lines
                    )
                }
                Err(err) => format!("Action result: memmgr\ntype: scratch\nop: query\nerror: {}", err),
            },
            ("scratch", "delete") => match self.scratch.delete(&action.id) {
                Ok(true) => format!(
                    "Action result: memmgr\ntype: scratch\nop: delete\nid: {}\ndeleted: true",
                    action.id
                ),
                Ok(false) => format!(
                    "Action result: memmgr\ntype: scratch\nop: delete\nid: {}\ndeleted: false",
                    action.id
                ),
                Err(err) => format!("Action result: memmgr\ntype: scratch\nop: delete\nerror: {}", err),
            },
            ("context", "shrink") => self
                .apply_prompt_shrink(&action.delta_ids, &action.slice_ids)
                .replacen("Action result: prompt_shrink", "Action result: memmgr\ntype: context\nop: shrink", 1),
            _ => format!(
                "Action result: memmgr\ntype: {}\nop: {}\nerror: unsupported_type_or_op",
                action.mem_type, action.op
            ),
        }
    }

    fn execute_capmgr_action(&mut self, action: &ParsedAction) -> String {
        self.current_stats.tool_calls += 1;
        capmgr::execute(
            &self.capabilities,
            capmgr::CapmgrActionInput {
                op: &action.op,
                kind: &action.scratch_type,
                id: &action.id,
            },
        )
    }

    fn execute_command_capability(&mut self, action: &ParsedAction, path: &Path) -> String {
        self.current_stats.tool_calls += 1;
        let payload = json!({
            "action": action.action,
            "intent": action.intent,
            "input": action.raw_input,
        });
        executor::execute_command_action(&action.action, path, &payload, action.timeout_ms)
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

    fn run_guarded_finalize_expect(&mut self, command: &str, timeout_ms: u64) -> ActionExecution {
        let timeout_ms = timeout_ms.clamp(1000, 15_000);
        let execution = execute_guarded_bash(
            command,
            "",
            false,
            false,
            timeout_ms,
            self.bash_approval_mode,
            "Verify final answer before showing it.",
            &self.shell_jobs,
        );
        match execution {
            ActionExecution::Completed(result) => {
                let body = format_expect_check_result(command, &result);
                let status = if expect_check_passed(&body) {
                    "guarded_finalize_expect_pass"
                } else {
                    "guarded_finalize_expect_fail"
                };
                self.record_guarded_finalize_audit(command, timeout_ms, status, Some(&body));
                ActionExecution::Completed(body)
            }
            ActionExecution::NeedsApproval(pending) => {
                let summary = format!(
                    "Expect check:\ncommand: {}\nstatus: needs_user_approval\napproval_id: {}\nverdict: PENDING",
                    command, pending.request.approval_id
                );
                self.record_guarded_finalize_audit(
                    command,
                    timeout_ms,
                    "guarded_finalize_expect_needs_user_approval",
                    Some(&summary),
                );
                ActionExecution::NeedsApproval(pending)
            }
        }
    }

    fn record_guarded_finalize_audit(
        &self,
        command: &str,
        timeout_ms: u64,
        status: &str,
        result: Option<&str>,
    ) {
        let turn_id = self
            .current_action_turn_id
            .as_deref()
            .unwrap_or("unknown_turn");
        self.action_audit.record_action(
            ActionAuditEntry {
                time_ms: now_ms(),
                round: self.current_round.max(1),
                action: "guarded_finalize_expect".to_string(),
                intent: "Verify final answer before showing it.".to_string(),
                status: status.to_string(),
                input: json!({
                    "command": command,
                    "timeout_ms": timeout_ms,
                }),
                result_summary: result.map(|text| compact_text(text, 2_000)),
            },
            turn_id,
            &self.current_action_user_question,
        );
    }

    fn collect_prompt_context_for_scratch(
        &self,
        delta_ids: &[String],
        slice_ids: &[String],
    ) -> Result<ScratchContextOffload, String> {
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
        if delta_id_set.is_empty() && slice_id_set.is_empty() {
            return Err("delta_ids_or_slice_ids_required".to_string());
        }
        if delta_id_set.contains("prompt_0") || slice_id_set.contains("prompt_0") {
            return Err("prompt_0_not_allowed".to_string());
        }

        let existing_delta_ids = self
            .deltas
            .iter()
            .map(|delta| delta.delta_id.clone())
            .collect::<HashSet<_>>();
        let mut matched_delta_ids = HashSet::new();
        let mut matched_slice_ids = HashSet::new();
        let mut sections = Vec::new();
        for delta in &self.deltas {
            let rendered = render_delta_slices(delta);
            if delta_id_set.contains(&delta.delta_id) {
                matched_delta_ids.insert(delta.delta_id.clone());
                sections.push(format!(
                    "[BEGIN SCRATCH OFFLOAD DELTA {} time_ms={}]",
                    delta.delta_id, delta.time_ms
                ));
                for slice in rendered {
                    matched_slice_ids.insert(slice.slice_id.clone());
                    sections.push(format_prompt_slice_for_scratch(&slice));
                }
                sections.push(format!("[END SCRATCH OFFLOAD DELTA {}]", delta.delta_id));
                continue;
            }
            for slice in rendered {
                if slice_id_set.contains(&slice.slice_id) {
                    matched_slice_ids.insert(slice.slice_id.clone());
                    sections.push(format_prompt_slice_for_scratch(&slice));
                }
            }
        }

        let mut missing = delta_id_set
            .difference(&existing_delta_ids)
            .cloned()
            .collect::<Vec<_>>();
        for id in slice_id_set {
            if !matched_slice_ids.contains(&id) {
                missing.push(id);
            }
        }
        missing.sort();
        missing.dedup();
        if !missing.is_empty() {
            return Err(format!(
                "invalid_prompt_refs missing_ids={}",
                missing.join(",")
            ));
        }
        if sections.is_empty() {
            return Err("no_visible_prompt_context_to_offload".to_string());
        }

        let mut matched_delta_ids = matched_delta_ids.into_iter().collect::<Vec<_>>();
        matched_delta_ids.sort();
        let mut matched_slice_ids = matched_slice_ids.into_iter().collect::<Vec<_>>();
        matched_slice_ids.sort();
        Ok(ScratchContextOffload {
            content: sections.join("\n"),
            delta_ids: matched_delta_ids,
            slice_ids: matched_slice_ids,
        })
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
        if shrunk_tokens_estimate > 0 {
            self.last_observed_prompt_tokens = 0;
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
    guard: MemGuard,
}
impl FileMemoryStore {
    fn new(dir: &Path) -> Self {
        let _ = fs::create_dir_all(dir);
        Self {
            dir: dir.to_path_buf(),
            file: dir.join("memory.jsonl"),
            guard: MemGuard::for_memory_dir(dir),
        }
    }
    fn write(&self, content: &str) -> std::io::Result<()> {
        let clean = content.trim();
        if clean.is_empty() {
            return Ok(());
        }
        self.guard
            .with_write(|| self.write_clean_unlocked(clean))
            .map_err(std::io::Error::other)?
    }

    fn write_clean_unlocked(&self, clean: &str) -> std::io::Result<()> {
        let time_ms = now_ms();
        let record = MemoryRecord {
            id: unique_id("mem"),
            created_at_ms: time_ms,
            updated_at_ms: time_ms,
            version: 1,
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
        self.guard
            .with_read(|| self.query_unlocked(query, limit))
            .map_err(std::io::Error::other)?
    }

    fn query_unlocked(&self, query: &str, limit: usize) -> std::io::Result<Vec<MemoryRecord>> {
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
                let record = normalize_memory_record(record);
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
        self.guard
            .with_read(|| {
                let mut rows = self.read_all_unlocked()?;
                rows.sort_by(|a, b| b.created_at_ms.cmp(&a.created_at_ms));
                rows.truncate(limit.max(1).min(50));
                Ok(rows)
            })
            .map_err(std::io::Error::other)?
    }

    fn update(
        &self,
        operation: &str,
        id: &str,
        content: &str,
        expected_version: Option<u64>,
    ) -> Result<String, String> {
        self.guard
            .with_write(|| self.update_unlocked(operation, id, content, expected_version))
            .map_err(|err| err.to_string())?
    }

    fn update_unlocked(
        &self,
        operation: &str,
        id: &str,
        content: &str,
        expected_version: Option<u64>,
    ) -> Result<String, String> {
        let op = operation.trim().to_lowercase();
        match op.as_str() {
            "insert" | "upsert" if id.trim().is_empty() => {
                let clean = content.trim();
                if clean.is_empty() {
                    return Err("content_required".to_string());
                }
                self.write_clean_unlocked(clean)
                    .map_err(|_| "write_failed".to_string())?;
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
                    .read_all_unlocked()
                    .map_err(|_| "memory_read_failed".to_string())?;
                let mut found = false;
                for row in &mut rows {
                    if row.id == clean_id {
                        if let Some(expected) = expected_version {
                            if row.version != expected {
                                return Err(memory_conflict_result(
                                    clean_id,
                                    expected,
                                    row.version,
                                    &row.content,
                                ));
                            }
                        } else {
                            return Err(memory_missing_expected_version_result(
                                clean_id,
                                row.version,
                                &row.content,
                            ));
                        }
                        row.content = clean.to_string();
                        row.updated_at_ms = now_ms();
                        row.version = row.version.saturating_add(1).max(1);
                        found = true;
                        break;
                    }
                }
                if !found {
                    if expected_version.is_some() && op == "update" {
                        return Err("id_not_found".to_string());
                    }
                    let time_ms = now_ms();
                    rows.push(MemoryRecord {
                        id: clean_id.to_string(),
                        created_at_ms: time_ms,
                        updated_at_ms: time_ms,
                        version: 1,
                        content: clean.to_string(),
                    });
                }
                self.write_all_unlocked(&rows)
                    .map_err(|_| "write_failed".to_string())?;
                Ok(format!(
                    "Action result: memory_update\noperation: {}\nid: {}\nversion: {}\nstored: {}",
                    if found { "update" } else { "insert" },
                    clean_id,
                    rows.iter()
                        .find(|row| row.id == clean_id)
                        .map(|row| row.version)
                        .unwrap_or(1),
                    clean
                ))
            }
            "delete" => {
                let clean_id = id.trim();
                if clean_id.is_empty() {
                    return Err("id_required".to_string());
                }
                let mut rows = self
                    .read_all_unlocked()
                    .map_err(|_| "memory_read_failed".to_string())?;
                let before = rows.len();
                if let Some(row) = rows.iter().find(|row| row.id == clean_id) {
                    if let Some(expected) = expected_version {
                        if row.version != expected {
                            return Err(memory_conflict_result(
                                clean_id,
                                expected,
                                row.version,
                                &row.content,
                            ));
                        }
                    } else {
                        return Err(memory_missing_expected_version_result(
                            clean_id,
                            row.version,
                            &row.content,
                        ));
                    }
                }
                rows.retain(|row| row.id != clean_id);
                if rows.len() == before {
                    return Err("id_not_found".to_string());
                }
                self.write_all_unlocked(&rows)
                    .map_err(|_| "write_failed".to_string())?;
                Ok(format!(
                    "Action result: memory_update\noperation: delete\nid: {}\ndeleted: true",
                    clean_id
                ))
            }
            _ => Err("operation_must_be_insert_update_upsert_or_delete".to_string()),
        }
    }

    fn write_all_unlocked(&self, rows: &[MemoryRecord]) -> std::io::Result<()> {
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

    fn read_all_unlocked(&self) -> std::io::Result<Vec<MemoryRecord>> {
        let file = match OpenOptions::new().read(true).open(&self.file) {
            Ok(file) => file,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => return Err(err),
        };
        let mut rows = Vec::new();
        for line in BufReader::new(file).lines().map_while(Result::ok) {
            if let Ok(record) = serde_json::from_str::<MemoryRecord>(&line) {
                rows.push(normalize_memory_record(record));
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
            "Action result: memory_schema\ntables:\n- memories(id TEXT, created_at_ms INTEGER, updated_at_ms INTEGER, version INTEGER, content TEXT)\n- chat_messages(id TEXT, session_id TEXT, turn_id TEXT, role TEXT, content TEXT, created_at_ms INTEGER, source TEXT, profile_name TEXT, model_name TEXT, source_message_id TEXT)\n- scratch_notes(id TEXT, created_at_ms INTEGER, scratch_type TEXT, label TEXT, content TEXT, prompt_delta_ids ARRAY, prompt_slice_ids ARRAY)\nsafe_interface: memmgr\nops:\n- durable: query|schema|sql|insert|update|upsert|delete\n- raw_chat: query|sql|delete\n- scratch: query|write|read|delete\n- context: shrink\nrules: memmgr sql ops accept SELECT, WITH ... SELECT, or PRAGMA table_info(memories/chat_messages); SQL writes are forbidden; use memmgr type=durable for durable memory insert/update/delete; use expected_version from query results when updating/deleting an existing durable memory to avoid multi-CLI conflicts; use memmgr type=raw_chat op=delete for explicit chat transcript deletion; scratch write requires kind=notes with content or kind=context_offload with delta_ids/slice_ids plus label; scratch read requires id and returns full scratch content. Empty raw_chat query lists recent chat records. loaded_chat_records={}.",
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
        self.guard
            .with_read(|| self.sql_read_unlocked(chat_history, sql, params, limit))?
    }

    fn sql_read_unlocked(
        &self,
        chat_history: &FileChatHistoryStore,
        sql: &str,
        params: &[String],
        limit: usize,
    ) -> Result<Vec<Vec<(String, String)>>, String> {
        validate_memory_sql(sql)?;
        let conn = Connection::open_in_memory().map_err(|_| "sqlite_open_failed".to_string())?;
        conn.execute(
            "CREATE TABLE memories(id TEXT NOT NULL, created_at_ms INTEGER NOT NULL, updated_at_ms INTEGER NOT NULL, version INTEGER NOT NULL, content TEXT NOT NULL)",
            [],
        )
        .map_err(|_| "sqlite_schema_failed".to_string())?;
        conn.execute(
            "CREATE TABLE chat_messages(id TEXT NOT NULL, session_id TEXT NOT NULL, turn_id TEXT NOT NULL, role TEXT NOT NULL, content TEXT NOT NULL, created_at_ms INTEGER NOT NULL, source TEXT NOT NULL, profile_name TEXT, model_name TEXT, source_message_id TEXT)",
            [],
        )
        .map_err(|_| "sqlite_schema_failed".to_string())?;
        for record in self
            .read_all_unlocked()
            .map_err(|_| "memory_read_failed".to_string())?
        {
            conn.execute(
                "INSERT INTO memories(id, created_at_ms, updated_at_ms, version, content) VALUES (?1, ?2, ?3, ?4, ?5)",
                (
                    &record.id,
                    record.created_at_ms,
                    record.updated_at_ms,
                    record.version,
                    &record.content,
                ),
            )
            .map_err(|_| "sqlite_load_failed".to_string())?;
        }
        for record in chat_history
            .read_all_unlocked()
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
    guard: MemGuard,
}

impl FileScratchStore {
    fn new(dir: &Path) -> Self {
        let _ = fs::create_dir_all(dir);
        Self {
            file: dir.join("scratch_notes.jsonl"),
            guard: MemGuard::for_memory_dir(dir),
        }
    }

    fn write_record(
        &self,
        scratch_type: &str,
        label: &str,
        content: &str,
        prompt_delta_ids: &[String],
        prompt_slice_ids: &[String],
    ) -> Result<ScratchNoteRecord, String> {
        let clean_type = memmgr::normalize_scratch_kind(scratch_type);
        let clean_label = label.trim();
        let clean_content = content.trim();
        if !matches!(clean_type.as_str(), "notes" | "context_offload") {
            return Err("type_unsupported".to_string());
        }
        if clean_label.is_empty() {
            return Err("label_required".to_string());
        }
        if clean_content.is_empty() {
            return Err("content_required".to_string());
        }
        self.guard.with_write(|| {
            self.write_clean_unlocked(
                &clean_type,
                clean_label,
                clean_content,
                prompt_delta_ids,
                prompt_slice_ids,
            )
        })?
    }

    fn write_clean_unlocked(
        &self,
        scratch_type: &str,
        label: &str,
        clean: &str,
        prompt_delta_ids: &[String],
        prompt_slice_ids: &[String],
    ) -> Result<ScratchNoteRecord, String> {
        let created_at_ms = now_ms();
        let mut clean_delta_ids = prompt_delta_ids
            .iter()
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty())
            .collect::<Vec<_>>();
        clean_delta_ids.sort();
        clean_delta_ids.dedup();
        let mut clean_slice_ids = prompt_slice_ids
            .iter()
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty())
            .collect::<Vec<_>>();
        clean_slice_ids.sort();
        clean_slice_ids.dedup();
        let record = ScratchNoteRecord {
            id: scratch_hash_id(
                scratch_type,
                label,
                clean,
                &clean_delta_ids,
                &clean_slice_ids,
            ),
            created_at_ms,
            scratch_type: scratch_type.to_string(),
            label: label.to_string(),
            content: clean.to_string(),
            prompt_delta_ids: clean_delta_ids,
            prompt_slice_ids: clean_slice_ids,
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

    fn read(&self, id: &str) -> Result<Option<ScratchNoteRecord>, String> {
        let clean_id = id.trim();
        if clean_id.is_empty() {
            return Err("id_required".to_string());
        }
        self.guard.with_read(|| {
            Ok(self
                .read_all_unlocked()?
                .into_iter()
                .find(|record| record.id == clean_id))
        })?
    }

    fn query(&self, query: &str, limit: usize) -> Result<Vec<ScratchNoteRecord>, String> {
        self.guard.with_read(|| self.query_unlocked(query, limit))?
    }

    fn query_unlocked(&self, query: &str, limit: usize) -> Result<Vec<ScratchNoteRecord>, String> {
        let terms = search_terms(query);
        let mut rows = self.read_all_unlocked()?;
        if !terms.is_empty() {
            rows.retain(|record| {
                let normalized = format!("{} {}", record.label, record.content).to_lowercase();
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
        self.guard.with_write(|| {
            let mut rows = self.read_all_unlocked()?;
            let before = rows.len();
            rows.retain(|record| record.id != clean_id);
            if rows.len() == before {
                return Ok(false);
            }
            self.write_all_unlocked(&rows)?;
            Ok(true)
        })?
    }

    fn read_all_unlocked(&self) -> Result<Vec<ScratchNoteRecord>, String> {
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

    fn write_all_unlocked(&self, rows: &[ScratchNoteRecord]) -> Result<(), String> {
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
struct FileChatHistoryStore {
    audit_file: PathBuf,
    legacy_audit_file: PathBuf,
    guard: MemGuard,
}
impl FileChatHistoryStore {
    fn new(memory_dir: &Path) -> Self {
        let space_dir = space_dir_for_memory_dir(memory_dir);
        let audit_file = space_dir.join("audit").join("api_audit.jsonl");
        let legacy_audit_file = space_dir.join("api_audit.jsonl");
        Self {
            audit_file,
            legacy_audit_file,
            guard: MemGuard::for_memory_dir(memory_dir),
        }
    }

    fn audit_files(&self) -> Vec<PathBuf> {
        let mut files = vec![self.audit_file.clone()];
        if self.legacy_audit_file != self.audit_file {
            files.push(self.legacy_audit_file.clone());
        }
        files
    }

    fn query(
        &self,
        query: &str,
        limit: usize,
        after_ms: Option<i64>,
        before_ms: Option<i64>,
    ) -> std::io::Result<Vec<ChatHistoryRecord>> {
        self.guard
            .with_read(|| self.query_unlocked(query, limit, after_ms, before_ms))
            .map_err(std::io::Error::other)?
    }

    fn query_unlocked(
        &self,
        query: &str,
        limit: usize,
        after_ms: Option<i64>,
        before_ms: Option<i64>,
    ) -> std::io::Result<Vec<ChatHistoryRecord>> {
        let terms = search_terms(query);
        let mut rows = self.read_all_unlocked()?;
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
        self.guard.with_write(|| {
            let targets = if clean_id.is_empty() {
                self.query_unlocked(query, limit, after_ms, before_ms)
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
            let mut deleted_turn_ids = HashSet::new();
            for audit_file in self.audit_files() {
                let file = match OpenOptions::new().read(true).open(&audit_file) {
                    Ok(file) => file,
                    Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
                    Err(_) => return Err("chat_history_read_failed".to_string()),
                };
                let mut retained = Vec::new();
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
                    .open(&audit_file)
                    .map_err(|_| "chat_history_write_failed".to_string())?;
                for line in retained {
                    writeln!(file, "{}", line)
                        .map_err(|_| "chat_history_write_failed".to_string())?;
                }
            }
            Ok(deleted_turn_ids.len())
        })?
    }

    fn read_all(&self) -> std::io::Result<Vec<ChatHistoryRecord>> {
        self.guard
            .with_read(|| self.read_all_unlocked())
            .map_err(std::io::Error::other)?
    }

    fn read_all_unlocked(&self) -> std::io::Result<Vec<ChatHistoryRecord>> {
        let mut rows = Vec::<ChatHistoryRecord>::new();
        for audit_file in self.audit_files() {
            let file = match OpenOptions::new().read(true).open(&audit_file) {
                Ok(file) => file,
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
                Err(err) => return Err(err),
            };
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
                        if let Some(existing) =
                            rows.iter_mut().rev().find(|row| row.turn_id == turn_id)
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

fn repair_failure_message(first_issue: &str, final_issue: &str) -> String {
    if first_issue == "truncated_model_output" || final_issue == "truncated_model_output" {
        return "模型回复被 API 提供商按最大输出 token 限制截断（例如 stop_reason=max_tokens），导致返回的 JSON 协议不完整。请调大 TIMEM_MAX_LLM_OUTPUT，或在交互提示中选择增加 10K 后重试。".to_string();
    }
    format!(
        "模型的回复不符合本地协议，已拦截原始报文展示。原因：{final_issue}。请重试或换一个更具体的问题。"
    )
}

fn can_show_plain_text_after_repair_failure(content: &str) -> bool {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return false;
    }
    if matches!(trimmed.chars().next(), Some('{') | Some('[')) {
        return false;
    }
    if trimmed.contains("```") || trimmed.contains('{') || trimmed.contains('}') {
        return false;
    }
    if extract_balanced_json_object(trimmed).is_some() {
        return false;
    }
    let lowered = trimmed.to_lowercase();
    ![
        "next_actions",
        "report_job_progress",
        "memory_candidates",
        "\"action\"",
        "'action'",
    ]
    .iter()
    .any(|needle| lowered.contains(needle))
}

#[allow(clippy::too_many_arguments)]
fn validate_memmgr_action(
    idx: usize,
    mem_type: &str,
    op: &str,
    query: &str,
    content: &str,
    scratch_type: &str,
    label: &str,
    sql: &str,
    params: &[String],
    id: &str,
    delta_ids: &[String],
    slice_ids: &[String],
) -> Result<(), String> {
    memmgr::validate_action(memmgr::MemmgrActionInput {
        idx,
        mem_type,
        op,
        query,
        content,
        scratch_type,
        label,
        sql,
        params,
        id,
        delta_ids,
        slice_ids,
    })
}

#[allow(clippy::too_many_arguments)]
fn validate_manifest_backed_action_extra(
    idx: usize,
    action: &str,
    mem_type: &str,
    op: &str,
    query: &str,
    content: &str,
    scratch_type: &str,
    label: &str,
    sql: &str,
    params: &[String],
    id: &str,
    delta_ids: &[String],
    slice_ids: &[String],
) -> Result<(), String> {
    match action {
        "capmgr" => {}
        "memmgr" => validate_memmgr_action(
            idx,
            mem_type,
            op,
            query,
            content,
            scratch_type,
            label,
            sql,
            params,
            id,
            delta_ids,
            slice_ids,
        )?,
        "run_bash" => {}
        "shell_job_status" => {}
        _ => {}
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn validate_legacy_action_shape(
    idx: usize,
    action: &str,
    query: &str,
    content: &str,
    scratch_type: &str,
    label: &str,
    sql: &str,
    params: &[String],
    operation: &str,
    id: &str,
    delta_ids: &[String],
    slice_ids: &[String],
) -> Result<(), String> {
    match action {
        "chat_history_query" => {}
        "chat_history_delete" => {
            if id.is_empty() && query.is_empty() {
                return Err(format!("next_actions[{idx}].input.id_or_query_required"));
            }
        }
        "query_memory" | "memory_query" => {
            if query.is_empty() {
                return Err(format!("next_actions[{idx}].input.query_required"));
            }
        }
        "memory_schema" => {}
        "memory_write" | "write_memory" => {
            if content.is_empty() && query.is_empty() {
                return Err(format!("next_actions[{idx}].input.content_required"));
            }
        }
        "memory_update" => {
            if operation.trim().is_empty() {
                return Err(format!("next_actions[{idx}].input.operation_required"));
            }
            if matches!(operation, "insert" | "upsert" | "update") && content.is_empty() {
                return Err(format!("next_actions[{idx}].input.content_required"));
            }
            if matches!(operation, "delete" | "update") && id.is_empty() {
                return Err(format!("next_actions[{idx}].input.id_required"));
            }
        }
        "scratch_write" => {
            let normalized_type = memmgr::normalize_scratch_kind(scratch_type);
            if scratch_type.trim().is_empty() {
                return Err(format!("next_actions[{idx}].input.type_required"));
            }
            if !matches!(normalized_type.as_str(), "notes" | "context_offload") {
                return Err(format!(
                    "next_actions[{idx}].input.type_unsupported:{scratch_type}"
                ));
            }
            if label.is_empty() {
                return Err(format!("next_actions[{idx}].input.label_required"));
            }
            if normalized_type == "notes" && content.is_empty() {
                return Err(format!("next_actions[{idx}].input.content_required"));
            }
            if normalized_type == "context_offload" && delta_ids.is_empty() && slice_ids.is_empty()
            {
                return Err(format!("next_actions[{idx}].input.prompt_refs_required"));
            }
        }
        "scratch_read" | "scratch_delete" => {
            if id.is_empty() {
                return Err(format!("next_actions[{idx}].input.id_required"));
            }
        }
        "scratch_query" => {}
        "prompt_shrink" => {
            if delta_ids.is_empty() && slice_ids.is_empty() {
                return Err(format!("next_actions[{idx}].input.ids_required"));
            }
        }
        "sql_read" | "memory_sql_query" => {
            if sql.is_empty() {
                return Err(format!("next_actions[{idx}].input.sql_required"));
            }
            let placeholder_count = sql.matches('?').count();
            if params.len() != placeholder_count {
                return Err(format!(
                    "next_actions[{idx}].input.params_count_mismatch expected={placeholder_count} actual={}",
                    params.len()
                ));
            }
        }
        _ => return Err(format!("next_actions[{idx}].unsupported_action:{action}")),
    }
    Ok(())
}

fn parse_envelope(content: &str, capabilities: &CapabilityRegistry) -> ParsedEnvelope {
    let value: Value = match parse_json_value_from_model_text(content) {
        Ok(value) => value,
        Err(_) => {
            return ParsedEnvelope {
                report_job_progress: String::new(),
                continue_work: true,
                continue_was_implicit: false,
                thought: String::new(),
                thought_durable: false,
                next_actions: vec![],
                memory_candidates: vec![],
                repair_issue: Some("invalid_json".to_string()),
            }
        }
    };
    if !value.is_object() {
        return ParsedEnvelope {
            report_job_progress: String::new(),
            continue_work: true,
            continue_was_implicit: false,
            thought: String::new(),
            thought_durable: false,
            next_actions: vec![],
            memory_candidates: vec![],
            repair_issue: Some("root_must_be_json_object".to_string()),
        };
    }
    let mut repair_issue: Option<String> = None;
    if value.get("response_to_user").is_some() {
        repair_issue = Some("response_to_user_renamed_to_report_job_progress".to_string());
    }
    let report_job_progress = value
        .get("report_job_progress")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let (continue_work, continue_was_implicit) = match value.get("continue") {
        Some(value) => match value.as_bool() {
            Some(value) => (value, false),
            None => {
                repair_issue =
                    repair_issue.or_else(|| Some("continue_must_be_boolean".to_string()));
                (true, false)
            }
        },
        None => (true, true),
    };
    let (thought, thought_durable) = {
        let v = value.get("thought");
        if let Some(obj) = v.and_then(Value::as_object) {
            let content = obj
                .get("content")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .map(ToString::to_string)
                .unwrap_or_default();
            let durable = obj.get("durable").and_then(Value::as_bool).unwrap_or(false);
            (content, durable)
        } else {
            // backward compat: plain string thought is always durable
            let s = v
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .map(ToString::to_string)
                .unwrap_or_default();
            let durable = !s.is_empty();
            (s, durable)
        }
    };

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
                let mem_type = input
                    .get("type")
                    .or_else(|| action.get("type"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_lowercase();
                let op = input
                    .get("op")
                    .or_else(|| input.get("operation"))
                    .or_else(|| action.get("op"))
                    .or_else(|| action.get("operation"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_lowercase();
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
                let scratch_type = input
                    .get("kind")
                    .or_else(|| input.get("scratch_type"))
                    .or_else(|| {
                        if name == "scratch_write" {
                            input.get("type")
                        } else {
                            None
                        }
                    })
                    .or_else(|| action.get("type"))
                    .or_else(|| action.get("scratch_type"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_string();
                let label = input
                    .get("label")
                    .or_else(|| action.get("label"))
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
                    .or_else(|| input.get("op"))
                    .or_else(|| action.get("operation"))
                    .or_else(|| action.get("op"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_lowercase()
                    .to_string();
                let expected_version = input
                    .get("expected_version")
                    .or_else(|| input.get("version"))
                    .or_else(|| action.get("expected_version"))
                    .or_else(|| action.get("version"))
                    .and_then(json_u64);
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
                let after_ms = input
                    .get("after_ms")
                    .or_else(|| action.get("after_ms"))
                    .and_then(json_i64);
                let before_ms = input
                    .get("before_ms")
                    .or_else(|| action.get("before_ms"))
                    .and_then(json_i64);
                let normalized_name = name.as_str();
                if capabilities.contains_tool(normalized_name) {
                    if let Err(issue) = capabilities.validate_action_input(normalized_name, input) {
                        repair_issue = Some(format!("next_actions[{idx}].{issue}"));
                        break;
                    }
                }
                if capabilities.contains_tool(normalized_name) {
                    if let Err(issue) = validate_manifest_backed_action_extra(
                        idx,
                        normalized_name,
                        &mem_type,
                        &op,
                        &query,
                        &content,
                        &scratch_type,
                        &label,
                        &sql,
                        &params,
                        &id,
                        &delta_ids,
                        &slice_ids,
                    ) {
                        repair_issue = Some(issue);
                        break;
                    }
                } else if let Err(issue) = validate_legacy_action_shape(
                    idx,
                    normalized_name,
                    &query,
                    &content,
                    &scratch_type,
                    &label,
                    &sql,
                    &params,
                    &operation,
                    &id,
                    &delta_ids,
                    &slice_ids,
                ) {
                    repair_issue = Some(issue);
                    break;
                }
                let parsed_timeout_ms = timeout_ms_raw
                    .or_else(|| timeout_sec_raw.map(|seconds| seconds.saturating_mul(1000)));
                let timeout_ms = if normalized_name == "shell_job_status" {
                    parsed_timeout_ms.unwrap_or(0).min(15000)
                } else {
                    parsed_timeout_ms.unwrap_or(5000).clamp(1000, 15000)
                };
                let expect = input
                    .get("expect")
                    .or_else(|| action.get("expect"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_string();
                let expect_timeout_ms = input
                    .get("expect_timeout_ms")
                    .or_else(|| action.get("expect_timeout_ms"))
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                next_actions.push(ParsedAction {
                    action: name,
                    intent: intent.to_string(),
                    raw_input: input.clone(),
                    mem_type,
                    op,
                    query,
                    content,
                    scratch_type,
                    label,
                    sql,
                    params,
                    operation,
                    expected_version,
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
                    expect,
                    expect_timeout_ms,
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
    if repair_issue.is_none() && !continue_work && report_job_progress.trim().is_empty() {
        repair_issue = Some("report_job_progress_required_when_continue_false".to_string());
    }
    // guarded finalize validation: continue:false + next_actions requires expect on last action
    if repair_issue.is_none() && continue_work {
        for (i, a) in next_actions.iter().enumerate() {
            if !a.expect.is_empty() {
                repair_issue = Some(format!("next_actions[{i}].expect_requires_continue_false"));
                break;
            }
        }
    }
    if repair_issue.is_none() && !continue_work && !next_actions.is_empty() {
        let last_idx = next_actions.len() - 1;
        for (i, a) in next_actions.iter().enumerate() {
            if i != last_idx && !a.expect.is_empty() {
                repair_issue = Some(format!(
                    "next_actions[{i}].expect_only_allowed_on_last_action"
                ));
                break;
            }
        }
        if repair_issue.is_none() {
            let last = &next_actions[last_idx];
            if last.expect.is_empty() {
                repair_issue =
                    Some("continue_false_next_actions_require_expect_on_last_action".to_string());
            } else if last.expect_timeout_ms == 0 {
                repair_issue = Some(format!(
                    "next_actions[{last_idx}].expect_timeout_ms_required"
                ));
            }
        }
    }
    if repair_issue.is_none() && continue_work && next_actions.is_empty() {
        repair_issue = Some("next_actions_required_when_continue_true".to_string());
    }
    ParsedEnvelope {
        report_job_progress,
        continue_work,
        continue_was_implicit,
        thought,
        thought_durable,
        next_actions,
        memory_candidates,
        repair_issue,
    }
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
        object.contains_key("report_job_progress") || object.contains_key("next_actions")
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
        "report_job_progress",
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

fn normalize_memory_record(mut record: MemoryRecord) -> MemoryRecord {
    if record.version == 0 {
        record.version = 1;
    }
    if record.updated_at_ms == 0 {
        record.updated_at_ms = record.created_at_ms;
    }
    record
}

fn memory_conflict_result(
    id: &str,
    expected_version: u64,
    current_version: u64,
    current_content: &str,
) -> String {
    format!(
        "memory_conflict id={} expected_version={} current_version={} current_content={}",
        id,
        expected_version,
        current_version,
        compact_text(current_content, 240)
    )
}

fn memory_missing_expected_version_result(
    id: &str,
    current_version: u64,
    current_content: &str,
) -> String {
    format!(
        "missing_expected_version id={} current_version={} current_content={} hint=query memory_sql_query/query_memory first, then retry memory_update with expected_version=current_version",
        id,
        current_version,
        compact_text(current_content, 240)
    )
}

fn json_i64(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_u64().and_then(|raw| i64::try_from(raw).ok()))
}

fn json_u64(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_i64().and_then(|raw| u64::try_from(raw).ok()))
        .or_else(|| value.as_str().and_then(|raw| raw.trim().parse().ok()))
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
    if let Err(reason) = shell_exec::validate_bash_request(command_to_run) {
        return ActionExecution::Completed(format!(
            "Action result: run_bash\ncommand: {}\nerror: {}",
            command_to_run, reason
        ));
    }
    if !background && !read_back.is_empty() && read_back != command_to_run {
        if let Err(reason) = shell_exec::validate_bash_request(read_back) {
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
    let mut result = shell_exec::execute_one_bash(command_to_run, timeout_ms);
    if !background && !read_back.is_empty() && read_back != command_to_run {
        result.push_str("\n\n");
        result.push_str("Read-back result:\n");
        result.push_str(&shell_exec::execute_one_bash(read_back, timeout_ms));
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
        shell_exec::execute_one_bash(command_to_run, timeout_ms)
    };
    if !read_back.is_empty() && read_back != command_to_run {
        result.push_str("\n\n");
        result.push_str("Read-back result:\n");
        result.push_str(&shell_exec::execute_one_bash(read_back, timeout_ms));
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

fn format_expect_check_result(command: &str, bash_result: &str) -> String {
    let verdict = if bash_result_status(bash_result) == Some(0) {
        "PASS"
    } else {
        "FAIL"
    };
    format!(
        "Expect check:\ncommand: {}\ncontrolled_bash_result:\n{}\nverdict: {}",
        command, bash_result, verdict
    )
}

fn rewrite_memmgr_result_header(result: String, legacy_action: &str) -> String {
    let mut lines = result.lines();
    if lines.next() != Some("Action result: memmgr") {
        return result;
    }
    let mut rewritten = vec![format!("Action result: {legacy_action}")];
    let mut skipping_envelope = true;
    for line in lines {
        if skipping_envelope && (line.starts_with("type: ") || line.starts_with("op: ")) {
            continue;
        }
        skipping_envelope = false;
        rewritten.push(line.to_string());
    }
    rewritten.join("\n")
}

fn expect_check_passed(expect_body: &str) -> bool {
    expect_body
        .lines()
        .any(|line| line.trim() == "verdict: PASS")
}

fn bash_result_status(result: &str) -> Option<i32> {
    result.lines().find_map(|line| {
        line.trim()
            .strip_prefix("status: ")
            .and_then(|raw| raw.trim().parse().ok())
    })
}

fn compact_text(text: &str, max_chars: usize) -> String {
    let mut out = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if out.chars().count() > max_chars {
        out = out.chars().take(max_chars).collect::<String>();
        out.push('…');
    }
    out
}

fn scratch_label_for_display(record: &ScratchNoteRecord) -> String {
    if record.label.trim().is_empty() {
        "(unlabeled)".to_string()
    } else {
        record.label.trim().to_string()
    }
}

fn format_scratch_write_result(record: &ScratchNoteRecord) -> String {
    format!(
        "Action result: scratch_write\nid: {}\nlabel: {}\ntype: {}\nprompt_delta_ids: {}\nprompt_slice_ids: {}\ncontent_preview: {}",
        record.id,
        scratch_label_for_display(record),
        memmgr::normalize_scratch_kind(&record.scratch_type),
        comma_or_none(&record.prompt_delta_ids),
        comma_or_none(&record.prompt_slice_ids),
        compact_text(&record.content, 320)
    )
}

fn format_scratch_read_result(record: &ScratchNoteRecord) -> String {
    format!(
        "Action result: scratch_read\nid: {}\nfound: true\nlabel: {}\ntype: {}\nprompt_delta_ids: {}\nprompt_slice_ids: {}\ncontent:\n{}",
        record.id,
        scratch_label_for_display(record),
        memmgr::normalize_scratch_kind(&record.scratch_type),
        comma_or_none(&record.prompt_delta_ids),
        comma_or_none(&record.prompt_slice_ids),
        record.content
    )
}

fn format_prompt_slice_for_scratch(slice: &PromptSlice) -> String {
    format!(
        "[BEGIN SCRATCH OFFLOAD SLICE {}]\ndelta_id: {}\nslice_id: {}\nslice: {}/{}\nprompt_type: {}\ntime_ms: {}\n{}\n[END SCRATCH OFFLOAD SLICE {}]",
        slice.slice_id,
        slice.delta_id,
        slice.slice_id,
        slice.slice_index,
        slice.slice_count,
        slice.prompt_type,
        slice.time_ms,
        slice.text,
        slice.slice_id
    )
}

fn comma_or_none(items: &[String]) -> String {
    if items.is_empty() {
        "none".to_string()
    } else {
        items.join(",")
    }
}

fn scratch_hash_id(
    scratch_type: &str,
    label: &str,
    content: &str,
    delta_ids: &[String],
    slice_ids: &[String],
) -> String {
    let mut hasher = DefaultHasher::new();
    scratch_type.hash(&mut hasher);
    label.hash(&mut hasher);
    content.hash(&mut hasher);
    delta_ids.hash(&mut hasher);
    slice_ids.hash(&mut hasher);
    now_ms().hash(&mut hasher);
    ID_COUNTER.fetch_add(1, Ordering::SeqCst).hash(&mut hasher);
    format!("scratch_{:016x}", hasher.finish())
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
