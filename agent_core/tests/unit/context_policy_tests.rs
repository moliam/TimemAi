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
