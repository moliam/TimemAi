use super::*;

#[test]
fn estimates_prompt_tokens_from_chars() {
    assert_eq!(estimate_prompt_context_tokens(""), 0);
    assert_eq!(estimate_prompt_context_tokens("a"), 1);
    assert_eq!(estimate_prompt_context_tokens("abcd"), 1);
    assert_eq!(estimate_prompt_context_tokens("abcde"), 2);
    assert_eq!(estimate_prompt_context_tokens("你好世界"), 1);
    assert_eq!(estimate_prompt_context_tokens("你好世界啊"), 2);
}
