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
mod tests {
    use super::*;
    use crate::ApiProtocol;

    fn config(provider: &str) -> ProviderConfig {
        ProviderConfig {
            provider: provider.to_string(),
            model: "qwen-plus".to_string(),
            base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1".to_string(),
            api_key: "secret".to_string(),
            timeout_secs: 120,
            max_llm_output_tokens: 10_000,
            max_llm_input_tokens: 100_000,
            api_protocol: ApiProtocol::OpenAiCompatible,
            response_protocol: crate::ResponseProtocolKind::Markdown,
        }
    }

    fn input() -> RuntimeConfigReportInput {
        RuntimeConfigReportInput {
            space: ".test_mem".to_string(),
            data_dir: "/tmp/timem/data".to_string(),
            api_audit_path: "/tmp/timem/data/.test_mem/audit/api_audit.json".to_string(),
            action_audit_path: "/tmp/timem/data/.test_mem/audit/action_audit.json".to_string(),
            bash_approval_mode: BashApprovalMode::Approve,
            work_instruction_mode: WorkInstructionLoadMode::Silent,
        }
    }

    #[test]
    fn config_report_is_ui_neutral_and_groups_effective_values() {
        let report = runtime_config_report(&config("aliyun"), input());
        let debug = format!("{report:?}");
        for forbidden in ["\x1b[", "▶", "Add..."] {
            assert!(
                !debug.contains(forbidden),
                "core config report must stay UI-neutral and avoid terminal marker {forbidden:?}"
            );
        }
        assert!(matches!(
            report.items[0],
            RuntimeConfigReportItem::Section(RuntimeConfigSection::Model)
        ));
        assert!(matches!(
            report.items[5],
            RuntimeConfigReportItem::Section(RuntimeConfigSection::Runtime)
        ));
        assert!(matches!(
            report.items[10],
            RuntimeConfigReportItem::Section(RuntimeConfigSection::Data)
        ));
        assert!(report.items.iter().any(|item| matches!(
            item,
            RuntimeConfigReportItem::Row(row)
                if row.key == "TIMEM_MAX_LLM_INPUT" && row.value == "100000"
        )));
        assert!(report.items.iter().any(|item| matches!(
            item,
            RuntimeConfigReportItem::Row(row)
                if row.key == "TIMEM_BASH_APPROVAL" && row.value == "approve"
        )));
        assert!(report.items.iter().any(|item| matches!(
            item,
            RuntimeConfigReportItem::Row(row)
                if row.key == "TIMEM_WORK_INSTRUCTIONS" && row.value == "silent"
        )));
        assert!(report.items.iter().any(|item| matches!(
            item,
            RuntimeConfigReportItem::Row(row)
                if row.kind == RuntimeConfigRowKind::BaseUrl && row.key == "TIMEM_BASE_URL"
        )));
    }

    #[test]
    fn config_report_marks_provider_overrides_as_not_default_only_for_known_defaults() {
        let mut known = config("aliyun");
        known.model = "aws-claude-sonnet-4-6".to_string();
        known.base_url = "https://example.invalid/v1".to_string();
        let report = runtime_config_report(&known, input());
        assert!(report.items.iter().any(|item| matches!(
            item,
            RuntimeConfigReportItem::Row(row) if row.key == "TIMEM_MODEL" && row.not_default
        )));
        assert!(report.items.iter().any(|item| matches!(
            item,
            RuntimeConfigReportItem::Row(row) if row.key == "TIMEM_BASE_URL" && row.not_default
        )));

        let mut custom = config("private");
        custom.model = "aws-claude-sonnet-4-6".to_string();
        custom.base_url = "https://example.invalid/v1".to_string();
        let report = runtime_config_report(&custom, input());
        assert!(report.items.iter().any(|item| matches!(
            item,
            RuntimeConfigReportItem::Row(row) if row.key == "TIMEM_MODEL" && !row.not_default
        )));
        assert!(report.items.iter().any(|item| matches!(
            item,
            RuntimeConfigReportItem::Row(row) if row.key == "TIMEM_BASE_URL" && !row.not_default
        )));
    }
}
