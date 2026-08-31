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
#[path = "../tests/unit/context_policy_tests.rs"]
mod tests;
