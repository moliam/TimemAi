use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PromptComponentRole {
    User,
    System,
    Assistant { speaker: String },
}

impl PromptComponentRole {
    pub fn user() -> Self {
        Self::User
    }

    pub fn system() -> Self {
        Self::System
    }

    pub fn assistant(speaker: impl Into<String>) -> Self {
        Self::Assistant {
            speaker: speaker.into(),
        }
    }

    pub(crate) fn prompt_type_hint(&self, kind: &str) -> String {
        match self {
            PromptComponentRole::User => match kind {
                "user_supplement" | "user_resume_directly" => kind.to_string(),
                _ => "user_question".to_string(),
            },
            PromptComponentRole::Assistant { .. } => match kind {
                "free_talk" => "llm_free_talk".to_string(),
                "llm_response_raw_xml" => "llm_response_raw_xml".to_string(),
                "context_compression_summary" => "context_compression_summary".to_string(),
                _ => "llm_response".to_string(),
            },
            PromptComponentRole::System => match kind {
                "response_repair" => "response_repair".to_string(),
                "context_compressed" => "context_compressed".to_string(),
                "runtime_note"
                | "user_interrupted_work"
                | "turn_progress_reminder"
                | "turn_time_reminder"
                | "turn_round_reminder"
                | "running_job_update"
                | "job_killed"
                | "disk_pressure"
                | "runtime_config_changed"
                | "user_supplement_context"
                | "user_supplement_action_dispatch_timeout"
                | "memo_interrupted_deleted"
                | "memo_forcibly_deleted" => "runtime_note".to_string(),
                "mcp_capability_catalog" => "mcp_capability_catalog".to_string(),
                "mcp_capability_update" => "mcp_capability_update".to_string(),
                _ => "result_of_llm_action".to_string(),
            },
        }
    }
}

/// System-side runtime sideband kinds: runtime narration about the world
/// (job exits, disk pressure, config changes, host-supplied supplement
/// context, memo deletions), not results of model actions. They render as
/// `runtime_note` so the action-result heading stays reserved for real
/// model action results.
pub(crate) const RUNTIME_SIDEBAND_KINDS: [&str; 8] = [
    "running_job_update",
    "job_killed",
    "disk_pressure",
    "runtime_config_changed",
    "user_supplement_context",
    "user_supplement_action_dispatch_timeout",
    "memo_interrupted_deleted",
    "memo_forcibly_deleted",
];

/// True for sideband kinds that must keep the tool-result byte gate at
/// ingress: they previously rode the `result_of_llm_action` fallback, and
/// their producers (notably host-provided supplement context) are not all
/// guaranteed bounded upstream.
pub(crate) fn is_runtime_sideband_kind(kind: &str) -> bool {
    RUNTIME_SIDEBAND_KINDS.contains(&kind)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PromptComponent {
    pub id: String,
    pub role: PromptComponentRole,
    pub kind: String,
    pub content: String,
    pub source: String,
    pub created_at_ms: i64,
    pub sequence: u64,
    pub batch_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_policy_hint: Option<String>,
}

impl PromptComponent {
    pub(crate) fn prompt_type(&self) -> String {
        self.role.prompt_type_hint(&self.kind)
    }
}
