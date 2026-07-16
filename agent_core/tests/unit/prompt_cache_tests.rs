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
            crate::prompt_render::formatted_response_trailer("XML", "Ai3")
        );

    let parts = prompt_parts_from_rendered_prompt(&prompt);
    assert!(parts.new_delta.contains("delta1"));
    assert!(!parts
        .new_delta
        .contains("Now please continue your ID's response part"));

    let blocks = plan_prompt_cache(&prompt);
    assert_eq!(blocks.len(), 3);
    assert!(blocks[1].text.contains("delta1"));
    assert!(!blocks[1]
        .text
        .contains("Now please continue your ID's response part"));
    assert_eq!(blocks[1].cache, CacheControl::Ephemeral);
    assert_eq!(
        blocks[2].text,
        "Now please continue your ID's response part in XML as required in protocol:\n## Ai3"
    );
    assert_eq!(blocks[2].cache, CacheControl::None);
}

#[test]
fn temporary_repair_delta_is_not_cache_controlled() {
    let prompt = format!(
            "[BEGIN SYSTEM PROMPT]\nSTATIC\n[END SYSTEM PROMPT]\n[BEGIN DELTA]\ndelta_id: pd_1\n\n## USER\nnormal delta\n[END DELTA]\n[BEGIN DELTA]\ndelta_id: temp_repair_123_1\n\n## TIMEM_ASSISTANT\nwrong\n\n## SYSTEM\nrepair\n[END DELTA]\n\n{}",
            crate::prompt_render::formatted_response_trailer("Markdown", "TIMEM_ASSISTANT")
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
