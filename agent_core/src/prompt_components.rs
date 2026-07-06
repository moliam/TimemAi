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
                "user_supplement" => "user_supplement".to_string(),
                _ => "user_question".to_string(),
            },
            PromptComponentRole::Assistant { .. } => match kind {
                "free_talk" => "llm_free_talk".to_string(),
                _ => "llm_response".to_string(),
            },
            PromptComponentRole::System => match kind {
                "response_repair" => "response_repair".to_string(),
                "context_compacted" => "context_compacted".to_string(),
                _ => "result_of_llm_action".to_string(),
            },
        }
    }
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
