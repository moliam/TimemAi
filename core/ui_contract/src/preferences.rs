use serde::{Deserialize, Serialize};

/// User-visible response format supported by the active Interface.
///
/// Interfaces select this at Core startup. Core owns the safe translation into
/// model instructions and never accepts arbitrary prompt text through this contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AssistantResponseFormat {
    /// Compatibility value for hosts that have not declared a presentation format.
    #[default]
    Unspecified,
    Markdown,
    PlainText,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct InterfacePreferences {
    pub assistant_response_format: AssistantResponseFormat,
    /// Whether Core should guide the model to discover reusable Claude/Codex tools.
    pub claude_codex_tool_discovery: bool,
}

impl InterfacePreferences {
    pub const fn markdown() -> Self {
        Self {
            assistant_response_format: AssistantResponseFormat::Markdown,
            claude_codex_tool_discovery: false,
        }
    }

    pub const fn with_claude_codex_tool_discovery(mut self, enabled: bool) -> Self {
        self.claude_codex_tool_discovery = enabled;
        self
    }

    pub const fn plain_text() -> Self {
        Self {
            assistant_response_format: AssistantResponseFormat::PlainText,
            claude_codex_tool_discovery: false,
        }
    }
}
