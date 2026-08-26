use crate::response_protocol::ParsedAction;
use crate::{
    compact_text, format_scratch_read_result, format_scratch_write_result,
    scratch_label_for_display, ActionOutcome, AgentCore, MemmgrResultEvidence,
};

fn memmgr_evidence_content(text: &str, memory_type: &str, op: &str) -> String {
    let prefix = format!("Action result: memmgr\ntype: {memory_type}\nop: {op}\n");
    text.strip_prefix(&prefix).unwrap_or(text).to_string()
}

fn durable_update_error_type(error: &str) -> &'static str {
    if error.starts_with("memory_conflict ") {
        "MemoryConflict"
    } else if error.starts_with("missing_expected_version ") {
        "MissingExpectedVersion"
    } else {
        match error {
            "id_not_found" => "NotFound",
            "content_required"
            | "id_required"
            | "operation_must_be_insert_update_upsert_or_delete" => "InvalidInput",
            _ => "StorageError",
        }
    }
}

pub fn normalize_scratch_kind(scratch_type: &str) -> String {
    match scratch_type.trim() {
        "note" | "notes" | "scratch" => "notes".to_string(),
        other => other.to_string(),
    }
}

pub(crate) fn execute_outcome(core: &mut AgentCore, action: &ParsedAction) -> ActionOutcome {
    let mem_type = action.input_lower("type");
    let op = action.input_lower("op");
    let search_text = action.input_str("search_text");
    let content = action.input_str("content");
    let scratch_type = action.input_str("kind");
    let label = action.input_str("label");
    let sql = action.input_str("sql");
    let params = action.input_params();
    let id = action.input_str("id");
    let limit = action.input_u64("limit").unwrap_or(5) as usize;
    let after_ms = action.input_i64("after_ms");
    let before_ms = action.input_i64("before_ms");
    let expected_version = action.input_u64("expected_version");

    let mut evidence_error_type = None;
    let outcome = match (mem_type.as_str(), op.as_str()) {
        ("durable", "schema") => {
            core.current_stats.mem_reads += 1;
            ActionOutcome::completed(core.memory.schema_text(&core.chat_history))
        }
        ("durable", "sql") | ("raw_chat", "sql") => {
            core.current_stats.mem_reads += 1;
            match core
                .memory
                .sql_read(&core.chat_history, &sql, &params, limit)
            {
                Ok(rows) if rows.is_empty() => {
                    let text = if mem_type == "durable" {
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
                    };
                    ActionOutcome::completed(text)
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
                    ActionOutcome::completed(format!(
                        "Action result: memmgr\ntype: {}\nop: sql\nsql: {}\nresults:\n{}",
                        mem_type, sql, lines
                    ))
                }
                Err(err) => {
                    evidence_error_type = Some("StorageError".to_string());
                    ActionOutcome::failed(format!(
                        "Action result: memmgr\ntype: {}\nop: sql\nsql: {}\nerror: {}",
                        mem_type, sql, err
                    ))
                }
            }
        }
        ("durable", "insert" | "update" | "upsert" | "delete") => {
            match core.memory.update(&op, &id, &content, expected_version) {
                Ok(result) => {
                    core.current_stats.mem_writes += 1;
                    ActionOutcome::completed(result)
                }
                Err(err) => {
                    evidence_error_type = Some(durable_update_error_type(&err).to_string());
                    ActionOutcome::failed(format!(
                        "Action result: memmgr\ntype: durable\nop: {}\nerror: {}",
                        op, err
                    ))
                }
            }
        }
        ("raw_chat", "search") => {
            let rows = match core
                .chat_history
                .query(&search_text, limit, after_ms, before_ms)
            {
                Ok(rows) => rows,
                Err(err) => {
                    let text = format!(
                        "Action result: memmgr\ntype: raw_chat\nop: search\nerror: {}",
                        err
                    );
                    return ActionOutcome::failed(text.clone()).with_memmgr_result(
                        MemmgrResultEvidence {
                            memory_type: mem_type,
                            op,
                            content: memmgr_evidence_content(&text, "raw_chat", "search"),
                            error_type: Some("StorageError".to_string()),
                        },
                    );
                }
            };
            let delta_rows = core.query_prompt_slices(&search_text, limit, after_ms, before_ms);
            if rows.is_empty() && delta_rows.is_empty() {
                ActionOutcome::completed(format!(
                    "Action result: memmgr\ntype: raw_chat\nop: search\nsearch_text: {}\nresults: none",
                    search_text
                ))
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
                ActionOutcome::completed(format!(
                    "Action result: memmgr\ntype: raw_chat\nop: search\nsearch_text: {}\nresults:\n{}",
                    search_text,
                    sections.join("\n")
                ))
            }
        }
        ("raw_chat", "delete") => {
            match core
                .chat_history
                .delete(&id, &search_text, limit, after_ms, before_ms)
            {
                Ok(deleted) => ActionOutcome::completed(format!(
                    "Action result: memmgr\ntype: raw_chat\nop: delete\nid: {}\nsearch_text: {}\ndeleted_count: {}",
                    id, search_text, deleted
                )),
                Err(err) => {
                    evidence_error_type = Some("StorageError".to_string());
                    ActionOutcome::failed(format!(
                        "Action result: memmgr\ntype: raw_chat\nop: delete\nerror: {}",
                        err
                    ))
                }
            }
        }
        ("scratch", "write") => {
            let scratch_type = normalize_scratch_kind(&scratch_type);
            match core
                .scratch
                .write_record(&scratch_type, &label, &content, &[], &[])
            {
                Ok(record) => ActionOutcome::completed(format_scratch_write_result(&record)),
                Err(err) => {
                    evidence_error_type = Some("StorageError".to_string());
                    ActionOutcome::failed(format!(
                        "Action result: memmgr\ntype: scratch\nop: write\nerror: {}",
                        err
                    ))
                }
            }
        }
        ("scratch", "read") => match core.scratch.read(&id) {
            Ok(Some(record)) => ActionOutcome::completed(format_scratch_read_result(&record)),
            Ok(None) => ActionOutcome::completed(format!(
                "Action result: memmgr\ntype: scratch\nop: read\nid: {}\nfound: false",
                id
            )),
            Err(err) => {
                evidence_error_type = Some("StorageError".to_string());
                ActionOutcome::failed(format!(
                    "Action result: memmgr\ntype: scratch\nop: read\nerror: {}",
                    err
                ))
            }
        },
        ("scratch", "search") => match core.scratch.query(&search_text, limit) {
            Ok(rows) if rows.is_empty() => ActionOutcome::completed(format!(
                "Action result: memmgr\ntype: scratch\nop: search\nsearch_text: {}\nresults: none",
                search_text
            )),
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
                ActionOutcome::completed(format!(
                        "Action result: memmgr\ntype: scratch\nop: search\nsearch_text: {}\nresults:\n{}",
                        search_text, lines
                    ))
            }
            Err(err) => {
                evidence_error_type = Some("StorageError".to_string());
                ActionOutcome::failed(format!(
                    "Action result: memmgr\ntype: scratch\nop: search\nerror: {}",
                    err
                ))
            }
        },
        ("scratch", "delete") => match core.scratch.delete(&id) {
            Ok(true) => ActionOutcome::completed(format!(
                "Action result: memmgr\ntype: scratch\nop: delete\nid: {}\ndeleted: true",
                id
            )),
            Ok(false) => ActionOutcome::completed(format!(
                "Action result: memmgr\ntype: scratch\nop: delete\nid: {}\ndeleted: false",
                id
            )),
            Err(err) => {
                evidence_error_type = Some("StorageError".to_string());
                ActionOutcome::failed(format!(
                    "Action result: memmgr\ntype: scratch\nop: delete\nerror: {}",
                    err
                ))
            }
        },
        _ => {
            evidence_error_type = Some("InvalidInput".to_string());
            ActionOutcome::failed(format!(
                "Action result: memmgr\ntype: {}\nop: {}\nerror: unsupported_type_or_op",
                mem_type, op
            ))
        }
    };

    let evidence_content = memmgr_evidence_content(&outcome.text, &mem_type, &op);
    outcome.with_memmgr_result(MemmgrResultEvidence {
        memory_type: mem_type,
        op,
        content: evidence_content,
        error_type: evidence_error_type,
    })
}

#[cfg(test)]
#[path = "../../../agent_core/tests/unit/capability_tool_memmgr_tests.rs"]
mod tests;
