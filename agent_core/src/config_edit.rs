use crate::{
    bash_approval_mode_label, default_base_url, is_default_base_url, parse_api_protocol,
    status_view::HostStatusLevel, BashApprovalMode, ModelServiceConfig, WorkInstructionLoadMode,
};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuntimeConfigField {
    Model,
    ApiProtocol,
    ResponseProtocol,
    BaseUrl,
    MaxInput,
    MaxOutput,
    BashApproval,
    WorkInstructions,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuntimeConfigEffect {
    None,
    MaxInputChanged(u32),
    BashApprovalChanged(BashApprovalMode),
    WorkInstructionsChanged(WorkInstructionLoadMode),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeConfigMenuReport {
    pub items: Vec<RuntimeConfigMenuItem>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeConfigMenuItem {
    pub field: RuntimeConfigField,
    pub key: &'static str,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeConfigApplyReport {
    pub field: RuntimeConfigField,
    pub key: &'static str,
    pub value: String,
    pub effect: RuntimeConfigEffect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuntimeConfigApplyError {
    InvalidApiProtocol,
    InvalidTokenCount { field: RuntimeConfigField },
    InvalidBashApproval,
    InvalidWorkInstructions,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuntimeConfigApplyMessageKind {
    Updated,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeConfigApplyMessage {
    pub kind: RuntimeConfigApplyMessageKind,
    pub level: HostStatusLevel,
    pub key: &'static str,
    pub value: String,
}

impl RuntimeConfigApplyReport {
    pub fn message(&self) -> RuntimeConfigApplyMessage {
        RuntimeConfigApplyMessage {
            kind: RuntimeConfigApplyMessageKind::Updated,
            level: HostStatusLevel::Info,
            key: self.key,
            value: self.value.clone(),
        }
    }
}

pub const RUNTIME_CONFIG_FIELDS: [RuntimeConfigField; 8] = [
    RuntimeConfigField::Model,
    RuntimeConfigField::ApiProtocol,
    RuntimeConfigField::ResponseProtocol,
    RuntimeConfigField::BaseUrl,
    RuntimeConfigField::MaxInput,
    RuntimeConfigField::MaxOutput,
    RuntimeConfigField::BashApproval,
    RuntimeConfigField::WorkInstructions,
];

pub fn bash_approval_mode_from_sources(
    option: Option<&str>,
    env: &HashMap<String, String>,
) -> BashApprovalMode {
    let raw = option
        .map(ToString::to_string)
        .or_else(|| env.get("TIMEM_BASH_APPROVAL").cloned())
        .unwrap_or_else(|| "approve".to_string())
        .trim()
        .to_lowercase();
    match raw.as_str() {
        "approve" => BashApprovalMode::Approve,
        "ask" => BashApprovalMode::Ask,
        _ => BashApprovalMode::Ask,
    }
}

pub fn capabilities_dir_from_sources(
    option: Option<&str>,
    env: &HashMap<String, String>,
) -> Option<PathBuf> {
    option
        .map(ToString::to_string)
        .or_else(|| env.get("TIMEM_CAPABILITIES_DIR").cloned())
        .map(PathBuf::from)
}

impl RuntimeConfigField {
    pub fn label(self) -> &'static str {
        match self {
            RuntimeConfigField::Model => "TIMEM_MODEL",
            RuntimeConfigField::ApiProtocol => "TIMEM_API_PROTOCOL",
            RuntimeConfigField::ResponseProtocol => "TIMEM_RESPONSE_PROTOCOL",
            RuntimeConfigField::BaseUrl => "TIMEM_BASE_URL",
            RuntimeConfigField::MaxInput => "TIMEM_MAX_LLM_INPUT",
            RuntimeConfigField::MaxOutput => "TIMEM_MAX_LLM_OUTPUT",
            RuntimeConfigField::BashApproval => "TIMEM_BASH_APPROVAL",
            RuntimeConfigField::WorkInstructions => "TIMEM_WORK_INSTRUCTIONS",
        }
    }
}

pub fn runtime_config_field_value(
    config: &ModelServiceConfig,
    bash_approval_mode: BashApprovalMode,
    work_instruction_mode: WorkInstructionLoadMode,
    field: RuntimeConfigField,
) -> String {
    match field {
        RuntimeConfigField::Model => config.model.clone(),
        RuntimeConfigField::ApiProtocol => config.api_protocol.label().to_string(),
        RuntimeConfigField::ResponseProtocol => config.response_protocol.name().to_string(),
        RuntimeConfigField::BaseUrl => config.base_url.clone(),
        RuntimeConfigField::MaxInput => config.max_llm_input_tokens.to_string(),
        RuntimeConfigField::MaxOutput => config.max_llm_output_tokens.to_string(),
        RuntimeConfigField::BashApproval => {
            bash_approval_mode_label(bash_approval_mode).to_string()
        }
        RuntimeConfigField::WorkInstructions => {
            work_instruction_mode_label(work_instruction_mode).to_string()
        }
    }
}

pub fn runtime_config_menu_report(
    config: &ModelServiceConfig,
    bash_approval_mode: BashApprovalMode,
    work_instruction_mode: WorkInstructionLoadMode,
) -> RuntimeConfigMenuReport {
    RuntimeConfigMenuReport {
        items: RUNTIME_CONFIG_FIELDS
            .iter()
            .map(|field| RuntimeConfigMenuItem {
                field: *field,
                key: field.label(),
                value: runtime_config_field_value(
                    config,
                    bash_approval_mode,
                    work_instruction_mode,
                    *field,
                ),
            })
            .collect(),
    }
}

pub fn runtime_config_apply_report(
    config: &ModelServiceConfig,
    bash_approval_mode: BashApprovalMode,
    work_instruction_mode: WorkInstructionLoadMode,
    field: RuntimeConfigField,
    effect: RuntimeConfigEffect,
) -> RuntimeConfigApplyReport {
    RuntimeConfigApplyReport {
        field,
        key: field.label(),
        value: runtime_config_field_value(config, bash_approval_mode, work_instruction_mode, field),
        effect,
    }
}

pub fn apply_runtime_config_value(
    config: &mut ModelServiceConfig,
    bash_approval_mode: &mut BashApprovalMode,
    work_instruction_mode: &mut WorkInstructionLoadMode,
    field: RuntimeConfigField,
    value: &str,
) -> Result<RuntimeConfigEffect, RuntimeConfigApplyError> {
    match field {
        RuntimeConfigField::Model => {
            config.model = value.to_string();
            Ok(RuntimeConfigEffect::None)
        }
        RuntimeConfigField::ApiProtocol => {
            let old_protocol = config.api_protocol;
            let used_default_base_url = is_default_base_url(&old_protocol, &config.base_url);
            let next_protocol = parse_api_protocol(value)
                .map_err(|_| RuntimeConfigApplyError::InvalidApiProtocol)?;
            config.api_protocol = next_protocol;
            if used_default_base_url {
                config.base_url = default_base_url(&next_protocol).to_string();
            }
            Ok(RuntimeConfigEffect::None)
        }
        RuntimeConfigField::ResponseProtocol => {
            config.response_protocol = crate::ResponseProtocolKind::from_name(value);
            if !matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "xml" | "xml_v1" | "json" | "json_v1" | "response_v1"
            ) {
                return Err(RuntimeConfigApplyError::InvalidApiProtocol);
            }
            Ok(RuntimeConfigEffect::None)
        }
        RuntimeConfigField::BaseUrl => {
            config.base_url = value.to_string();
            Ok(RuntimeConfigEffect::None)
        }
        RuntimeConfigField::MaxInput => {
            let tokens = parse_token_count(value)
                .ok_or(RuntimeConfigApplyError::InvalidTokenCount { field })?
                .max(3_000);
            config.max_llm_input_tokens = tokens;
            Ok(RuntimeConfigEffect::MaxInputChanged(tokens))
        }
        RuntimeConfigField::MaxOutput => {
            let tokens = parse_token_count(value)
                .ok_or(RuntimeConfigApplyError::InvalidTokenCount { field })?
                .max(512);
            config.max_llm_output_tokens = tokens;
            Ok(RuntimeConfigEffect::None)
        }
        RuntimeConfigField::BashApproval => {
            let mode = match value.trim().to_lowercase().as_str() {
                "approve" => BashApprovalMode::Approve,
                "ask" => BashApprovalMode::Ask,
                _ => return Err(RuntimeConfigApplyError::InvalidBashApproval),
            };
            *bash_approval_mode = mode;
            Ok(RuntimeConfigEffect::BashApprovalChanged(mode))
        }
        RuntimeConfigField::WorkInstructions => {
            let mode = match value.trim().to_ascii_lowercase().as_str() {
                "silent" => WorkInstructionLoadMode::Silent,
                "ask" => WorkInstructionLoadMode::Ask,
                "off" | "disable" | "disabled" => WorkInstructionLoadMode::Off,
                _ => return Err(RuntimeConfigApplyError::InvalidWorkInstructions),
            };
            *work_instruction_mode = mode;
            Ok(RuntimeConfigEffect::WorkInstructionsChanged(mode))
        }
    }
}

pub fn work_instruction_mode_label(mode: WorkInstructionLoadMode) -> &'static str {
    match mode {
        WorkInstructionLoadMode::Silent => "silent",
        WorkInstructionLoadMode::Ask => "ask",
        WorkInstructionLoadMode::Off => "off",
    }
}

pub fn parse_token_count(value: &str) -> Option<u32> {
    let raw = value.trim().to_lowercase();
    let (number, multiplier) = if let Some(prefix) = raw.strip_suffix('k') {
        (prefix.trim(), 1_000f64)
    } else if let Some(prefix) = raw.strip_suffix('m') {
        (prefix.trim(), 1_000_000f64)
    } else {
        (raw.as_str(), 1f64)
    };
    let parsed = number.parse::<f64>().ok()?;
    if !parsed.is_finite() || parsed <= 0.0 {
        return None;
    }
    Some((parsed * multiplier).round().clamp(1.0, u32::MAX as f64) as u32)
}

#[cfg(test)]
#[path = "../tests/unit/config_edit_tests.rs"]
mod tests;
