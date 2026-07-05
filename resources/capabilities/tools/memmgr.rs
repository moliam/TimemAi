use crate::response_protocol::ParsedAction;
use crate::{
    compact_text, format_scratch_read_result, format_scratch_write_result,
    scratch_label_for_display, AgentCore,
};

pub fn normalize_scratch_kind(scratch_type: &str) -> String {
    match scratch_type.trim() {
        "note" | "notes" | "scratch" => "notes".to_string(),
        "context" | "context_offload" | "offload" => "context_offload".to_string(),
        other => other.to_string(),
    }
}

pub(crate) fn execute(core: &mut AgentCore, action: &ParsedAction) -> String {
    let mem_type = action.input_lower("type");
    let op = action.input_lower("op");
    let query = action.input_str("query");
    let content = action.input_str("content");
    let scratch_type = action.input_str("kind");
    let label = action.input_str("label");
    let sql = action.input_str("sql");
    let params = action.input_params();
    let id = action.input_str("id");
    let delta_ids = action.input_list("delta_ids");
    let slice_ids = action.input_list("slice_ids");
    if !slice_ids.is_empty() {
        return "Action result: memmgr\nerror: slice_ids_removed_use_delta_ids".to_string();
    }
    let limit = action.input_u64("limit").unwrap_or(5) as usize;
    let after_ms = action.input_i64("after_ms");
    let before_ms = action.input_i64("before_ms");
    let expected_version = action.input_u64("expected_version");
    match (mem_type.as_str(), op.as_str()) {
        ("durable", "query") => {
            core.current_stats.mem_reads += 1;
            let rows = core.memory.query(&query, limit).unwrap_or_default();
            if rows.is_empty() {
                format!(
                    "Action result: memmgr\ntype: durable\nop: query\nquery: {}\nresults: none",
                    query
                )
            } else {
                let lines = rows
                    .into_iter()
                    .map(|r| {
                        format!(
                            "- id={} version={} created_at_ms={} updated_at_ms={} content={}",
                            r.id, r.version, r.created_at_ms, r.updated_at_ms, r.content
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                format!(
                    "Action result: memmgr\ntype: durable\nop: query\nquery: {}\nresults:\n{}",
                    query, lines
                )
            }
        }
        ("durable", "schema") => {
            core.current_stats.mem_reads += 1;
            core.memory.schema_text(&core.chat_history)
        }
        ("durable", "sql") | ("raw_chat", "sql") => {
            core.current_stats.mem_reads += 1;
            match core
                .memory
                .sql_read(&core.chat_history, &sql, &params, limit)
            {
                Ok(rows) if rows.is_empty() => {
                    format!(
                        "Action result: memmgr\ntype: {}\nop: sql\nsql: {}\nresults: none",
                        mem_type, sql
                    )
                }
                Ok(rows) => {
                    let lines = rows
                        .into_iter()
                        .map(|row| {
                            let cells = row
                                .into_iter()
                                .map(|(column, value)| format!("{}={}", column, value))
                                .collect::<Vec<_>>()
                                .join(", ");
                            format!("- {}", cells)
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    format!(
                        "Action result: memmgr\ntype: {}\nop: sql\nsql: {}\nresults:\n{}",
                        mem_type, sql, lines
                    )
                }
                Err(err) => format!(
                    "Action result: memmgr\ntype: {}\nop: sql\nsql: {}\nerror: {}",
                    mem_type, sql, err
                ),
            }
        }
        ("durable", "insert" | "update" | "upsert" | "delete") => {
            match core.memory.update(&op, &id, &content, expected_version) {
                Ok(result) => {
                    core.current_stats.mem_writes += 1;
                    result
                }
                Err(err) => {
                    format!(
                        "Action result: memmgr\ntype: durable\nop: {}\nerror: {}",
                        op, err
                    )
                }
            }
        }
        ("raw_chat", "query") => {
            let rows = core
                .chat_history
                .query(&query, limit, after_ms, before_ms)
                .unwrap_or_default();
            let delta_rows = core.query_prompt_slices(&query, limit, after_ms, before_ms);
            if rows.is_empty() && delta_rows.is_empty() {
                format!(
                    "Action result: memmgr\ntype: raw_chat\nop: query\nquery: {}\nresults: none",
                    query
                )
            } else {
                let mut sections = Vec::new();
                if !rows.is_empty() {
                    let lines = rows
                        .into_iter()
                        .map(|record| {
                            format!(
                                "- source=chat_record time_ms={} session={} turn_id={} user={} assistant={}",
                                record.started_at_ms,
                                record.session,
                                record.turn_id,
                                compact_text(&record.user_input, 160),
                                compact_text(&record.assistant_output, 220)
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    sections.push(format!("chat_records:\n{}", lines));
                }
                if !delta_rows.is_empty() {
                    let lines = delta_rows
                        .into_iter()
                        .map(|slice| {
                            format!(
                                "- source=prompt_delta delta_id={} slice_id={} slice={}/{} prompt_type={} time_ms={} text={}",
                                slice.delta_id,
                                slice.slice_id,
                                slice.slice_index,
                                slice.slice_count,
                                slice.prompt_type,
                                slice.time_ms,
                                compact_text(&slice.text, 240)
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    sections.push(format!("current_prompt_deltas:\n{}", lines));
                }
                format!(
                    "Action result: memmgr\ntype: raw_chat\nop: query\nquery: {}\nresults:\n{}",
                    query,
                    sections.join("\n")
                )
            }
        }
        ("raw_chat", "delete") => {
            match core.chat_history.delete(&id, &query, limit, after_ms, before_ms) {
                Ok(deleted) => format!(
                    "Action result: memmgr\ntype: raw_chat\nop: delete\nid: {}\nquery: {}\ndeleted_count: {}",
                    id, query, deleted
                ),
                Err(err) => {
                    format!("Action result: memmgr\ntype: raw_chat\nop: delete\nerror: {}", err)
                }
            }
        }
        ("scratch", "write") => {
            let scratch_type = normalize_scratch_kind(&scratch_type);
            let write_result = if scratch_type == "context_offload" {
                core.collect_prompt_context_for_scratch(&delta_ids, &[])
                    .and_then(|offload| {
                        core.scratch.write_record(
                            &scratch_type,
                            &label,
                            &offload.content,
                            &offload.delta_ids,
                            &offload.slice_ids,
                        )
                    })
            } else {
                core.scratch
                    .write_record(&scratch_type, &label, &content, &[], &[])
            };
            match write_result {
                Ok(record) => format_scratch_write_result(&record),
                Err(err) => format!(
                    "Action result: memmgr\ntype: scratch\nop: write\nerror: {}",
                    err
                ),
            }
        }
        ("scratch", "read") => match core.scratch.read(&id) {
            Ok(Some(record)) => format_scratch_read_result(&record),
            Ok(None) => format!(
                "Action result: memmgr\ntype: scratch\nop: read\nid: {}\nfound: false",
                id
            ),
            Err(err) => format!(
                "Action result: memmgr\ntype: scratch\nop: read\nerror: {}",
                err
            ),
        },
        ("scratch", "query") => match core.scratch.query(&query, limit) {
            Ok(rows) if rows.is_empty() => format!(
                "Action result: memmgr\ntype: scratch\nop: query\nquery: {}\nresults: none",
                query
            ),
            Ok(rows) => {
                let lines = rows
                    .into_iter()
                    .map(|row| {
                        format!(
                            "- id={} label={} type={} time_ms={} content_preview={}",
                            row.id,
                            scratch_label_for_display(&row),
                            normalize_scratch_kind(&row.scratch_type),
                            row.created_at_ms,
                            compact_text(&row.content, 240)
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                format!(
                    "Action result: memmgr\ntype: scratch\nop: query\nquery: {}\nresults:\n{}",
                    query, lines
                )
            }
            Err(err) => format!(
                "Action result: memmgr\ntype: scratch\nop: query\nerror: {}",
                err
            ),
        },
        ("scratch", "delete") => match core.scratch.delete(&id) {
            Ok(true) => format!(
                "Action result: memmgr\ntype: scratch\nop: delete\nid: {}\ndeleted: true",
                id
            ),
            Ok(false) => format!(
                "Action result: memmgr\ntype: scratch\nop: delete\nid: {}\ndeleted: false",
                id
            ),
            Err(err) => format!(
                "Action result: memmgr\ntype: scratch\nop: delete\nerror: {}",
                err
            ),
        },
        ("context", "shrink") => core.apply_prompt_shrink(
            "Action result: memmgr\ntype: context\nop: shrink",
            &delta_ids,
            &[],
        ),
        _ => format!(
            "Action result: memmgr\ntype: {}\nop: {}\nerror: unsupported_type_or_op",
            mem_type, op
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scratch_kind_aliases_are_normalized() {
        assert_eq!(normalize_scratch_kind("note"), "notes");
        assert_eq!(normalize_scratch_kind("context"), "context_offload");
        assert_eq!(normalize_scratch_kind("custom"), "custom");
    }
}
