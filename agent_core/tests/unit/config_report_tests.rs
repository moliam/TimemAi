use super::*;
use crate::ApiProtocol;

fn config() -> ModelServiceConfig {
    ModelServiceConfig {
        interaction: Default::default(),
        model: "qwen-plus".to_string(),
        base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1".to_string(),
        api_key: "secret".to_string(),
        http_headers: Default::default(),
        timeout_secs: 120,
        max_llm_output_tokens: 10_000,
        max_llm_input_tokens: 100_000,
        api_protocol: ApiProtocol::OpenAiCompatible,
        response_protocol: crate::ResponseProtocolKind::Json,
        openai_compatible: crate::OpenAiCompatibleOptions::default(),
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
    let report = runtime_config_report(&config(), input());
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
            if row.kind == RuntimeConfigRowKind::ResponseProtocol
                && row.key == "TIMEM_RESPONSE_PROTOCOL"
                && row.value == "json"
    )));
    assert!(report.items.iter().any(|item| matches!(
        item,
        RuntimeConfigReportItem::Row(row)
            if row.kind == RuntimeConfigRowKind::BaseUrl && row.key == "TIMEM_BASE_URL"
    )));
}

#[test]
fn config_report_marks_model_and_base_url_overrides_as_not_default() {
    let mut known = config();
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
}
