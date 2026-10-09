//! The single boundary between tool execution output and model-visible tool results.
//!
//! Tool implementations should return the most useful result they can. Transport,
//! process and file-reading safety limits belong to those layers; prompt-size policy
//! belongs here so inline and native tool calling cannot drift apart.

pub const MAX_MODEL_TOOL_RESULT_BYTES: usize = 30 * 1024;
pub const DEFAULT_MODEL_TOOL_RESULT_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Retention {
    Head,
    Tail,
}

impl Retention {
    pub(crate) fn from_tail_out(tail_out: bool) -> Self {
        if tail_out {
            Self::Tail
        } else {
            Self::Head
        }
    }
}

pub fn validate_model_tool_result_bytes(max_bytes: usize) -> Result<(), String> {
    if matches!(max_bytes, 8_192 | 10_240 | 16_384 | 20_480 | 30_720) {
        Ok(())
    } else {
        Err("model_tool_result_bytes_invalid".to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RetainedFragment {
    pub text: String,
    pub truncated: bool,
    pub original_bytes: usize,
    pub retained_bytes: usize,
    pub retained: &'static str,
}

pub(crate) fn retain_fragment(text: &str, budget: usize, retention: Retention) -> RetainedFragment {
    let original_bytes = text.len();
    let retained_text = if original_bytes <= budget {
        text.to_string()
    } else {
        match retention {
            Retention::Head => utf8_prefix(text, budget).to_string(),
            Retention::Tail => text[utf8_suffix_start(text, budget)..].to_string(),
        }
    };
    RetainedFragment {
        retained_bytes: retained_text.len(),
        text: retained_text,
        truncated: original_bytes > budget,
        original_bytes,
        retained: match retention {
            Retention::Head => "head",
            Retention::Tail => "tail",
        },
    }
}

pub(crate) fn fit(text: &str, budget: usize, retention: Retention) -> String {
    if text.len() <= budget {
        return text.to_string();
    }
    if budget == 0 {
        return String::new();
    }

    let mut retained_budget = budget;
    loop {
        let (retained, omitted) = split_for_budget(text, retained_budget, retention);
        let omitted_words = omitted.split_whitespace().count();
        let notice = match retention {
            Retention::Head => format!(
                "!!!Too long, {omitted_words} words truncated. Generate more actions if necessary !!!"
            ),
            Retention::Tail => format!(
                "!!!Too long, {omitted_words} words truncated before. Generate more actions if necessary !!!"
            ),
        };
        if notice.len() >= budget {
            return utf8_prefix(&notice, budget).to_string();
        }
        let next_budget = budget.saturating_sub(notice.len() + 1);
        if next_budget == retained_budget {
            return match retention {
                Retention::Head => format!("{}\n{notice}", retained.trim_end()),
                Retention::Tail => format!("{notice}\n{}", retained.trim_start()),
            };
        }
        retained_budget = next_budget;
    }
}

fn split_for_budget(text: &str, budget: usize, retention: Retention) -> (&str, &str) {
    match retention {
        Retention::Head => {
            let end = utf8_prefix(text, budget).len();
            (&text[..end], &text[end..])
        }
        Retention::Tail => {
            let start = utf8_suffix_start(text, budget);
            (&text[start..], &text[..start])
        }
    }
}

fn utf8_prefix(text: &str, budget: usize) -> &str {
    let mut end = budget.min(text.len());
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

fn utf8_suffix_start(text: &str, budget: usize) -> usize {
    let mut start = text.len().saturating_sub(budget);
    while start < text.len() && !text.is_char_boundary(start) {
        start += 1;
    }
    start
}

#[cfg(test)]
#[path = "../tests/unit/tool_result_gate_tests.rs"]
mod tests;
