use std::time::Duration;

pub const DEFAULT_STALE_CONTEXT_IDLE: Duration = Duration::from_secs(3 * 60 * 60);
pub const DEFAULT_STALE_CONTEXT_TOKEN_THRESHOLD: u32 = 10_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StaleContextPolicy {
    pub idle_threshold: Duration,
    pub token_threshold: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StaleContextDecisionRequest {
    pub idle: Duration,
    pub dynamic_context_tokens: u32,
    pub continue_keeps_dynamic_context: bool,
    pub decline_clears_dynamic_context: bool,
}

impl Default for StaleContextPolicy {
    fn default() -> Self {
        Self {
            idle_threshold: DEFAULT_STALE_CONTEXT_IDLE,
            token_threshold: DEFAULT_STALE_CONTEXT_TOKEN_THRESHOLD,
        }
    }
}

impl StaleContextPolicy {
    pub fn should_prompt(self, idle: Duration, dynamic_context_tokens: u32) -> bool {
        idle >= self.idle_threshold && dynamic_context_tokens > self.token_threshold
    }

    pub fn decision_request(
        self,
        idle: Duration,
        dynamic_context_tokens: u32,
    ) -> Option<StaleContextDecisionRequest> {
        self.should_prompt(idle, dynamic_context_tokens)
            .then_some(StaleContextDecisionRequest {
                idle,
                dynamic_context_tokens,
                continue_keeps_dynamic_context: true,
                decline_clears_dynamic_context: true,
            })
    }
}

pub fn stale_context_prompt_needed(idle: Duration, dynamic_context_tokens: u32) -> bool {
    StaleContextPolicy::default().should_prompt(idle, dynamic_context_tokens)
}

pub fn stale_context_decision_request(
    idle: Duration,
    dynamic_context_tokens: u32,
) -> Option<StaleContextDecisionRequest> {
    StaleContextPolicy::default().decision_request(idle, dynamic_context_tokens)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_context_prompt_requires_idle_time_and_large_context() {
        assert!(!stale_context_prompt_needed(
            DEFAULT_STALE_CONTEXT_IDLE - Duration::from_secs(1),
            DEFAULT_STALE_CONTEXT_TOKEN_THRESHOLD + 1
        ));
        assert!(!stale_context_prompt_needed(
            DEFAULT_STALE_CONTEXT_IDLE,
            DEFAULT_STALE_CONTEXT_TOKEN_THRESHOLD
        ));
        assert!(stale_context_prompt_needed(
            DEFAULT_STALE_CONTEXT_IDLE,
            DEFAULT_STALE_CONTEXT_TOKEN_THRESHOLD + 1
        ));
    }

    #[test]
    fn stale_context_decision_request_is_structured_and_ui_neutral() {
        let request = stale_context_decision_request(
            DEFAULT_STALE_CONTEXT_IDLE,
            DEFAULT_STALE_CONTEXT_TOKEN_THRESHOLD + 1,
        )
        .unwrap();

        assert_eq!(request.idle, DEFAULT_STALE_CONTEXT_IDLE);
        assert_eq!(
            request.dynamic_context_tokens,
            DEFAULT_STALE_CONTEXT_TOKEN_THRESHOLD + 1
        );
        assert!(request.continue_keeps_dynamic_context);
        assert!(request.decline_clears_dynamic_context);

        let debug = format!("{request:?}");
        for forbidden in ["YES", "NO", "继续", "清空", "\x1b", "["] {
            assert!(
                !debug.contains(forbidden),
                "core stale context request leaked UI text {forbidden:?}: {debug}"
            );
        }
    }
}
