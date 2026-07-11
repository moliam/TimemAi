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
mod tests {
    use super::*;

    #[test]
    fn cache_planner_splits_static_and_dynamic_prompt() {
        let prompt = "[BEGIN SYSTEM PROMPT]\nSTATIC\n[END SYSTEM PROMPT]\n[BEGIN DELTA]\ndelta_id: pd_1\n\n## USER\ndelta1\n[END DELTA]";

        let blocks = plan_prompt_cache(prompt);

        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].role, PromptBlockRole::System);
        assert_eq!(blocks[0].text, "STATIC");
        assert_eq!(blocks[0].cache, CacheControl::Ephemeral);
        assert_eq!(blocks[1].role, PromptBlockRole::User);
        assert!(blocks[1].text.contains("delta1"));
        assert_eq!(blocks[1].cache, CacheControl::Ephemeral);
    }

    #[test]
    fn cache_planner_marks_only_recent_dynamic_tail() {
        let mut prompt = "[BEGIN SYSTEM PROMPT]\nSTATIC\n[END SYSTEM PROMPT]\n".to_string();
        for idx in 1..=5 {
            prompt.push_str(&format!(
                "[BEGIN DELTA]\ndelta_id: pd_{idx}\n\n## TIMEM_ASSISTANT\ndelta {idx}\n[END DELTA]\n"
            ));
        }

        let blocks = plan_prompt_cache(&prompt);
        let cached_texts = blocks
            .iter()
            .filter(|block| block.cache == CacheControl::Ephemeral)
            .map(|block| block.text.as_str())
            .collect::<Vec<_>>();

        assert!(cached_texts.iter().any(|text| *text == "STATIC"));
        assert!(!cached_texts.iter().any(|text| text.contains("delta 2")));
        assert!(cached_texts.iter().any(|text| text.contains("delta 3")));
        assert!(cached_texts.iter().any(|text| text.contains("delta 4")));
        assert!(cached_texts.iter().any(|text| text.contains("delta 5")));
    }

    #[test]
    fn cache_planner_keeps_one_delta_as_one_addressable_block() {
        let prompt = "[BEGIN SYSTEM PROMPT]\nSTATIC\n[END SYSTEM PROMPT]\n[BEGIN DELTA]\ndelta_id: pd_1\n\n## USER\nslice one\n\n## SYSTEM\nslice two\n[END DELTA]";

        let blocks = plan_prompt_cache(prompt);

        assert_eq!(blocks.len(), 2);
        assert!(blocks[1].text.contains("slice one"));
        assert!(blocks[1].text.contains("slice two"));
    }

    #[test]
    fn formatted_response_trailer_is_not_cached_or_merged_into_delta() {
        let prompt = format!(
            "[BEGIN SYSTEM PROMPT]\nSTATIC\n[END SYSTEM PROMPT]\n[BEGIN DELTA]\ndelta_id: pd_1\n\n## USER\ndelta1\n[END DELTA]\n\n{}",
            crate::prompt_render::formatted_response_trailer("XML")
        );

        let parts = prompt_parts_from_rendered_prompt(&prompt);
        assert!(parts.new_delta.contains("delta1"));
        assert!(!parts
            .new_delta
            .contains("Follow the system prompt, give your XML formatted response"));

        let blocks = plan_prompt_cache(&prompt);
        assert_eq!(blocks.len(), 3);
        assert!(blocks[1].text.contains("delta1"));
        assert!(!blocks[1]
            .text
            .contains("Follow the system prompt, give your XML formatted response"));
        assert_eq!(blocks[1].cache, CacheControl::Ephemeral);
        assert_eq!(
            blocks[2].text,
            "Follow the system prompt, give your XML formatted response. It must start with <response>:"
        );
        assert_eq!(blocks[2].cache, CacheControl::None);
    }

    #[test]
    fn temporary_repair_delta_is_not_cache_controlled() {
        let prompt = format!(
            "[BEGIN SYSTEM PROMPT]\nSTATIC\n[END SYSTEM PROMPT]\n[BEGIN DELTA]\ndelta_id: pd_1\n\n## USER\nnormal delta\n[END DELTA]\n[BEGIN DELTA]\ndelta_id: temp_repair_123_1\n\n## TIMEM_ASSISTANT\nwrong\n\n## SYSTEM\nrepair\n[END DELTA]\n\n{}",
            crate::prompt_render::formatted_response_trailer("XML")
        );

        let blocks = plan_prompt_cache(&prompt);
        let repair_block = blocks
            .iter()
            .find(|block| block.text.contains("temp_repair_123_1"))
            .expect("missing temporary repair block");

        assert_eq!(repair_block.cache, CacheControl::None);
    }

    #[derive(Debug, Default)]
    struct SimulatedCloudCache {
        stored_hashes: std::collections::HashSet<String>,
    }

    #[derive(Debug, Default, PartialEq, Eq)]
    struct SimulatedCacheUsage {
        read_chars: usize,
        created_chars: usize,
    }

    impl SimulatedCloudCache {
        fn observe(&mut self, blocks: &[PromptBlock]) -> SimulatedCacheUsage {
            let mut prefix = String::new();
            let mut prefixes = Vec::new();
            let mut cache_indexes = Vec::new();
            for (idx, block) in blocks.iter().enumerate() {
                prefix.push_str(match block.role {
                    PromptBlockRole::System => "\n<system>\n",
                    PromptBlockRole::User => "\n<user>\n",
                });
                prefix.push_str(&block.text);
                prefixes.push((stable_text_fingerprint(&prefix), prefix.chars().count()));
                if block.cache == CacheControl::Ephemeral {
                    cache_indexes.push(idx);
                }
            }

            let mut usage = SimulatedCacheUsage::default();
            let mut write_end = 0;
            for idx in cache_indexes.iter().copied() {
                let lookback_start = idx.saturating_sub(19);
                let mut best_hit = 0;
                for probe_idx in (lookback_start..=idx).rev() {
                    let (hash, prefix_chars) = &prefixes[probe_idx];
                    if self.stored_hashes.contains(hash) {
                        best_hit = *prefix_chars;
                        break;
                    }
                }
                usage.read_chars = usage.read_chars.max(best_hit);
                let (hash, prefix_chars) = &prefixes[idx];
                if !self.stored_hashes.contains(hash) {
                    write_end = write_end.max(*prefix_chars);
                }
            }
            usage.created_chars = write_end.saturating_sub(usage.read_chars);
            for idx in cache_indexes {
                self.stored_hashes.insert(prefixes[idx].0.clone());
            }
            usage
        }
    }

    fn rendered_prompt_with_stable_assistant_count(stable_count: usize) -> String {
        let mut prompt = "[BEGIN SYSTEM PROMPT]\nSTATIC\n[END SYSTEM PROMPT]\n".to_string();
        for idx in 1..=stable_count {
            prompt.push_str(&format!(
                "[BEGIN DELTA]\ndelta_id: pd_{idx}\n\n## TIMEM_ASSISTANT\nassistant stable checkpoint payload {idx}\n[END DELTA]\n"
            ));
        }
        let current = stable_count + 1;
        prompt.push_str(&format!(
            "[BEGIN DELTA]\ndelta_id: pd_{current}\n\n## USER\ncurrent user turn {current}\n[END DELTA]"
        ));
        prompt
    }

    #[test]
    fn cache_planner_improves_hits_against_simulated_cloud_cache() {
        let mut cloud = SimulatedCloudCache::default();
        let usage1 = cloud.observe(&plan_prompt_cache(
            &rendered_prompt_with_stable_assistant_count(1),
        ));
        let usage2 = cloud.observe(&plan_prompt_cache(
            &rendered_prompt_with_stable_assistant_count(2),
        ));
        let usage3 = cloud.observe(&plan_prompt_cache(
            &rendered_prompt_with_stable_assistant_count(3),
        ));
        let usage4 = cloud.observe(&plan_prompt_cache(
            &rendered_prompt_with_stable_assistant_count(4),
        ));
        let usage5 = cloud.observe(&plan_prompt_cache(
            &rendered_prompt_with_stable_assistant_count(5),
        ));
        let usage6 = cloud.observe(&plan_prompt_cache(
            &rendered_prompt_with_stable_assistant_count(6),
        ));

        assert!(usage1.created_chars > 0);
        assert_eq!(usage1.read_chars, 0);
        assert!(usage2.read_chars > 0, "static + recent tail should hit");
        assert!(usage2.created_chars > 0, "new tail block should be written");
        assert!(usage3.read_chars > 0, "previous tail prefix should hit");
        assert!(usage3.created_chars > 0, "new tail block should advance");
        assert!(usage4.read_chars > usage4.created_chars);
        assert!(
            usage5.created_chars > 0,
            "next turn should create the advanced tail block"
        );
        assert!(usage6.read_chars > usage6.created_chars);
    }

    #[test]
    fn growing_old_delta_block_would_keep_cache_hits_low() {
        let mut cloud = SimulatedCloudCache::default();
        let mut created = Vec::new();
        let mut reads = Vec::new();
        for stable_count in 1..=4 {
            let parts = prompt_parts_from_rendered_prompt(
                &rendered_prompt_with_stable_assistant_count(stable_count),
            );
            let legacy_blocks = vec![
                PromptBlock {
                    role: PromptBlockRole::System,
                    text: parts.static_prompt,
                    cache: CacheControl::Ephemeral,
                },
                PromptBlock {
                    role: PromptBlockRole::User,
                    text: parts.old_deltas,
                    cache: CacheControl::Ephemeral,
                },
                PromptBlock {
                    role: PromptBlockRole::User,
                    text: parts.new_delta,
                    cache: CacheControl::None,
                },
            ];
            let usage = cloud.observe(&legacy_blocks);
            created.push(usage.created_chars);
            reads.push(usage.read_chars);
        }

        assert!(
            created.iter().skip(1).all(|chars| *chars > 0),
            "legacy old_deltas block changes every turn and keeps creating cache"
        );
        assert!(
            reads.iter().skip(1).all(|chars| *chars < created[0] * 2),
            "legacy strategy mostly reuses only the small static block"
        );
    }
}
