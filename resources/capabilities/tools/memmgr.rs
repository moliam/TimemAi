use crate::response_protocol::ParsedAction;
use crate::{
    compact_text, format_scratch_read_result, format_scratch_write_result,
    scratch_label_for_display, AgentCore,
};

pub fn normalize_scratch_kind(scratch_type: &str) -> String {
    match scratch_type.trim() {
        "note" | "notes" | "scratch" => "notes".to_string(),
        other => other.to_string(),
    }
}

pub(crate) fn execute(core: &mut AgentCore, action: &ParsedAction) -> String {
    let mem_type = action.input_lower("type");
    let op = action.input_lower("op");
    let search_text = action.input_str("search_text");
    let content = action.input_str("content");
    let scratch_type = action.input_str("kind");
    let label = action.input_str("label");
    let sql = action.input_str("sql");
    let params = action.input_params();
    let id = action.input_str("id");
    let slice_ids = action.input_list("slice_ids");
    if !slice_ids.is_empty() {
        return "Action result: memmgr\nerror: slice_ids_removed_use_delta_ids".to_string();
    }
    let limit = action.input_u64("limit").unwrap_or(5) as usize;
    let after_ms = action.input_i64("after_ms");
    let before_ms = action.input_i64("before_ms");
    let expected_version = action.input_u64("expected_version");
    match (mem_type.as_str(), op.as_str()) {
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
                    if mem_type == "durable" {
                        let total_rows = core.memory.count().unwrap_or_default();
                        format!(
                            "Action result: memmgr\ntype: durable\nop: sql\nsql: {}\nresults: none\ndurable_memory_total_rows: {}",
                            sql, total_rows
                        )
                    } else {
                        format!(
                            "Action result: memmgr\ntype: {}\nop: sql\nsql: {}\nresults: none",
                            mem_type, sql
                        )
                    }
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
        ("raw_chat", "search") => {
            let rows = core
                .chat_history
                .query(&search_text, limit, after_ms, before_ms)
                .unwrap_or_default();
            let delta_rows = core.query_prompt_slices(&search_text, limit, after_ms, before_ms);
            if rows.is_empty() && delta_rows.is_empty() {
                format!(
                    "Action result: memmgr\ntype: raw_chat\nop: search\nsearch_text: {}\nresults: none",
                    search_text
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
                    "Action result: memmgr\ntype: raw_chat\nop: search\nsearch_text: {}\nresults:\n{}",
                    search_text,
                    sections.join("\n")
                )
            }
        }
        ("raw_chat", "delete") => {
            match core
                .chat_history
                .delete(&id, &search_text, limit, after_ms, before_ms)
            {
                Ok(deleted) => format!(
                    "Action result: memmgr\ntype: raw_chat\nop: delete\nid: {}\nsearch_text: {}\ndeleted_count: {}",
                    id, search_text, deleted
                ),
                Err(err) => {
                    format!("Action result: memmgr\ntype: raw_chat\nop: delete\nerror: {}", err)
                }
            }
        }
        ("scratch", "write") => {
            let scratch_type = normalize_scratch_kind(&scratch_type);
            let write_result = core
                .scratch
                .write_record(&scratch_type, &label, &content, &[], &[]);
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
        ("scratch", "search") => match core.scratch.query(&search_text, limit) {
            Ok(rows) if rows.is_empty() => format!(
                "Action result: memmgr\ntype: scratch\nop: search\nsearch_text: {}\nresults: none",
                search_text
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
                    "Action result: memmgr\ntype: scratch\nop: search\nsearch_text: {}\nresults:\n{}",
                    search_text, lines
                )
            }
            Err(err) => format!(
                "Action result: memmgr\ntype: scratch\nop: search\nerror: {}",
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
        assert_eq!(normalize_scratch_kind("custom"), "custom");
    }
}
