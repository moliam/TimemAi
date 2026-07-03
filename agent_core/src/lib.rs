use rusqlite::{params_from_iter, types::ValueRef, Connection};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};
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
pub mod prompt_render;
pub mod prompt_spec;
pub mod self_tool;
pub mod shell_exec;
use self_tool::{SelfToolAbout, SelfToolInput, SelfToolPaths, SelfToolProcess, SelfToolState};
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
    pub repair_calls: u32,
    pub tool_calls: u32,
    pub mem_reads: u32,
    pub mem_writes: u32,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
    pub cached_tokens: u32,
    pub cache_created_tokens: u32,
    pub shrunk_tokens: u32,
}
impl UsageStats {
    pub fn zero() -> Self {
        Self {
            llm_calls: 0,
            repair_calls: 0,
            tool_calls: 0,
            mem_reads: 0,
            mem_writes: 0,
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: 0,
            cached_tokens: 0,
            cache_created_tokens: 0,
            shrunk_tokens: 0,
        }
    }
    pub fn add(&mut self, other: &UsageStats) {
        self.llm_calls += other.llm_calls;
        self.repair_calls += other.repair_calls;
        self.tool_calls += other.tool_calls;
        self.mem_reads += other.mem_reads;
        self.mem_writes += other.mem_writes;
        self.prompt_tokens += other.prompt_tokens;
        self.completion_tokens += other.completion_tokens;
        self.total_tokens += other.total_tokens;
        self.cached_tokens += other.cached_tokens;
        self.cache_created_tokens += other.cache_created_tokens;
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
    pub(crate) slices: Vec<PromptSlice>,
    #[serde(default)]
    pub hidden_slice_ids: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct PromptSlice {
    pub(crate) delta_id: String,
    pub(crate) slice_id: String,
    pub(crate) prompt_type: String,
    pub(crate) time_ms: i64,
    pub(crate) text: String,
    pub(crate) slice_index: usize,
    pub(crate) slice_count: usize,
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
}
impl ParsedAction {
    fn audit_input(&self) -> Value {
        let mut input = self.raw_input.clone();
        if self.action == "self_tool" {
            if let Some(object) = input.as_object_mut() {
                if let Some(key) = object.get("key").and_then(Value::as_str) {
                    if self_tool::is_sensitive_env_key(key)
                        || self_tool::is_memory_path_env_key(key)
                    {
                        object.insert("value".to_string(), json!("<redacted>"));
                    }
                }
            }
        }
        input
    }

    fn input_str(&self, key: &str) -> String {
        self.raw_input
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string()
    }

    fn input_raw_str(&self, key: &str) -> String {
        self.raw_input
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    }

    fn input_lower(&self, key: &str) -> String {
        self.input_str(key).to_lowercase()
    }

    fn input_u64(&self, key: &str) -> Option<u64> {
        self.raw_input.get(key).and_then(json_u64)
    }

    fn input_i64(&self, key: &str) -> Option<i64> {
        self.raw_input.get(key).and_then(json_i64)
    }

    fn input_bool(&self, key: &str) -> bool {
        self.raw_input
            .get(key)
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }

    fn input_list(&self, key: &str) -> Vec<String> {
        self.raw_input
            .get(key)
            .map(json_string_list)
            .unwrap_or_default()
    }

    fn input_params(&self) -> Vec<String> {
        self.raw_input
            .get("params")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(json_sql_param_to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    }

    fn timeout_ms(&self, default_ms: u64) -> u64 {
        self.input_u64("timeout_ms")
            .or_else(|| {
                self.input_u64("timeout_sec")
                    .map(|seconds| seconds.saturating_mul(1000))
            })
            .unwrap_or(default_ms)
    }

    fn shell_timeout_ms(&self) -> u64 {
        self.timeout_ms(5000).clamp(1000, 15000)
    }

    fn status_timeout_ms(&self) -> u64 {
        self.timeout_ms(0).min(15000)
    }

    fn background(&self) -> bool {
        self.input_bool("background")
            || self
                .raw_input
                .get("mode")
                .and_then(Value::as_str)
                .is_some_and(|mode| mode.trim().eq_ignore_ascii_case("background"))
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedEnvelope {
    report_job_progress: String,
    final_answer: String,
    continue_work: bool,
    thought: String,
    thought_keep_in_context: bool,
    next_actions: Vec<ParsedAction>,
    memory_candidates: Vec<String>,
    runtime_note: Option<String>,
    repair_issue: Option<String>,
}

impl ParsedEnvelope {
    fn final_text(&self) -> String {
        if self.final_answer.trim().is_empty() {
            self.report_job_progress.trim().to_string()
        } else {
            self.final_answer.trim().to_string()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingApproval {
    request: ApprovalRequest,
    command: String,
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
#[allow(clippy::large_enum_variant)]
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

fn default_self_tool_paths(memory_dir: &Path) -> SelfToolPaths {
    let space_dir = space_dir_for_memory_dir(memory_dir).to_path_buf();
    SelfToolPaths {
        space_dir: space_dir.clone(),
        memory_dir: memory_dir.to_path_buf(),
        memory_file: memory_dir.join("memory.jsonl"),
        scratch_file: memory_dir.join("scratch_notes.jsonl"),
        api_audit_file: space_dir.join("audit").join("api_audit.json"),
        action_audit_file: space_dir.join("audit").join("action_audit.json"),
    }
}

fn default_self_tool_about() -> SelfToolAbout {
    SelfToolAbout {
        name: "TimemAi".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        author: "TimemAi <phylimo@163.com>".to_string(),
        summary: "A lightweight local agent with Bash capability and multidimensional, time-aware memory.".to_string(),
        project: "https://github.com/moliam/TimemAi".to_string(),
        star_message: "Please star https://github.com/moliam/TimemAi".to_string(),
    }
}

fn default_self_tool_process() -> SelfToolProcess {
    SelfToolProcess {
        pid: std::process::id(),
        current_dir: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        executable: std::env::current_exe().unwrap_or_else(|_| PathBuf::from("timem")),
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
    self_tool: SelfToolState,
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
        let self_tool = SelfToolState::new(
            std::env::vars().collect::<BTreeMap<_, _>>(),
            default_self_tool_paths(memory_dir),
            default_self_tool_about(),
            default_self_tool_process(),
        );
        Self {
            static_prompt: static_prompt.into(),
            profile,
            capabilities: CapabilityRegistry::builtin(),
            memory: FileMemoryStore::new(memory_dir),
            scratch: FileScratchStore::new(memory_dir),
            chat_history: FileChatHistoryStore::new(memory_dir),
            shell_jobs: FileShellJobStore::new(memory_dir),
            action_audit: FileActionAuditStore::new(memory_dir),
            self_tool,
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
    pub fn set_self_tool_state(&mut self, self_tool: SelfToolState) {
        self.self_tool = self_tool;
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
    pub fn last_repair_issue(&self) -> Option<&str> {
        self.last_repair_issue.as_deref()
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
                &response.content,
            );
        }
        let parsed = parse_envelope(&response.content, &self.capabilities);
        let mut slices = Vec::new();
        if !parsed.thought.is_empty() && parsed.thought_keep_in_context {
            slices.push((
                "llm_thought".to_string(),
                format!("Thought:\n{}", parsed.thought),
            ));
        }
        if let Some(issue) = parsed.repair_issue.clone() {
            if !self.repair_attempted {
                return self.request_protocol_repair(
                    &issue,
                    protocol_repair_instruction(&issue),
                    &response.content,
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
            let final_text = if parsed.final_text().is_empty() {
                repair_failure_message(self.last_repair_issue.as_deref().unwrap_or(&issue), &issue)
            } else {
                parsed.final_text()
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
                for candidate in &parsed.memory_candidates {
                    if self.memory.write(candidate).is_ok() {
                        self.current_stats.tool_calls += 1;
                        self.current_stats.mem_writes += 1;
                    }
                }
                let final_text = parsed.final_text();
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
            let pending_final_text = parsed.final_text();
            let last_idx = parsed.next_actions.len() - 1;
            let command = parsed.next_actions[last_idx].input_str("command");
            let timeout_ms = parsed.next_actions[last_idx].shell_timeout_ms();
            if !pending_final_text.is_empty() {
                slices.push((
                    "llm_progress".to_string(),
                    format!("Job progress shown to user:\n{}", pending_final_text),
                ));
            }
            let command_body = match self.run_finished_final_command(&command, timeout_ms) {
                ActionExecution::Completed(result) => result,
                ActionExecution::NeedsApproval(pending) => {
                    self.append_delta(slices);
                    let request = pending.request.clone();
                    self.pending_approval = Some(pending);
                    return CoreStep::NeedsUserApproval { request };
                }
            };
            let pass = finished_final_command_passed(&command_body);
            slices.push(("result_of_llm_action".to_string(), command_body));
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
                "Note: 你上轮用 status:finished + final_answer + 最终 run_bash command 声明完成，但命令返回非 0。Runtime 已忽略 final_answer，请根据以上命令输出修正后再回复。".to_string(),
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
        // Omitted status is an intentional shorthand for status:working.
        if let Some(note) = parsed.runtime_note.as_deref() {
            slices.push(("runtime_note".to_string(), note.to_string()));
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
        for candidate in &parsed.memory_candidates {
            if self.memory.write(candidate).is_ok() {
                self.current_stats.tool_calls += 1;
                self.current_stats.mem_writes += 1;
            }
        }
        let final_text = parsed.final_text();
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
        prompt_render::render_prompt(&self.static_prompt, &self.capabilities, &self.deltas)
    }
    fn render_prompt_slices(&self) -> Vec<PromptSlice> {
        prompt_render::render_prompt_slices(&self.deltas)
    }
    fn remaining_rounds(&self) -> u32 {
        self.round_budget.saturating_sub(self.current_round)
    }

    fn request_protocol_repair(
        &mut self,
        issue: &str,
        instruction: &str,
        raw_response: &str,
    ) -> CoreStep {
        self.repair_attempted = true;
        self.last_repair_issue = Some(issue.to_string());
        self.current_stats.repair_calls = self.current_stats.repair_calls.saturating_add(1);
        self.append_delta(vec![(
            "response_repair".to_string(),
            format!(
                "Protocol repair request\nshrink_priority: discard_first\nissue: {}\nreason: {}\ninstruction:\n{}\n\nPrevious model response to repair:\n[BEGIN PREVIOUS_LLM_RESPONSE]\n{}\n[END PREVIOUS_LLM_RESPONSE]",
                issue,
                protocol_repair_reason(issue),
                instruction,
                focused_repair_response_text(issue, raw_response),
            ),
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
            .filter(|delta| !prompt_render::render_delta_slices(delta).is_empty())
            .rev()
            .take(12)
            .map(|delta| {
                let token_estimate = prompt_render::render_delta_slices(delta)
                    .iter()
                    .map(|slice| estimate_prompt_tokens(&slice.text))
                    .sum::<u32>();
                format!(
                    "- delta_id={} time_ms={} visible_slices={} estimated_tokens={}",
                    delta.delta_id,
                    delta.time_ms,
                    prompt_render::render_delta_slices(delta).len(),
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
        rows.truncate(limit.clamp(1, 50));
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
            executor::ExecutorTarget::Command { .. } => {
                unreachable!("command target returned early")
            }
        };
        let result = match dispatch_name {
            "capmgr" => self.execute_capmgr_action(&action),
            "memmgr" => self.execute_memmgr_action(&action),
            "self_tool" => self.execute_self_tool_action(&action),
            "shell_job_status" => {
                self.current_stats.tool_calls += 1;
                if action.raw_input.get("timeout_ms").is_none()
                    && action.raw_input.get("timeout_sec").is_none()
                {
                    "Action result: shell_job_status\nerror: invalid_input\nmessage: Missing `timeout_ms`. Choose how long the runtime should wait for this status check, from 0 to 15000 ms.".to_string()
                } else {
                    self.shell_jobs
                        .status(&action.input_str("job_id"), action.status_timeout_ms())
                }
            }
            "run_bash" => {
                self.current_stats.tool_calls += 1;
                let execution = execute_guarded_bash(
                    &action.input_str("command"),
                    action.background(),
                    action.shell_timeout_ms(),
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

    fn execute_memmgr_action(&mut self, action: &ParsedAction) -> String {
        self.current_stats.tool_calls += 1;
        let mem_type = action.input_lower("type");
        let op = action.input_lower("op");
        let query = action.input_str("query");
        let content = action.input_str("content");
        let scratch_type = action.input_str("kind");
        let label = action.input_str("label");
        let sql = action.input_str("sql");
        let params = action.input_params();
        let id = action.input_str("id");
        let delta_ids = action.input_list("delta_ids");
        let slice_ids = action.input_list("slice_ids");
        let limit = action.input_u64("limit").unwrap_or(5) as usize;
        let after_ms = action.input_i64("after_ms");
        let before_ms = action.input_i64("before_ms");
        let expected_version = action.input_u64("expected_version");
        if let Err(issue) = memmgr::validate_action(memmgr::MemmgrActionInput {
            idx: 0,
            mem_type: &mem_type,
            op: &op,
            query: &query,
            content: &content,
            scratch_type: &scratch_type,
            label: &label,
            sql: &sql,
            params: &params,
            id: &id,
            delta_ids: &delta_ids,
            slice_ids: &slice_ids,
        }) {
            return format!(
                "Action result: memmgr\ntype: {}\nop: {}\nerror: invalid_input\nmessage: {}",
                empty_as_missing(&mem_type),
                empty_as_missing(&op),
                natural_tool_input_message(&issue)
            );
        }
        match (mem_type.as_str(), op.as_str()) {
            ("durable", "query") => {
                self.current_stats.mem_reads += 1;
                let rows = self.memory.query(&query, limit).unwrap_or_default();
                if rows.is_empty() {
                    format!(
                        "Action result: memmgr\ntype: durable\nop: query\nquery: {}\nresults: none",
                        query
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
                        query, lines
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
                match self
                    .memory
                    .sql_read(&self.chat_history, &sql, &params, limit)
                {
                    Ok(rows) if rows.is_empty() => {
                        format!(
                            "Action result: memmgr\ntype: {}\nop: sql\nsql: {}\nresults: none",
                            mem_type, sql
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
                            mem_type, sql, lines
                        )
                    }
                    Err(err) => format!(
                        "Action result: memmgr\ntype: {}\nop: sql\nsql: {}\nerror: {}",
                        mem_type, sql, err
                    ),
                }
            }
            ("durable", "insert" | "update" | "upsert" | "delete") => {
                match self.memory.update(&op, &id, &content, expected_version) {
                    Ok(result) => {
                        self.current_stats.mem_writes += 1;
                        result
                            .replacen("Action result: memory_update", "Action result: memmgr\ntype: durable", 1)
                    }
                    Err(err) => format!("Action result: memmgr\ntype: durable\nop: {}\nerror: {}", op, err),
                }
            }
            ("raw_chat", "query") => {
                let rows = self
                    .chat_history
                    .query(&query, limit, after_ms, before_ms)
                    .unwrap_or_default();
                let delta_rows = self.query_prompt_slices(&query, limit, after_ms, before_ms);
                if rows.is_empty() && delta_rows.is_empty() {
                    format!(
                        "Action result: memmgr\ntype: raw_chat\nop: query\nquery: {}\nresults: none",
                        query
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
                        query,
                        sections.join("\n")
                    )
                }
            }
            ("raw_chat", "delete") => match self.chat_history.delete(
                &id,
                &query,
                limit,
                after_ms,
                before_ms,
            ) {
                Ok(deleted) => format!(
                    "Action result: memmgr\ntype: raw_chat\nop: delete\nid: {}\nquery: {}\ndeleted_count: {}",
                    id, query, deleted
                ),
                Err(err) => format!("Action result: memmgr\ntype: raw_chat\nop: delete\nerror: {}", err),
            },
            ("scratch", "write") => {
                let scratch_type = memmgr::normalize_scratch_kind(&scratch_type);
                let write_result = if scratch_type == "context_offload" {
                    self.collect_prompt_context_for_scratch(&delta_ids, &slice_ids)
                        .and_then(|offload| {
                            self.scratch.write_record(
                                &scratch_type,
                                &label,
                                &offload.content,
                                &offload.delta_ids,
                                &offload.slice_ids,
                            )
                        })
                } else {
                    self.scratch
                        .write_record(&scratch_type, &label, &content, &[], &[])
                };
                match write_result {
                    Ok(record) => format_scratch_write_result(&record)
                        .replacen("Action result: scratch_write", "Action result: memmgr\ntype: scratch\nop: write", 1),
                    Err(err) => format!("Action result: memmgr\ntype: scratch\nop: write\nerror: {}", err),
                }
            }
            ("scratch", "read") => match self.scratch.read(&id) {
                Ok(Some(record)) => format_scratch_read_result(&record)
                    .replacen("Action result: scratch_read", "Action result: memmgr\ntype: scratch\nop: read", 1),
                Ok(None) => format!(
                    "Action result: memmgr\ntype: scratch\nop: read\nid: {}\nfound: false",
                    id
                ),
                Err(err) => format!("Action result: memmgr\ntype: scratch\nop: read\nerror: {}", err),
            },
            ("scratch", "query") => match self.scratch.query(&query, limit) {
                Ok(rows) if rows.is_empty() => format!(
                    "Action result: memmgr\ntype: scratch\nop: query\nquery: {}\nresults: none",
                    query
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
                        query, lines
                    )
                }
                Err(err) => format!("Action result: memmgr\ntype: scratch\nop: query\nerror: {}", err),
            },
            ("scratch", "delete") => match self.scratch.delete(&id) {
                Ok(true) => format!(
                    "Action result: memmgr\ntype: scratch\nop: delete\nid: {}\ndeleted: true",
                    id
                ),
                Ok(false) => format!(
                    "Action result: memmgr\ntype: scratch\nop: delete\nid: {}\ndeleted: false",
                    id
                ),
                Err(err) => format!("Action result: memmgr\ntype: scratch\nop: delete\nerror: {}", err),
            },
            ("context", "shrink") => self
                .apply_prompt_shrink(&delta_ids, &slice_ids)
                .replacen("Action result: prompt_shrink", "Action result: memmgr\ntype: context\nop: shrink", 1),
            _ => format!(
                "Action result: memmgr\ntype: {}\nop: {}\nerror: unsupported_type_or_op",
                mem_type, op
            ),
        }
    }

    fn execute_capmgr_action(&mut self, action: &ParsedAction) -> String {
        self.current_stats.tool_calls += 1;
        capmgr::execute(
            &self.capabilities,
            capmgr::CapmgrActionInput {
                op: &action.input_lower("op"),
                kind: &action.input_str("kind"),
                id: &action.input_str("id"),
            },
        )
    }

    fn execute_self_tool_action(&mut self, action: &ParsedAction) -> String {
        self.current_stats.tool_calls += 1;
        self.self_tool.execute(SelfToolInput {
            self_type: &action.input_lower("type"),
            op: &action.input_lower("op"),
            key: &action.input_str("key"),
            value: &action.input_raw_str("value"),
        })
    }

    fn execute_command_capability(&mut self, action: &ParsedAction, path: &Path) -> String {
        self.current_stats.tool_calls += 1;
        let payload = json!({
            "action": action.action,
            "intent": action.intent,
            "args": action.raw_input,
        });
        executor::execute_command_action(&action.action, path, &payload, action.shell_timeout_ms())
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

    fn run_finished_final_command(&mut self, command: &str, timeout_ms: u64) -> ActionExecution {
        let timeout_ms = timeout_ms.clamp(1000, 15_000);
        let execution = execute_guarded_bash(
            command,
            false,
            timeout_ms,
            self.bash_approval_mode,
            "Verify final answer before showing it.",
            &self.shell_jobs,
        );
        match execution {
            ActionExecution::Completed(result) => {
                let body = format_finished_final_command_result(command, &result);
                let status = if finished_final_command_passed(&body) {
                    "final_command_check_command_pass"
                } else {
                    "final_command_check_command_fail"
                };
                self.record_finished_final_command_audit(command, timeout_ms, status, Some(&body));
                ActionExecution::Completed(body)
            }
            ActionExecution::NeedsApproval(pending) => {
                let summary = format!(
                    "Final run_bash command:\ncommand: {}\nstatus: needs_user_approval\napproval_id: {}\nverdict: PENDING",
                    command, pending.request.approval_id
                );
                self.record_finished_final_command_audit(
                    command,
                    timeout_ms,
                    "final_command_check_command_needs_user_approval",
                    Some(&summary),
                );
                ActionExecution::NeedsApproval(pending)
            }
        }
    }

    fn record_finished_final_command_audit(
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
                action: "final_command_check_command".to_string(),
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
            let rendered = prompt_render::render_delta_slices(delta);
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
                for slice in prompt_render::render_delta_slices(delta) {
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
                let slices = prompt_render::render_delta_slices(delta);
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
        rows.truncate(limit.clamp(1, 50));
        Ok(rows)
    }
    fn recent(&self, limit: usize) -> std::io::Result<Vec<MemoryRecord>> {
        self.guard
            .with_read(|| {
                let mut rows = self.read_all_unlocked()?;
                rows.sort_by(|a, b| b.created_at_ms.cmp(&a.created_at_ms));
                rows.truncate(limit.clamp(1, 50));
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
            #[allow(clippy::needless_range_loop)]
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
            if out.len() >= limit.clamp(1, 200) {
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
        rows.truncate(limit.clamp(1, 50));
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
        let audit_file = space_dir.join("audit").join("api_audit.json");
        let legacy_audit_file = space_dir.join("api_audit.jsonl");
        Self {
            audit_file,
            legacy_audit_file,
            guard: MemGuard::for_memory_dir(memory_dir),
        }
    }

    fn audit_files(&self) -> Vec<PathBuf> {
        let mut files = vec![self.audit_file.clone()];
        let audit_dir_jsonl = self.audit_file.with_extension("jsonl");
        if audit_dir_jsonl != self.audit_file {
            files.push(audit_dir_jsonl);
        }
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
        rows.truncate(limit.clamp(1, 50));
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
                if !audit_file.exists() {
                    continue;
                }
                let events = read_audit_events_unlocked(&audit_file)
                    .map_err(|_| "chat_history_read_failed".to_string())?;
                let mut retained = Vec::new();
                for value in events {
                    let turn_id = value
                        .get("turn_id")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    if !turn_id.is_empty() && targets.contains(&turn_id) {
                        deleted_turn_ids.insert(turn_id);
                        continue;
                    }
                    retained.push(value);
                }
                write_audit_events_unlocked(&audit_file, &retained)
                    .map_err(|_| "chat_history_write_failed".to_string())?;
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
            for value in read_audit_events_unlocked(&audit_file)? {
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

fn read_audit_events_unlocked(path: &Path) -> std::io::Result<Vec<Value>> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err),
    };
    if text.trim().is_empty() {
        return Ok(Vec::new());
    }
    if let Ok(value) = serde_json::from_str::<Value>(&text) {
        if let Some(events) = value.get("events").and_then(Value::as_array) {
            return Ok(events.clone());
        }
    }
    Ok(text
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .collect())
}

fn write_audit_events_unlocked(path: &Path, events: &[Value]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    if path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(path)?;
        for event in events {
            writeln!(file, "{}", serde_json::to_string(event).unwrap_or_default())?;
        }
        return Ok(());
    }
    let doc = json!({"version": 1, "events": events});
    let text = serde_json::to_string_pretty(&doc).map_err(std::io::Error::other)?;
    fs::write(path, format!("{text}\n"))
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

fn parse_envelope(content: &str, capabilities: &CapabilityRegistry) -> ParsedEnvelope {
    let value: Value = match parse_json_value_from_model_text(content) {
        Ok(value) => value,
        Err(_) => {
            return ParsedEnvelope {
                report_job_progress: String::new(),
                final_answer: String::new(),
                continue_work: true,
                thought: String::new(),
                thought_keep_in_context: false,
                next_actions: vec![],
                memory_candidates: vec![],
                runtime_note: None,
                repair_issue: Some("invalid_json".to_string()),
            }
        }
    };
    // Auto-wrap array of action objects into {"next_actions": [...]}
    let value = if value.is_array() {
        let arr = value.as_array().unwrap();
        let all_actions = !arr.is_empty()
            && arr.iter().all(|item| {
                item.as_object()
                    .is_some_and(|obj| obj.contains_key("action"))
            });
        if all_actions {
            json!({"next_actions": value})
        } else {
            value
        }
    } else {
        value
    };
    if !value.is_object() {
        return ParsedEnvelope {
            report_job_progress: String::new(),
            final_answer: String::new(),
            continue_work: true,
            thought: String::new(),
            thought_keep_in_context: false,
            next_actions: vec![],
            memory_candidates: vec![],
            runtime_note: None,
            repair_issue: Some("root_must_be_json_object".to_string()),
        };
    }
    let mut repair_issue: Option<String> = None;
    if let Some(object) = value.as_object() {
        if let Some(extra_key) = object
            .keys()
            .find(|key| !is_allowed_response_top_level_key(key))
        {
            repair_issue = Some(format!("unexpected_top_level_field:{extra_key}"));
        }
    }
    if value.get("response_to_user").is_some() {
        repair_issue = Some("response_to_user_renamed_to_report_job_progress".to_string());
    }
    let report_job_progress = value
        .get("report_job_progress")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let mut final_answer = value
        .get("final_answer")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let status = value.get("status").and_then(Value::as_str);
    let mut continue_work = match status {
        Some("working") => true,
        Some("finished") => false,
        Some(_) => {
            repair_issue =
                repair_issue.or_else(|| Some("status_must_be_working_or_finished".to_string()));
            true
        }
        None => match value.get("continue") {
            Some(value) => match value.as_bool() {
                Some(value) => value,
                None => {
                    repair_issue =
                        repair_issue.or_else(|| Some("continue_must_be_boolean".to_string()));
                    true
                }
            },
            None => true,
        },
    };
    if value.get("status").is_none()
        && value.get("continue").and_then(Value::as_bool) == Some(false)
        && final_answer.trim().is_empty()
    {
        final_answer = report_job_progress.trim().to_string();
    }
    let (thought, thought_keep_in_context) = {
        let v = value.get("thought");
        if let Some(obj) = v.and_then(Value::as_object) {
            let content = obj
                .get("content")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .map(ToString::to_string)
                .unwrap_or_default();
            let keep_in_context = obj
                .get("keep_in_context")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            (content, keep_in_context)
        } else {
            // Plain string thought predates the explicit object form; keep it
            // in context because the model had no separate retention flag.
            let s = v
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .map(ToString::to_string)
                .unwrap_or_default();
            let keep_in_context = !s.is_empty();
            (s, keep_in_context)
        }
    };
    let mut runtime_note: Option<String> = None;

    let mut next_actions = Vec::new();
    let bare_action = value.get("action").and_then(Value::as_str).is_some();
    let action_values = if let Some(next_actions_value) = value.get("next_actions") {
        if let Some(actions) = next_actions_value.as_array() {
            Some(actions.iter().collect::<Vec<_>>())
        } else if !next_actions_value.is_null() {
            repair_issue = Some("next_actions_must_be_array".to_string());
            None
        } else {
            None
        }
    } else if bare_action {
        Some(vec![&value])
    } else {
        None
    };
    if let Some(actions) = action_values {
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
            let parsed_input_holder;
            let action_args = action.get("args");
            let input = match action_args {
                Some(Value::Object(_)) => {
                    parsed_input_holder = action_args.cloned().unwrap_or(Value::Null);
                    &parsed_input_holder
                }
                Some(_) => {
                    repair_issue = Some(format!("next_actions[{idx}].args_must_be_object"));
                    break;
                }
                None => {
                    repair_issue = Some(format!("next_actions[{idx}].args_required"));
                    break;
                }
            };
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
            let normalized_name = name.as_str();
            if !capabilities.contains_tool(normalized_name) {
                repair_issue = Some(format!("unsupported_action:{normalized_name}"));
                break;
            }
            if let Err(issue) = capabilities.validate_action_input(normalized_name, input) {
                repair_issue = Some(format!("next_actions[{idx}].{issue}"));
                break;
            }
            next_actions.push(ParsedAction {
                action: name,
                intent: intent.to_string(),
                raw_input: input.clone(),
            });
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
    if repair_issue.is_none() && !continue_work && final_answer.trim().is_empty() {
        repair_issue = Some("final_answer_required_when_status_finished".to_string());
    }
    if repair_issue.is_none()
        && continue_work
        && status != Some("finished")
        && !final_answer.trim().is_empty()
    {
        repair_issue = Some("final_answer_requires_status_finished".to_string());
    }
    if repair_issue.is_none()
        && !continue_work
        && starts_with_runtime_progress_marker(&final_answer)
    {
        repair_issue = Some("final_answer_must_not_start_with_runtime_progress_marker".to_string());
    }
    if repair_issue.is_none()
        && !continue_work
        && !next_actions.is_empty()
        && !is_valid_finished_final_command_actions(&next_actions)
        && next_actions.iter().any(is_evidence_gathering_action)
    {
        continue_work = true;
        final_answer.clear();
        runtime_note = Some(
            "Note: 上轮输出同时声明 status:finished/final_answer 和需要取证的 next_actions。Runtime 已丢弃提前 final_answer，并按 status:working 执行这些 actions。请只根据下面的 action results 给最终答案。"
                .to_string(),
        );
    }
    if repair_issue.is_none() && !continue_work && !next_actions.is_empty() {
        let last_idx = next_actions.len() - 1;
        if next_actions.len() > 1 {
            repair_issue =
                Some("status_finished_next_actions_must_only_contain_guard_command".to_string());
        }
        if repair_issue.is_none() {
            let last = &next_actions[last_idx];
            if last.action != "run_bash" {
                repair_issue = Some("status_finished_guard_requires_run_bash_command".to_string());
            } else if last.input_str("command").is_empty() {
                repair_issue =
                    Some("status_finished_next_actions_require_command_on_last_action".to_string());
            } else if last.background() {
                repair_issue = Some("status_finished_guard_must_only_use_command".to_string());
            }
        }
    }
    if repair_issue.is_none() && continue_work && next_actions.is_empty() {
        repair_issue = Some("next_actions_required_when_status_working".to_string());
    }
    ParsedEnvelope {
        report_job_progress,
        final_answer,
        continue_work,
        thought,
        thought_keep_in_context,
        next_actions,
        memory_candidates,
        runtime_note,
        repair_issue,
    }
}

fn starts_with_runtime_progress_marker(text: &str) -> bool {
    let trimmed = text.trim_start();
    trimmed.starts_with('◉') || trimmed.starts_with("▰▱")
}

fn is_allowed_response_top_level_key(key: &str) -> bool {
    matches!(
        key,
        "status"
            | "report_job_progress"
            | "final_answer"
            | "next_actions"
            | "thought"
            | "memory_candidates"
            | "continue"
            | "acceptance_check"
            | "action"
            | "args"
            | "intent"
    )
}

fn is_valid_finished_final_command_actions(actions: &[ParsedAction]) -> bool {
    let [action] = actions else {
        return false;
    };
    action.action == "run_bash" && !action.input_str("command").is_empty() && !action.background()
}

fn is_evidence_gathering_action(action: &ParsedAction) -> bool {
    action.action != "run_bash" || !action.input_str("command").is_empty() || action.background()
}

fn protocol_repair_instruction(issue: &str) -> &'static str {
    match issue {
        "final_answer_requires_status_finished" => {
            "检查到刚刚的输出格式有点问题：你提供了 final_answer，但缺少 status:\"finished\"。如果 job 确实已经 finished，请同时提供 status:\"finished\" 和 final_answer；如果仍在工作中，请去掉 final_answer，并提供 next_actions。Return exactly one valid JSON object. Do not use markdown fences."
        }
        "final_answer_required_when_status_finished" => {
            "检查到刚刚的输出格式有点问题：你提供了 status:\"finished\"，但缺少 final_answer。如果 job 确实已经 finished，请同时提供 status:\"finished\" 和 final_answer；如果仍在工作中，请不要使用 status:\"finished\"，并提供 next_actions。Return exactly one valid JSON object. Do not use markdown fences."
        }
        "status_finished_guard_must_only_use_command" => {
            "检查到刚刚的输出格式有点问题：status:\"finished\" + next_actions 只能包含一个最终 run_bash command，不能使用后台执行或其他额外字段。如果还需要查询证据，请省略 status 或使用 status:\"working\"；拿到 action result 后再给 status:\"finished\" + final_answer。Return exactly one valid JSON object. Do not use markdown fences."
        }
        "status_finished_next_actions_must_only_contain_guard_command" => {
            "检查到刚刚的输出格式有点问题：status:\"finished\" + next_actions 只能包含一个最终 run_bash command。如果还需要多个动作或查询证据，请使用 status:\"working\"，执行 action 后再基于 action result 给出最终答案。Return exactly one valid JSON object. Do not use markdown fences."
        }
        "status_finished_guard_requires_run_bash_command" => {
            "检查到刚刚的输出格式有点问题：status:\"finished\" + next_actions 的最终确认只能使用 action:\"run_bash\" 且 args.command 非空。如果需要查询记忆或执行其他工具，请使用 status:\"working\"。Return exactly one valid JSON object. Do not use markdown fences."
        }
        "status_finished_next_actions_require_command_on_last_action" => {
            "检查到刚刚的输出格式有点问题：status:\"finished\" + next_actions 需要一个最终 run_bash command，用该 command 的返回值决定 final_answer 是否展示。Return exactly one valid JSON object. Do not use markdown fences."
        }
        _ => {
            "Return exactly one valid JSON object. Omitted status defaults to working; include next_actions when working. Use status:\"finished\" together with final_answer only when the job is complete. Do not use markdown fences."
        }
    }
}

fn protocol_repair_reason(issue: &str) -> &'static str {
    match issue {
        "truncated_model_output" => {
            "The provider stopped the model output before a complete response_v1 JSON object was produced."
        }
        "invalid_json" => "The previous model response could not be parsed as one JSON object.",
        "root_must_be_json_object" => {
            "The previous model response parsed as JSON, but the root value was not an object."
        }
        "final_answer_requires_status_finished" => {
            "The previous model response included final_answer without status:\"finished\"."
        }
        "final_answer_required_when_status_finished" => {
            "The previous model response included status:\"finished\" without final_answer."
        }
        "status_finished_guard_must_only_use_command" => {
            "The previous model response used status:\"finished\" with a final run_bash action that included fields other than command/timeout_ms."
        }
        "status_finished_next_actions_must_only_contain_guard_command" => {
            "The previous model response used status:\"finished\" with multiple next_actions. With status:\"finished\" and final_answer, next_actions may only contain one final run_bash command."
        }
        "status_finished_guard_requires_run_bash_command" => {
            "The previous model response used status:\"finished\" with a final action that was not run_bash."
        }
        "status_finished_next_actions_require_command_on_last_action" => {
            "The previous model response used status:\"finished\" with next_actions but without a run_bash command."
        }
        "final_answer_must_not_start_with_runtime_progress_marker" => {
            "The final_answer started with a runtime UI progress marker instead of user-facing content."
        }
        _ => "The previous model response did not match the local response_v1 protocol.",
    }
}

fn focused_repair_response_text(issue: &str, text: &str) -> String {
    const REPAIR_CONTEXT_CHARS: usize = 6_000;
    let trimmed = text.trim();
    let char_count = trimmed.chars().count();
    if char_count <= REPAIR_CONTEXT_CHARS * 2 {
        return trimmed.to_string();
    }
    if let Some(focus) = repair_focus_char_index(issue, trimmed) {
        return char_window_around_focus(trimmed, focus, REPAIR_CONTEXT_CHARS);
    }
    let head: String = trimmed.chars().take(REPAIR_CONTEXT_CHARS).collect();
    let tail_start = char_count.saturating_sub(REPAIR_CONTEXT_CHARS);
    let tail: String = trimmed.chars().skip(tail_start).collect();
    format!(
        "{head}\n[TRUNCATED previous response: omitted middle chars {}..{} of {} chars; no precise repair focus found]\n{tail}",
        REPAIR_CONTEXT_CHARS, tail_start, char_count
    )
}

fn repair_focus_char_index(issue: &str, text: &str) -> Option<usize> {
    if matches!(issue, "invalid_json" | "truncated_model_output") {
        let json_start_byte = text.find('{').unwrap_or(0);
        let json_text = &text[json_start_byte..];
        if let Err(err) = serde_json::from_str::<Value>(json_text) {
            if let Some(relative_idx) =
                line_column_to_char_index(json_text, err.line(), err.column())
            {
                return Some(text[..json_start_byte].chars().count() + relative_idx);
            }
        }
    }
    let marker = match issue {
        "final_answer_requires_status_finished"
        | "final_answer_must_not_start_with_runtime_progress_marker" => "final_answer",
        "final_answer_required_when_status_finished" | "status_must_be_working_or_finished" => {
            "status"
        }
        issue if issue.starts_with("next_actions") => "next_actions",
        issue if issue.contains("memmgr") => "memmgr",
        issue if issue.contains("capmgr") => "capmgr",
        _ => "",
    };
    if marker.is_empty() {
        return None;
    }
    text.find(marker)
        .map(|byte_idx| text[..byte_idx].chars().count())
}

fn line_column_to_char_index(text: &str, line: usize, column: usize) -> Option<usize> {
    if line == 0 {
        return None;
    }
    let mut current_line = 1usize;
    let mut current_column = 1usize;
    for (char_idx, ch) in text.chars().enumerate() {
        if current_line == line && current_column >= column.max(1) {
            return Some(char_idx);
        }
        if ch == '\n' {
            current_line += 1;
            current_column = 1;
        } else {
            current_column += 1;
        }
    }
    Some(text.chars().count())
}

fn char_window_around_focus(text: &str, focus: usize, context_chars: usize) -> String {
    let char_count = text.chars().count();
    let start = focus.saturating_sub(context_chars);
    let end = focus.saturating_add(context_chars).min(char_count);
    let window: String = text.chars().skip(start).take(end - start).collect();
    format!(
        "[FOCUSED previous response: chars {}..{} of {} chars; focus char {}]\n{}",
        start, end, char_count, focus, window
    )
}

/// Strip markdown code fences (```json ... ``` or ``` ... ```) from model output.
fn strip_markdown_code_fences(input: &str) -> Option<&str> {
    let trimmed = input.trim();
    let rest = trimmed.strip_prefix("```")?;
    let after_tag = rest.find('\n').map(|i| &rest[i + 1..]).unwrap_or("");
    let body = after_tag.strip_suffix("```").map(str::trim)?;
    if body.is_empty() {
        None
    } else {
        Some(body)
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
    // Strip markdown code fences and retry
    if let Some(stripped) = strip_markdown_code_fences(trimmed) {
        if let Ok(value) = serde_json::from_str(stripped) {
            return Ok(value);
        }
        if let Some(repaired) = repair_known_string_field_quotes(stripped) {
            if let Ok(value) = serde_json::from_str(&repaired) {
                return Ok(value);
            }
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
        object.contains_key("report_job_progress")
            || object.contains_key("next_actions")
            || object.contains_key("final_answer")
            || object.contains_key("status")
            || object.contains_key("thought")
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
        "missing_expected_version id={} current_version={} current_content={} hint=query with memmgr type=durable op=query or op=sql first, then retry memmgr type=durable op=update with expected_version=current_version",
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

fn json_string_list(value: &Value) -> Vec<String> {
    if let Some(items) = value.as_array() {
        return json_string_array(items);
    }
    value
        .as_str()
        .map(|text| {
            text.split(',')
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(|item| item.trim_matches(['"', '\'']).to_string())
                .collect()
        })
        .unwrap_or_default()
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
    background: bool,
    timeout_ms: u64,
    approval_mode: BashApprovalMode,
    intent: &str,
    shell_jobs: &FileShellJobStore,
) -> ActionExecution {
    let command_to_run = command.trim();
    if command_to_run.is_empty() {
        return ActionExecution::Completed(
            "Action result: run_bash\nerror: command_required".to_string(),
        );
    }
    if let Err(reason) = shell_exec::validate_bash_request(command_to_run) {
        return ActionExecution::Completed(format!(
            "Action result: run_bash\ncommand: {}\nerror: {}",
            command_to_run, reason
        ));
    }
    if approval_mode == BashApprovalMode::Ask {
        return ActionExecution::NeedsApproval(PendingApproval {
            request: ApprovalRequest {
                approval_id: format!("approval_{}", now_ms()),
                action: "run_bash".to_string(),
                command: command_to_run.to_string(),
                reason: "run_bash_requires_user_approval".to_string(),
                risk: "local_shell_command".to_string(),
                intent: intent.to_string(),
            },
            command: command_to_run.to_string(),
            background,
            timeout_ms,
            intent: intent.to_string(),
        });
    }
    if background {
        return ActionExecution::Completed(shell_jobs.spawn(command_to_run));
    }
    ActionExecution::Completed(shell_exec::execute_one_bash(command_to_run, timeout_ms))
}

fn execute_approved_bash(
    command: &str,
    background: bool,
    timeout_ms: u64,
    request: &ApprovalRequest,
    shell_jobs: &FileShellJobStore,
) -> String {
    let mut result = if background {
        shell_jobs.spawn(command.trim())
    } else {
        shell_exec::execute_one_bash(command.trim(), timeout_ms)
    };
    result.push_str(&format!(
        "\napproval_id: {}\napproval_status: approved_by_user",
        request.approval_id
    ));
    result
}

fn format_finished_final_command_result(command: &str, bash_result: &str) -> String {
    let verdict = if bash_result_status(bash_result) == Some(0) {
        "PASS"
    } else {
        "FAIL"
    };
    format!(
        "Final run_bash command:\ncommand: {}\ncontrolled_bash_result:\n{}\nverdict: {}",
        command, bash_result, verdict
    )
}

fn empty_as_missing(value: &str) -> &str {
    if value.trim().is_empty() {
        "(missing)"
    } else {
        value.trim()
    }
}

fn natural_tool_input_message(issue: &str) -> String {
    let field = issue
        .rsplit('.')
        .next()
        .unwrap_or(issue)
        .split(':')
        .next()
        .unwrap_or(issue);
    match field {
        "type_required" => "Missing `type`. Choose which memory surface to use, such as durable, raw_chat, scratch, or context.".to_string(),
        "op_required" => "Missing `op`. Choose an operation for the selected type, such as query, write, read, delete, sql, or shrink.".to_string(),
        "query_required" => "Missing `query`. Provide the search text for this query operation.".to_string(),
        "content_required" => "Missing `content`. Provide the text that should be written or updated.".to_string(),
        "id_required" => "Missing `id`. Provide the id returned by a previous query/read/write result.".to_string(),
        "id_or_query_required" => "Missing target. Provide either `id` or `query` so the runtime knows what to delete.".to_string(),
        "operation_required" => "Missing `operation`/`op`. Provide the memory update operation.".to_string(),
        "kind_required" | "type_required_when_op=write" => "Missing scratch `kind`. Use notes for a written checkpoint, or context_offload to store existing prompt delta/slice content.".to_string(),
        "label_required" => "Missing `label`. Provide a short retrieval label for this scratch record.".to_string(),
        "prompt_refs_required" => "Missing prompt references. For context_offload, provide at least one `delta_ids` or `slice_ids` value.".to_string(),
        "ids_required" => "Missing context ids. Provide `delta_ids` or `slice_ids` to shrink/offload dynamic prompt context.".to_string(),
        "sql_required" => "Missing `sql`. Provide a read-only SQL query for the selected memory surface.".to_string(),
        other if other.starts_with("params_count_mismatch") || issue.contains("params_count_mismatch") => {
            format!("SQL placeholder count does not match `params`. {issue}")
        }
        other if other.starts_with("unsupported_memmgr_type_or_op") || issue.contains("unsupported_memmgr_type_or_op") => {
            format!("Unsupported memory type/op combination. Use a supported memmgr type and operation. Detail: {issue}")
        }
        other if other.starts_with("kind_unsupported") || issue.contains("kind_unsupported") => {
            format!("Unsupported scratch kind. Use notes or context_offload. Detail: {issue}")
        }
        _ => format!("Invalid tool input. Detail: {issue}"),
    }
}

fn finished_final_command_passed(result_body: &str) -> bool {
    result_body
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
/// # Safety
/// The caller must ensure that the pointer is valid and was obtained from a corresponding allocation function, or is null.
pub unsafe extern "C" fn timem_core_free(handle: *mut AgentCoreHandle) {
    if !handle.is_null() {
        unsafe {
            drop(Box::from_raw(handle));
        }
    }
}

#[no_mangle]
/// # Safety
/// The caller must ensure that `value` is a valid pointer obtained from a previous call to a function that returns a C string, or is null.
pub unsafe extern "C" fn timem_core_free_string(value: *mut c_char) {
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
