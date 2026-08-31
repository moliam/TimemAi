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
}

impl InterfacePreferences {
    pub const fn markdown() -> Self {
        Self {
            assistant_response_format: AssistantResponseFormat::Markdown,
        }
    }

    pub const fn plain_text() -> Self {
        Self {
            assistant_response_format: AssistantResponseFormat::PlainText,
        }
    }
}
