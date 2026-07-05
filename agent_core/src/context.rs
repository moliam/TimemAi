pub fn estimate_prompt_context_tokens(prompt: &str) -> u32 {
    prompt.chars().count().div_ceil(4).min(u32::MAX as usize) as u32
}

#[cfg(test)]
mod tests {
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
}
