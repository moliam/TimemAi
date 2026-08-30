use super::*;

#[test]
fn token_summary_uses_latest_prompt_tokens_for_context_when_present() {
    let total = UsageStats {
        llm_calls: 3,
        repair_calls: 1,
        prompt_tokens: 85_000,
        completion_tokens: 3_500,
        cached_tokens: 53_900,
        ..UsageStats::zero()
    };
    let latest = UsageStats {
        prompt_tokens: 80_000,
        completion_tokens: 321,
        ..UsageStats::zero()
    };
    let summary = token_status_summary(&total, Some(&latest), 100_000, 13);
    assert_eq!(summary.context_tokens, 80_000);
    assert_eq!(summary.context_percent, 80);
    assert_eq!(summary.context_bar_filled, 8);
    assert_eq!(summary.context_bar_total, 10);
    assert_eq!(summary.model_rounds, 13);
    assert_eq!(summary.repair_calls, 1);
    assert_eq!(summary.latest, Some(latest));
}

#[test]
fn token_summary_ignores_zero_latest_usage_and_rounds_context_up() {
    let total = UsageStats {
        prompt_tokens: 4_900,
        completion_tokens: 39,
        ..UsageStats::zero()
    };
    let summary = token_status_summary(&total, Some(&UsageStats::zero()), 100_000, 1);
    assert_eq!(summary.latest, None);
    assert_eq!(summary.context_tokens, 4_900);
    assert_eq!(summary.context_percent, 5);
    assert_eq!(summary.context_bar_filled, 1);
}

#[test]
fn runtime_token_status_view_exposes_ui_neutral_breakdowns() {
    let total = UsageStats {
        llm_calls: 4,
        repair_calls: 2,
        prompt_tokens: 32_000,
        completion_tokens: 1_200,
        cached_tokens: 8_000,
        cache_created_tokens: 4_000,
        shrunk_tokens: 2_000,
        ..UsageStats::zero()
    };
    let latest = UsageStats {
        prompt_tokens: 7_000,
        completion_tokens: 120,
        cached_tokens: 3_000,
        ..UsageStats::zero()
    };

    let view = runtime_token_status_view(&total, Some(&latest), 20_000, 4);

    assert_eq!(view.total.input_tokens, 32_000);
    assert_eq!(view.total.output_tokens, 1_200);
    assert_eq!(view.total.cached_tokens, 8_000);
    assert_eq!(view.total.cache_created_tokens, 4_000);
    assert_eq!(view.total.shrunk_tokens, 2_000);
    assert_eq!(view.latest.unwrap().input_tokens, 7_000);
    assert_eq!(view.context_tokens, 7_000);
    assert_eq!(view.context_percent, 35);
    assert_eq!(view.context_bar_filled, 4);
    assert_eq!(view.context_bar_total, 10);
    assert_eq!(view.model_rounds, 4);
    assert_eq!(view.repair_calls, 2);
}
