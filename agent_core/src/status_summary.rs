use crate::UsageStats;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenStatusSummary {
    pub total: UsageStats,
    pub latest: Option<UsageStats>,
    pub context_tokens: u32,
    pub context_percent: u32,
    pub context_bar_filled: u32,
    pub context_bar_total: u32,
    pub model_rounds: u32,
    pub repair_calls: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenUsageBreakdown {
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cached_tokens: u32,
    pub cache_created_tokens: u32,
    pub shrunk_tokens: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeTokenStatusView {
    pub total: TokenUsageBreakdown,
    pub latest: Option<TokenUsageBreakdown>,
    pub context_tokens: u32,
    pub context_percent: u32,
    pub context_bar_filled: u32,
    pub context_bar_total: u32,
    pub model_rounds: u32,
    pub repair_calls: u32,
}

pub fn token_status_summary(
    total: &UsageStats,
    latest: Option<&UsageStats>,
    max_llm_input_tokens: u32,
    model_rounds: u32,
) -> TokenStatusSummary {
    let latest = meaningful_latest_usage(latest).cloned();
    let context_tokens = latest
        .as_ref()
        .map(|usage| usage.prompt_tokens)
        .filter(|tokens| *tokens > 0)
        .unwrap_or(total.prompt_tokens);
    let context_percent = context_percent(context_tokens, max_llm_input_tokens);
    TokenStatusSummary {
        total: total.clone(),
        latest,
        context_tokens,
        context_percent,
        context_bar_filled: context_bar_filled(context_percent, 10),
        context_bar_total: 10,
        model_rounds,
        repair_calls: total.repair_calls,
    }
}

pub fn runtime_token_status_view(
    total: &UsageStats,
    latest: Option<&UsageStats>,
    max_llm_input_tokens: u32,
    model_rounds: u32,
) -> RuntimeTokenStatusView {
    let summary = token_status_summary(total, latest, max_llm_input_tokens, model_rounds);
    RuntimeTokenStatusView {
        total: token_usage_breakdown(&summary.total),
        latest: summary.latest.as_ref().map(token_usage_breakdown),
        context_tokens: summary.context_tokens,
        context_percent: summary.context_percent,
        context_bar_filled: summary.context_bar_filled,
        context_bar_total: summary.context_bar_total,
        model_rounds: summary.model_rounds,
        repair_calls: summary.repair_calls,
    }
}

fn token_usage_breakdown(usage: &UsageStats) -> TokenUsageBreakdown {
    TokenUsageBreakdown {
        input_tokens: usage.prompt_tokens,
        output_tokens: usage.completion_tokens,
        cached_tokens: usage.cached_tokens,
        cache_created_tokens: usage.cache_created_tokens,
        shrunk_tokens: usage.shrunk_tokens,
    }
}

pub fn meaningful_latest_usage(latest: Option<&UsageStats>) -> Option<&UsageStats> {
    latest.filter(|usage| {
        usage.prompt_tokens > 0
            || usage.completion_tokens > 0
            || usage.cached_tokens > 0
            || usage.cache_created_tokens > 0
            || usage.shrunk_tokens > 0
    })
}

pub fn context_percent(context_tokens: u32, max_llm_input_tokens: u32) -> u32 {
    if context_tokens == 0 || max_llm_input_tokens == 0 {
        return 0;
    }
    let percent = context_tokens
        .saturating_mul(100)
        .saturating_add(max_llm_input_tokens - 1)
        / max_llm_input_tokens;
    percent.clamp(1, 100)
}

pub fn context_bar_filled(context_percent: u32, bar_total: u32) -> u32 {
    if context_percent == 0 || bar_total == 0 {
        return 0;
    }
    context_percent.saturating_mul(bar_total).saturating_add(99) / 100
}

#[cfg(test)]
mod tests {
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
}
