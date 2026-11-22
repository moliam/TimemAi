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
    let scope = action.input_lower("scope");
    let requested_session_id = action.input_str("session_id");
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
            ActionOutcome::completed(core.memory.schema_text())
        }
        ("durable", "sql") => {
            core.current_stats.mem_reads += 1;
            match core.memory.sql_read(&sql, &params, limit) {
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
                        mem_type,
                        sql,
                        sql_error_prose(&err)
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
                        op,
                        durable_error_prose(&op, &err)
                    ))
                }
            }
        }
        ("raw_chat", "search") => {
            let scope = if scope.is_empty() {
                "current_session".to_string()
            } else {
                scope
            };
            let session_scope = match scope.as_str() {
                "current_session" if requested_session_id.is_empty() => {
                    Some(core.current_session_id())
                }
                "session" if !requested_session_id.trim().is_empty() => {
                    Some(requested_session_id.clone())
                }
                "global" if requested_session_id.is_empty() => None,
                "current_session" => {
                    return ActionOutcome::failed(
                        "Action result: memmgr\ntype: raw_chat\nop: search\nerror: \
                         scope=\"current_session\" searches this session only, so it must not \
                         also carry a session_id (detail: session_id_not_allowed). Drop \
                         session_id, or use scope=\"session\" with the target session id.",
                    )
                }
                "session" => {
                    return ActionOutcome::failed(
                        "Action result: memmgr\ntype: raw_chat\nop: search\nerror: \
                         scope=\"session\" needs a non-empty session_id naming the target \
                         session (detail: session_id_required). Provide it and retry.",
                    )
                }
                _ => {
                    return ActionOutcome::failed(
                        "Action result: memmgr\ntype: raw_chat\nop: search\nerror: scope \
                         must be \"current_session\", \"session\", or \"global\" (detail: \
                         invalid_raw_chat_scope). Use one of those values and retry.",
                    )
                }
            };
            let rows = match core.chat_history.query(
                &search_text,
                limit,
                after_ms,
                before_ms,
                session_scope.as_deref(),
            ) {
                Ok(rows) => rows,
                Err(err) => {
                    let text = format!(
                        "Action result: memmgr\ntype: raw_chat\nop: search\nerror: The raw \
                         chat search could not be read from storage: {err}",
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
            let delta_rows = core
                .query_prompt_slices(
                    &search_text,
                    limit.saturating_mul(4).clamp(4, 50),
                    after_ms,
                    before_ms,
                )
                .into_iter()
                .filter(|slice| {
                    matches!(
                        slice.prompt_type.as_str(),
                        "user_question" | "user_supplement" | "user_resume_directly"
                    )
                })
                .take(limit)
                .collect::<Vec<_>>();
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
                        .flat_map(|record| {
                            let mut messages = Vec::new();
                            if !record.user_input.trim().is_empty() {
                                messages.push(format!("- source=chat_history session_id={} turn_id={} role=user created_at_ms={} content={}", record.session, record.turn_id, record.started_at_ms, compact_text(&record.user_input, 240)));
                            }
                            if !record.assistant_output.trim().is_empty() {
                                messages.push(format!("- source=chat_history session_id={} turn_id={} role=assistant created_at_ms={} content={}", record.session, record.turn_id, record.started_at_ms, compact_text(&record.assistant_output, 320)));
                            }
                            messages
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
                        "Action result: memmgr\ntype: raw_chat\nop: delete\nerror: The raw \
                         chat delete could not be applied to storage: {err}",
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
                        "Action result: memmgr\ntype: scratch\nop: write\nerror: The scratch \
                         note could not be written to storage: {err}",
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
                    "Action result: memmgr\ntype: scratch\nop: read\nerror: The scratch note \
                     could not be read from storage: {err}",
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
                    "Action result: memmgr\ntype: scratch\nop: search\nerror: The scratch \
                     search could not be read from storage: {err}",
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
                    "Action result: memmgr\ntype: scratch\nop: delete\nerror: The scratch \
                     note could not be deleted from storage: {err}",
                ))
            }
        },
        _ => {
            evidence_error_type = Some("InvalidInput".to_string());
            ActionOutcome::failed(format!(
                "Action result: memmgr\ntype: {}\nop: {}\nerror: No memory operation was \
                 performed: type/op must be durable(schema|sql|insert|update|upsert|delete), \
                 raw_chat(search|delete), or scratch(search|write|read|delete) (detail: \
                 unsupported_type_or_op). Use one of those combinations.",
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

fn sql_error_prose(err: &str) -> String {
    format!(
        "Cannot run the SQL query ({err}). memmgr SQL accepts SELECT, WITH ... SELECT, or \
         PRAGMA table_info(memories) only; writes go through type=durable ops."
    )
}

fn durable_error_prose(op: &str, err: &str) -> String {
    match err {
        "content_required" => {
            "The durable memory write needs a non-empty `content` (detail: content_required). \
             Provide the content and retry."
                .to_string()
        }
        "id_required" => {
            "The durable memory operation needs the row `id` (detail: id_required). Read rows \
             first with type=durable op=sql and retry with an existing id."
                .to_string()
        }
        "id_not_found" => {
            "No durable memory row with this id exists (detail: id_not_found). List rows with \
             type=durable op=sql, then retry with an existing id."
                .to_string()
        }
        "operation_must_be_insert_update_upsert_or_delete" => {
            "The durable op must be insert, update, upsert, or delete (detail: \
             operation_must_be_insert_update_upsert_or_delete). Use one of those ops."
                .to_string()
        }
        _ if err.starts_with("memory_conflict ") => format!(
            "The durable memory row changed since you read it, so the write was rejected. \
             Re-read the row with type=durable op=sql and retry with \
             expected_version=current_version (detail: {err})."
        ),
        _ if err.starts_with("missing_expected_version ") => format!(
            "Updating or deleting an existing durable row requires expected_version from the \
             latest type=durable op=sql read (detail: {err}). Read the row first, then retry \
             with expected_version=current_version."
        ),
        _ => format!("Cannot {op} the durable memory: {err}"),
    }
}

#[cfg(test)]
#[path = "../../../core/agent/tests/unit/capability_tool_memmgr_tests.rs"]
mod tests;
