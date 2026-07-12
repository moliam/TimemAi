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
