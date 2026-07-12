use crate::prompt_render::split_formatted_response_trailer;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptBlockRole {
    System,
    User,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheControl {
    None,
    Ephemeral,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptBlock {
    pub role: PromptBlockRole,
    pub text: String,
    pub cache: CacheControl,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptParts {
    pub static_prompt: String,
    pub old_deltas: String,
    pub new_delta: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PromptSegment {
    text: String,
}

const DYNAMIC_TAIL_CACHE_BLOCKS: usize = 3;

pub fn split_prompt(full_prompt: &str) -> (String, String) {
    const BEGIN_MARKER: &str = "[BEGIN SYSTEM PROMPT]";
    const END_MARKER: &str = "[END SYSTEM PROMPT]";
    let Some(begin_idx) = full_prompt.find(BEGIN_MARKER) else {
        return (String::new(), full_prompt.to_string());
    };
    let content_start = begin_idx + BEGIN_MARKER.len();
    let Some(end_idx) = full_prompt[content_start..].find(END_MARKER) else {
        return (String::new(), full_prompt.to_string());
    };
    let static_content = full_prompt[content_start..content_start + end_idx]
        .trim()
        .to_string();
    let dynamic_part = full_prompt[content_start + end_idx + END_MARKER.len()..]
        .trim_start_matches(['\n', '\r', ' ', '\t'])
        .to_string();
    (static_content, dynamic_part)
}

pub fn split_old_and_new_delta(dynamic_prompt: &str) -> (String, String) {
    let dynamic_prompt = dynamic_prompt.trim();
    if dynamic_prompt.is_empty() {
        return (String::new(), String::new());
    }
    let starts = prompt_delta_segment_starts(dynamic_prompt);
    let Some(last_start) = starts.last().copied() else {
        return (String::new(), dynamic_prompt.to_string());
    };
    let last_delta_id = segment_delta_id(&dynamic_prompt[last_start..]);
    if let Some(last_delta_id) = last_delta_id {
        for start in starts {
            if segment_delta_id(&dynamic_prompt[start..]).as_deref() == Some(last_delta_id.as_str())
            {
                let old_deltas = dynamic_prompt[..start].trim_end().to_string();
                let new_delta = dynamic_prompt[start..].trim_start().to_string();
                return (old_deltas, new_delta);
            }
        }
    }
    let old_deltas = dynamic_prompt[..last_start].trim_end().to_string();
    let new_delta = dynamic_prompt[last_start..].trim_start().to_string();
    (old_deltas, new_delta)
}

pub fn prompt_parts_from_rendered_prompt(rendered_prompt: &str) -> PromptParts {
    let rendered_prompt = split_formatted_response_trailer(rendered_prompt).0;
    let (static_prompt, dynamic_prompt) = split_prompt(rendered_prompt);
    let dynamic_prompt = if dynamic_prompt.is_empty() {
        rendered_prompt.to_string()
    } else {
        dynamic_prompt
    };
    let static_prompt = if static_prompt.is_empty() {
        rendered_prompt.to_string()
    } else {
        static_prompt
    };
    let (old_deltas, new_delta) = split_old_and_new_delta(&dynamic_prompt);
    let new_delta = if new_delta.is_empty() {
        dynamic_prompt
    } else {
        new_delta
    };
    PromptParts {
        static_prompt,
        old_deltas,
        new_delta,
    }
}

pub fn plan_incremental_cache(parts: PromptParts) -> Vec<PromptBlock> {
    let mut blocks = vec![PromptBlock {
        role: PromptBlockRole::System,
        text: parts.static_prompt,
        cache: CacheControl::Ephemeral,
    }];

    let dynamic = [parts.old_deltas.as_str(), parts.new_delta.as_str()]
        .into_iter()
        .filter(|part| !part.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    if dynamic.trim().is_empty() {
        return blocks;
    }

    let segments = split_prompt_segments(&dynamic);
    let cache_indexes = cache_tail_indexes(&segments);
    blocks.extend(segments.into_iter().enumerate().map(|(idx, segment)| {
        let cache = if is_temporary_prompt_segment(&segment) {
            CacheControl::None
        } else if cache_indexes.contains(&idx) {
            CacheControl::Ephemeral
        } else {
            CacheControl::None
        };
        PromptBlock {
            role: PromptBlockRole::User,
            text: segment.text,
            cache,
        }
    }));
    blocks
}

fn is_temporary_prompt_segment(segment: &PromptSegment) -> bool {
    segment_delta_id(&segment.text)
        .as_deref()
        .is_some_and(|id| id.starts_with("temp_"))
}

pub fn plan_prompt_cache(rendered_prompt: &str) -> Vec<PromptBlock> {
    let (prompt_without_trailer, trailer) = split_formatted_response_trailer(rendered_prompt);
    let mut blocks =
        plan_incremental_cache(prompt_parts_from_rendered_prompt(prompt_without_trailer));
    if let Some(trailer) = trailer {
        blocks.push(PromptBlock {
            role: PromptBlockRole::User,
            text: trailer,
            cache: CacheControl::None,
        });
    }
    blocks
}

pub fn stable_text_fingerprint(text: &str) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in text.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn prompt_delta_segment_starts(text: &str) -> Vec<usize> {
    let mut starts = Vec::new();
    if text.starts_with("[BEGIN DELTA]") || text.starts_with("[BEGIN SEGMENT ") {
        starts.push(0);
    }
    for marker in ["\n[BEGIN DELTA]", "\n[BEGIN SEGMENT "] {
        let mut offset = 0;
        while let Some(relative) = text[offset..].find(marker) {
            let start = offset + relative + 1;
            if !starts.contains(&start) {
                starts.push(start);
            }
            offset = start + 1;
        }
    }
    starts.sort_unstable();
    starts
}

fn segment_delta_id(segment: &str) -> Option<String> {
    segment.lines().find_map(|line| {
        line.strip_prefix("delta_id:")
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    })
}

fn split_prompt_segments(dynamic_prompt: &str) -> Vec<PromptSegment> {
    let dynamic_prompt = dynamic_prompt.trim();
    if dynamic_prompt.is_empty() {
        return Vec::new();
    }
    let starts = prompt_delta_segment_starts(dynamic_prompt);
    if starts.is_empty() {
        return vec![PromptSegment {
            text: dynamic_prompt.to_string(),
        }];
    }

    starts
        .iter()
        .enumerate()
        .map(|(idx, start)| {
            let end = starts.get(idx + 1).copied().unwrap_or(dynamic_prompt.len());
            let text = dynamic_prompt[*start..end].trim().to_string();
            PromptSegment { text }
        })
        .collect()
}

fn cache_tail_indexes(segments: &[PromptSegment]) -> Vec<usize> {
    if segments.is_empty() {
        return Vec::new();
    }
    let first_tail = segments.len().saturating_sub(DYNAMIC_TAIL_CACHE_BLOCKS);
    (first_tail..segments.len()).collect()
}

#[cfg(test)]
#[path = "../tests/unit/prompt_cache_tests.rs"]
mod tests;
