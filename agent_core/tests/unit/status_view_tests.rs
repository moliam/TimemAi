use super::*;

#[test]
fn runtime_status_snapshot_groups_retry_state_for_host_rendering() {
    let snapshot = RuntimeStatusSnapshot {
        provider: "test".to_string(),
        model: "test-model".to_string(),
        intent: "thinking".to_string(),
        memory_activity: CoreMemoryActivity::None,
        model_round: 2,
        direction: ModelDirection::Upstream,
        usage: UsageStats::zero(),
        latest_usage: None,
        tick: 0,
        elapsed_secs: 3,
        max_llm_input_tokens: 100_000,
        retry: Some(RuntimeRetryStatus {
            until_epoch_ms: Some(123),
            error: Some("provider_network_error".to_string()),
            attempt: Some(1),
            max_attempts: Some(5),
        }),
    };

    let retry = snapshot.retry.as_ref().unwrap();
    assert_eq!(retry.attempt, Some(1));
    assert_eq!(retry.max_attempts, Some(5));
    assert_eq!(retry.error.as_deref(), Some("provider_network_error"));
}

#[test]
fn runtime_active_elapsed_excludes_paused_time_and_saturates() {
    assert_eq!(
        runtime_active_elapsed_secs(Duration::from_secs(10), Duration::from_secs(3)),
        7
    );
    assert_eq!(
        runtime_active_elapsed_secs(Duration::from_secs(2), Duration::from_secs(5)),
        0
    );
}

#[test]
fn runtime_status_snapshot_keeps_memory_activity_structured_for_host_rendering() {
    let snapshot = RuntimeStatusSnapshot {
        provider: "test".to_string(),
        model: "test-model".to_string(),
        intent: "query memory".to_string(),
        memory_activity: CoreMemoryActivity::Read,
        model_round: 1,
        direction: ModelDirection::Upstream,
        usage: UsageStats::zero(),
        latest_usage: None,
        tick: 0,
        elapsed_secs: 0,
        max_llm_input_tokens: 100_000,
        retry: None,
    };

    assert_eq!(snapshot.memory_activity, CoreMemoryActivity::Read);
    let debug = format!("{snapshot:?}");
    assert!(!debug.contains('⛃'));
    assert!(!debug.contains('◂'));
    assert!(!debug.contains('▸'));
}

#[test]
fn host_status_message_is_structured_and_ui_neutral() {
    let info = HostStatusMessage::info("loaded");
    let warning = HostStatusMessage::warning("needs attention");
    let error = HostStatusMessage::error("failed");

    assert_eq!(info.level, HostStatusLevel::Info);
    assert_eq!(warning.level, HostStatusLevel::Warning);
    assert_eq!(error.level, HostStatusLevel::Error);

    let debug = format!("{info:?} {warning:?} {error:?}");
    for forbidden in ["ⓘ", "!", "\x1b"] {
        assert!(
            !debug.contains(forbidden),
            "core host status leaked shell rendering {forbidden:?}: {debug}"
        );
    }
}

#[test]
fn runtime_retry_status_view_applies_defaults_and_countdown() {
    let retry = RuntimeRetryStatus {
        until_epoch_ms: Some(12_300),
        error: Some(" provider_network_error ".to_string()),
        attempt: Some(2),
        max_attempts: Some(7),
    };
    let view = runtime_retry_status_view(&retry, 10_000);
    assert_eq!(view.remaining_secs, 3);
    assert_eq!(view.attempt, 2);
    assert_eq!(view.max_attempts, 7);
    assert_eq!(view.error.as_deref(), Some("provider_network_error"));

    let retry = RuntimeRetryStatus {
        until_epoch_ms: None,
        error: Some("   ".to_string()),
        attempt: None,
        max_attempts: None,
    };
    let view = runtime_retry_status_view(&retry, 10_000);
    assert_eq!(view.remaining_secs, 0);
    assert_eq!(view.attempt, 1);
    assert_eq!(view.max_attempts, 5);
    assert_eq!(view.error, None);
}

#[test]
fn compact_runtime_status_text_normalizes_controls_and_bounds_width() {
    assert_eq!(compact_runtime_status_text("  a\n\tb  ", 10), "a b");
    assert_eq!(compact_runtime_status_text("abc\u{0007}def", 10), "abcdef");
    assert_eq!(compact_runtime_status_text("abcdef", 4), "abc…");
    assert_eq!(compact_runtime_status_text("abcdef", 1), "…");
    assert_eq!(compact_runtime_status_text("abcdef", 0), "");
}
