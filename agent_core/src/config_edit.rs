use crate::{
    bash_approval_mode_label, default_api_protocol_for_provider,
    known_default_base_url_for_provider, parse_api_protocol, status_view::HostStatusLevel,
    BashApprovalMode, ProviderConfig, WorkInstructionLoadMode,
};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuntimeConfigField {
    Model,
    GatewayProvider,
    ApiProtocol,
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
    EmptyGatewayProvider,
    CustomGatewayRequiresBaseUrl,
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
    RuntimeConfigField::GatewayProvider,
    RuntimeConfigField::ApiProtocol,
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
        .unwrap_or_else(|| "ask".to_string())
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
            RuntimeConfigField::GatewayProvider => "TIMEM_GATEWAY_PROVIDER",
            RuntimeConfigField::ApiProtocol => "TIMEM_API_PROTOCOL",
            RuntimeConfigField::BaseUrl => "TIMEM_BASE_URL",
            RuntimeConfigField::MaxInput => "TIMEM_MAX_LLM_INPUT",
            RuntimeConfigField::MaxOutput => "TIMEM_MAX_LLM_OUTPUT",
            RuntimeConfigField::BashApproval => "TIMEM_BASH_APPROVAL",
            RuntimeConfigField::WorkInstructions => "TIMEM_WORK_INSTRUCTIONS",
        }
    }
}

pub fn runtime_config_field_value(
    config: &ProviderConfig,
    bash_approval_mode: BashApprovalMode,
    work_instruction_mode: WorkInstructionLoadMode,
    field: RuntimeConfigField,
) -> String {
    match field {
        RuntimeConfigField::Model => config.model.clone(),
        RuntimeConfigField::GatewayProvider => config.provider.clone(),
        RuntimeConfigField::ApiProtocol => config.api_protocol.label().to_string(),
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
    config: &ProviderConfig,
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
    config: &ProviderConfig,
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
    config: &mut ProviderConfig,
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
        RuntimeConfigField::GatewayProvider => {
            apply_gateway_provider(config, value)?;
            Ok(RuntimeConfigEffect::None)
        }
        RuntimeConfigField::ApiProtocol => {
            config.api_protocol = parse_api_protocol(value)
                .map_err(|_| RuntimeConfigApplyError::InvalidApiProtocol)?;
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

fn apply_gateway_provider(
    config: &mut ProviderConfig,
    value: &str,
) -> Result<(), RuntimeConfigApplyError> {
    let old_provider = config.provider.clone();
    let next_provider = value.to_lowercase();
    if next_provider.trim().is_empty() {
        return Err(RuntimeConfigApplyError::EmptyGatewayProvider);
    }
    if let Some(default_base_url) = known_default_base_url_for_provider(&next_provider) {
        config.provider = next_provider.clone();
        config.api_protocol = default_api_protocol_for_provider(&next_provider);
        config.base_url = default_base_url;
        return Ok(());
    }

    let old_default_base_url = known_default_base_url_for_provider(&old_provider);
    let using_old_default = old_default_base_url
        .as_deref()
        .map(|default| config.base_url.trim_end_matches('/') == default.trim_end_matches('/'))
        .unwrap_or(false);
    if using_old_default {
        return Err(RuntimeConfigApplyError::CustomGatewayRequiresBaseUrl);
    }
    config.provider = next_provider;
    Ok(())
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
mod tests {
    use super::*;
    use crate::ApiProtocol;

    fn test_config() -> ProviderConfig {
        ProviderConfig {
            provider: "aliyun".to_string(),
            api_protocol: ApiProtocol::OpenAiCompatible,
            api_key: "secret".to_string(),
            model: "qwen-plus".to_string(),
            base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1".to_string(),
            timeout_secs: 120,
            max_llm_output_tokens: 10_000,
            max_llm_input_tokens: 100_000,
            response_protocol: crate::ResponseProtocolKind::Markdown,
        }
    }

    #[test]
    fn token_count_parser_accepts_suffixes_and_rejects_invalid_values() {
        assert_eq!(parse_token_count("10K"), Some(10_000));
        assert_eq!(parse_token_count("1.5M"), Some(1_500_000));
        assert_eq!(parse_token_count("512"), Some(512));
        assert_eq!(parse_token_count("0"), None);
        assert_eq!(parse_token_count("nan"), None);
    }

    #[test]
    fn runtime_option_sources_follow_option_env_default_precedence() {
        let mut env = HashMap::new();
        env.insert("TIMEM_BASH_APPROVAL".to_string(), " APPROVE ".to_string());
        env.insert(
            "TIMEM_CAPABILITIES_DIR".to_string(),
            "/env/capabilities".to_string(),
        );

        assert_eq!(
            bash_approval_mode_from_sources(Some("ask"), &env),
            BashApprovalMode::Ask
        );
        assert_eq!(
            bash_approval_mode_from_sources(None, &env),
            BashApprovalMode::Approve
        );
        assert_eq!(
            capabilities_dir_from_sources(Some("/cli/capabilities"), &env).as_deref(),
            Some(std::path::Path::new("/cli/capabilities"))
        );
        assert_eq!(
            capabilities_dir_from_sources(None, &env).as_deref(),
            Some(std::path::Path::new("/env/capabilities"))
        );
    }

    #[test]
    fn stale_bash_approval_aliases_fall_back_to_ask() {
        assert_eq!(
            bash_approval_mode_from_sources(Some("never"), &HashMap::new()),
            BashApprovalMode::Ask
        );
        let mut env = HashMap::new();
        env.insert("TIMEM_BASH_APPROVAL".to_string(), "approval".to_string());
        assert_eq!(
            bash_approval_mode_from_sources(None, &env),
            BashApprovalMode::Ask
        );
    }

    #[test]
    fn applies_model_token_and_bash_updates() {
        let mut config = test_config();
        let mut bash = BashApprovalMode::Ask;
        let mut work = WorkInstructionLoadMode::Silent;

        assert_eq!(
            runtime_config_field_value(&config, bash, work, RuntimeConfigField::MaxInput),
            "100000"
        );
        assert_eq!(
            apply_runtime_config_value(
                &mut config,
                &mut bash,
                &mut work,
                RuntimeConfigField::MaxOutput,
                "20K"
            )
            .unwrap(),
            RuntimeConfigEffect::None
        );
        assert_eq!(
            apply_runtime_config_value(
                &mut config,
                &mut bash,
                &mut work,
                RuntimeConfigField::MaxInput,
                "120K"
            )
            .unwrap(),
            RuntimeConfigEffect::MaxInputChanged(120_000)
        );
        assert_eq!(
            apply_runtime_config_value(
                &mut config,
                &mut bash,
                &mut work,
                RuntimeConfigField::WorkInstructions,
                "ask",
            )
            .unwrap(),
            RuntimeConfigEffect::WorkInstructionsChanged(WorkInstructionLoadMode::Ask)
        );
        assert_eq!(
            apply_runtime_config_value(
                &mut config,
                &mut bash,
                &mut work,
                RuntimeConfigField::BashApproval,
                "approve",
            )
            .unwrap(),
            RuntimeConfigEffect::BashApprovalChanged(BashApprovalMode::Approve)
        );

        assert_eq!(config.max_llm_output_tokens, 20_000);
        assert_eq!(config.max_llm_input_tokens, 120_000);
        assert_eq!(bash, BashApprovalMode::Approve);
        assert_eq!(work, WorkInstructionLoadMode::Ask);
    }

    #[test]
    fn config_menu_report_is_ui_neutral_command_data() {
        let config = test_config();
        let report = runtime_config_menu_report(
            &config,
            BashApprovalMode::Approve,
            WorkInstructionLoadMode::Ask,
        );
        let debug = format!("{report:?}");
        for forbidden in ["\x1b[", "▶", "Add..."] {
            assert!(
                !debug.contains(forbidden),
                "core config menu report must stay UI-neutral and avoid terminal marker {forbidden:?}"
            );
        }
        assert_eq!(report.items.len(), RUNTIME_CONFIG_FIELDS.len());
        assert_eq!(report.items[0].field, RuntimeConfigField::Model);
        assert_eq!(report.items[0].key, "TIMEM_MODEL");
        assert_eq!(report.items[0].value, "qwen-plus");
        assert!(report.items.iter().any(|item| {
            item.field == RuntimeConfigField::BashApproval
                && item.key == "TIMEM_BASH_APPROVAL"
                && item.value == "approve"
        }));
        assert!(report.items.iter().any(|item| {
            item.field == RuntimeConfigField::MaxInput
                && item.key == "TIMEM_MAX_LLM_INPUT"
                && item.value == "100000"
        }));
        assert!(report.items.iter().any(|item| {
            item.field == RuntimeConfigField::WorkInstructions
                && item.key == "TIMEM_WORK_INSTRUCTIONS"
                && item.value == "ask"
        }));
    }

    #[test]
    fn config_apply_report_is_ui_neutral_command_data() {
        let mut config = test_config();
        let mut bash = BashApprovalMode::Ask;
        let mut work = WorkInstructionLoadMode::Silent;
        let effect = apply_runtime_config_value(
            &mut config,
            &mut bash,
            &mut work,
            RuntimeConfigField::MaxInput,
            "120K",
        )
        .unwrap();
        let report =
            runtime_config_apply_report(&config, bash, work, RuntimeConfigField::MaxInput, effect);

        assert_eq!(report.field, RuntimeConfigField::MaxInput);
        assert_eq!(report.key, "TIMEM_MAX_LLM_INPUT");
        assert_eq!(report.value, "120000");
        assert_eq!(report.effect, RuntimeConfigEffect::MaxInputChanged(120_000));
        assert_eq!(
            report.message(),
            RuntimeConfigApplyMessage {
                kind: RuntimeConfigApplyMessageKind::Updated,
                level: HostStatusLevel::Info,
                key: "TIMEM_MAX_LLM_INPUT",
                value: "120000".to_string(),
            }
        );

        let debug = format!("{report:?}");
        for forbidden in ["已更新", "配置无效", "\x1b[", "▶"] {
            assert!(
                !debug.contains(forbidden),
                "core config apply report must stay UI-neutral and avoid terminal marker {forbidden:?}"
            );
        }
    }

    #[test]
    fn config_apply_errors_are_structured_and_ui_neutral() {
        let mut config = test_config();
        let mut bash = BashApprovalMode::Ask;
        let mut work = WorkInstructionLoadMode::Silent;

        assert_eq!(
            apply_runtime_config_value(
                &mut config,
                &mut bash,
                &mut work,
                RuntimeConfigField::ApiProtocol,
                "bad-protocol",
            ),
            Err(RuntimeConfigApplyError::InvalidApiProtocol)
        );
        assert_eq!(
            apply_runtime_config_value(
                &mut config,
                &mut bash,
                &mut work,
                RuntimeConfigField::MaxInput,
                "not-a-number",
            ),
            Err(RuntimeConfigApplyError::InvalidTokenCount {
                field: RuntimeConfigField::MaxInput
            })
        );
        assert_eq!(
            apply_runtime_config_value(
                &mut config,
                &mut bash,
                &mut work,
                RuntimeConfigField::WorkInstructions,
                "maybe",
            ),
            Err(RuntimeConfigApplyError::InvalidWorkInstructions)
        );
        assert_eq!(
            apply_runtime_config_value(
                &mut config,
                &mut bash,
                &mut work,
                RuntimeConfigField::BashApproval,
                "always",
            ),
            Err(RuntimeConfigApplyError::InvalidBashApproval)
        );

        let debug = format!(
            "{:?}",
            [
                RuntimeConfigApplyError::EmptyGatewayProvider,
                RuntimeConfigApplyError::CustomGatewayRequiresBaseUrl,
                RuntimeConfigApplyError::InvalidApiProtocol,
                RuntimeConfigApplyError::InvalidTokenCount {
                    field: RuntimeConfigField::MaxOutput,
                },
                RuntimeConfigApplyError::InvalidBashApproval,
                RuntimeConfigApplyError::InvalidWorkInstructions,
            ]
        );
        for forbidden in ["配置无效", "请输入", "只能", "不能为空", "\x1b["] {
            assert!(
                !debug.contains(forbidden),
                "core config errors must stay structured and UI-neutral: {debug}"
            );
        }
    }

    #[test]
    fn gateway_provider_update_keeps_dependent_defaults_consistent() {
        let mut config = test_config();
        let mut bash = BashApprovalMode::Ask;
        let mut work = WorkInstructionLoadMode::Silent;

        apply_runtime_config_value(
            &mut config,
            &mut bash,
            &mut work,
            RuntimeConfigField::GatewayProvider,
            "anthropic",
        )
        .unwrap();
        assert_eq!(config.provider, "anthropic");
        assert_eq!(config.api_protocol, ApiProtocol::Anthropic);
        assert_eq!(config.base_url, "https://api.anthropic.com");

        let err = apply_runtime_config_value(
            &mut config,
            &mut bash,
            &mut work,
            RuntimeConfigField::GatewayProvider,
            "private",
        )
        .unwrap_err();
        assert_eq!(err, RuntimeConfigApplyError::CustomGatewayRequiresBaseUrl);

        apply_runtime_config_value(
            &mut config,
            &mut bash,
            &mut work,
            RuntimeConfigField::BaseUrl,
            "https://private.example/v1",
        )
        .unwrap();
        apply_runtime_config_value(
            &mut config,
            &mut bash,
            &mut work,
            RuntimeConfigField::GatewayProvider,
            "private",
        )
        .unwrap();
        assert_eq!(config.provider, "private");
        assert_eq!(config.base_url, "https://private.example/v1");
    }
}
