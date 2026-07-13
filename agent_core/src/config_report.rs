use crate::{
    default_api_protocol_for_provider, is_default_base_url_for_provider,
    is_default_model_for_provider, work_instruction_mode_label, BashApprovalMode, ProviderConfig,
    WorkInstructionLoadMode,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeConfigReport {
    pub items: Vec<RuntimeConfigReportItem>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeConfigReportItem {
    Section(RuntimeConfigSection),
    Row(RuntimeConfigReportRow),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeConfigSection {
    Model,
    Runtime,
    Data,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeConfigRowKind {
    Model,
    GatewayProvider,
    ApiProtocol,
    BaseUrl,
    MaxLlmInput,
    MaxLlmOutput,
    BashApproval,
    WorkInstructions,
    Space,
    DataDir,
    ApiAudit,
    ActionAudit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeConfigReportRow {
    pub kind: RuntimeConfigRowKind,
    pub key: String,
    pub value: String,
    pub not_default: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeConfigReportInput {
    pub space: String,
    pub data_dir: String,
    pub api_audit_path: String,
    pub action_audit_path: String,
    pub bash_approval_mode: BashApprovalMode,
    pub work_instruction_mode: WorkInstructionLoadMode,
}

pub fn runtime_config_report(
    config: &ProviderConfig,
    input: RuntimeConfigReportInput,
) -> RuntimeConfigReport {
    let default_protocol = default_api_protocol_for_provider(&config.provider);
    RuntimeConfigReport {
        items: vec![
            RuntimeConfigReportItem::Section(RuntimeConfigSection::Model),
            row(
                RuntimeConfigRowKind::Model,
                "TIMEM_MODEL",
                config.model.clone(),
                !is_default_model_for_provider(&config.provider, &config.model),
            ),
            row(
                RuntimeConfigRowKind::GatewayProvider,
                "TIMEM_GATEWAY_PROVIDER",
                config.provider.clone(),
                false,
            ),
            row(
                RuntimeConfigRowKind::ApiProtocol,
                "TIMEM_API_PROTOCOL",
                config.api_protocol.label().to_string(),
                config.api_protocol != default_protocol,
            ),
            row(
                RuntimeConfigRowKind::BaseUrl,
                "TIMEM_BASE_URL",
                config.base_url.clone(),
                !is_default_base_url_for_provider(&config.provider, &config.base_url),
            ),
            RuntimeConfigReportItem::Section(RuntimeConfigSection::Runtime),
            row(
                RuntimeConfigRowKind::MaxLlmInput,
                "TIMEM_MAX_LLM_INPUT",
                config.max_llm_input_tokens.to_string(),
                false,
            ),
            row(
                RuntimeConfigRowKind::MaxLlmOutput,
                "TIMEM_MAX_LLM_OUTPUT",
                config.max_llm_output_tokens.to_string(),
                false,
            ),
            row(
                RuntimeConfigRowKind::BashApproval,
                "TIMEM_BASH_APPROVAL",
                bash_approval_mode_label(input.bash_approval_mode).to_string(),
                false,
            ),
            row(
                RuntimeConfigRowKind::WorkInstructions,
                "TIMEM_WORK_INSTRUCTIONS",
                work_instruction_mode_label(input.work_instruction_mode).to_string(),
                input.work_instruction_mode != WorkInstructionLoadMode::Silent,
            ),
            RuntimeConfigReportItem::Section(RuntimeConfigSection::Data),
            row(
                RuntimeConfigRowKind::Space,
                "TIMEM_SPACE",
                input.space,
                false,
            ),
            row(
                RuntimeConfigRowKind::DataDir,
                "TIMEM_DATA_DIR",
                input.data_dir,
                false,
            ),
            row(
                RuntimeConfigRowKind::ApiAudit,
                "local_audit",
                input.api_audit_path,
                false,
            ),
            row(
                RuntimeConfigRowKind::ActionAudit,
                "",
                input.action_audit_path,
                false,
            ),
        ],
    }
}

pub fn bash_approval_mode_label(mode: BashApprovalMode) -> &'static str {
    match mode {
        BashApprovalMode::Ask => "ask",
        BashApprovalMode::Approve => "approve",
    }
}

fn row(
    kind: RuntimeConfigRowKind,
    key: impl Into<String>,
    value: impl Into<String>,
    not_default: bool,
) -> RuntimeConfigReportItem {
    RuntimeConfigReportItem::Row(RuntimeConfigReportRow {
        kind,
        key: key.into(),
        value: value.into(),
        not_default,
    })
}

#[cfg(test)]
#[path = "../tests/unit/config_report_tests.rs"]
mod tests;
