pub fn estimate_prompt_context_tokens(prompt: &str) -> u32 {
    prompt.chars().count().div_ceil(4).min(u32::MAX as usize) as u32
}

#[cfg(test)]
#[path = "../tests/unit/context_tests.rs"]
mod tests;
