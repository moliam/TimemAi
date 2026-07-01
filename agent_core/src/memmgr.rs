#[derive(Debug, Clone, Copy)]
pub struct MemmgrActionInput<'a> {
    pub idx: usize,
    pub mem_type: &'a str,
    pub op: &'a str,
    pub query: &'a str,
    pub content: &'a str,
    pub scratch_type: &'a str,
    pub label: &'a str,
    pub sql: &'a str,
    pub params: &'a [String],
    pub id: &'a str,
    pub delta_ids: &'a [String],
    pub slice_ids: &'a [String],
}

pub fn validate_action(input: MemmgrActionInput<'_>) -> Result<(), String> {
    if input.mem_type.is_empty() {
        return Err(issue(input.idx, "type_required"));
    }
    if input.op.is_empty() {
        return Err(issue(input.idx, "op_required"));
    }
    match (input.mem_type, input.op) {
        ("durable", "query") => {
            if input.query.is_empty() {
                return Err(issue(input.idx, "query_required"));
            }
        }
        ("durable", "schema") => {}
        ("durable" | "raw_chat", "sql") => {
            if input.sql.is_empty() {
                return Err(issue(input.idx, "sql_required"));
            }
            let placeholder_count = input.sql.matches('?').count();
            if input.params.len() != placeholder_count {
                return Err(format!(
                    "next_actions[{}].input.params_count_mismatch expected={placeholder_count} actual={}",
                    input.idx,
                    input.params.len()
                ));
            }
        }
        ("durable", "insert" | "update" | "upsert") => {
            if input.content.is_empty() {
                return Err(issue(input.idx, "content_required"));
            }
            if matches!(input.op, "update") && input.id.is_empty() {
                return Err(issue(input.idx, "id_required"));
            }
        }
        ("durable", "delete") => {
            if input.id.is_empty() {
                return Err(issue(input.idx, "id_required"));
            }
        }
        ("raw_chat", "query") => {}
        ("raw_chat", "delete") => {
            if input.id.is_empty() && input.query.is_empty() {
                return Err(issue(input.idx, "id_or_query_required"));
            }
        }
        ("scratch", "write") => {
            let normalized_type = normalize_scratch_kind(input.scratch_type);
            if input.scratch_type.trim().is_empty() {
                return Err(issue(input.idx, "kind_required"));
            }
            if !matches!(normalized_type.as_str(), "notes" | "context_offload") {
                return Err(format!(
                    "next_actions[{}].input.kind_unsupported:{}",
                    input.idx, input.scratch_type
                ));
            }
            if input.label.is_empty() {
                return Err(issue(input.idx, "label_required"));
            }
            if normalized_type == "notes" && input.content.is_empty() {
                return Err(issue(input.idx, "content_required"));
            }
            if normalized_type == "context_offload"
                && input.delta_ids.is_empty()
                && input.slice_ids.is_empty()
            {
                return Err(issue(input.idx, "prompt_refs_required"));
            }
        }
        ("scratch", "read" | "delete") => {
            if input.id.is_empty() {
                return Err(issue(input.idx, "id_required"));
            }
        }
        ("scratch", "query") => {}
        ("context", "shrink") => {
            if input.delta_ids.is_empty() && input.slice_ids.is_empty() {
                return Err(issue(input.idx, "ids_required"));
            }
        }
        _ => {
            return Err(format!(
                "next_actions[{}].input.unsupported_memmgr_type_or_op:{}/{}",
                input.idx, input.mem_type, input.op
            ));
        }
    }
    Ok(())
}

pub fn normalize_scratch_kind(scratch_type: &str) -> String {
    match scratch_type.trim() {
        "note" | "notes" | "scratch" => "notes".to_string(),
        "context" | "context_offload" | "offload" => "context_offload".to_string(),
        other => other.to_string(),
    }
}

fn issue(idx: usize, name: &str) -> String {
    format!("next_actions[{idx}].input.{name}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input<'a>(mem_type: &'a str, op: &'a str) -> MemmgrActionInput<'a> {
        MemmgrActionInput {
            idx: 0,
            mem_type,
            op,
            query: "",
            content: "",
            scratch_type: "",
            label: "",
            sql: "",
            params: &[],
            id: "",
            delta_ids: &[],
            slice_ids: &[],
        }
    }

    #[test]
    fn durable_query_requires_query() {
        assert_eq!(
            validate_action(input("durable", "query")).unwrap_err(),
            "next_actions[0].input.query_required"
        );
    }

    #[test]
    fn sql_requires_matching_placeholder_params() {
        let params = vec!["a".to_string()];
        let action = MemmgrActionInput {
            sql: "select * from memories where id=? and content like ?",
            params: &params,
            ..input("durable", "sql")
        };

        assert!(validate_action(action)
            .unwrap_err()
            .contains("params_count_mismatch expected=2 actual=1"));
    }

    #[test]
    fn scratch_context_offload_requires_prompt_refs() {
        let action = MemmgrActionInput {
            scratch_type: "context_offload",
            label: "large context",
            ..input("scratch", "write")
        };

        assert_eq!(
            validate_action(action).unwrap_err(),
            "next_actions[0].input.prompt_refs_required"
        );
    }

    #[test]
    fn scratch_kind_aliases_are_normalized() {
        assert_eq!(normalize_scratch_kind("note"), "notes");
        assert_eq!(normalize_scratch_kind("context"), "context_offload");
        assert_eq!(normalize_scratch_kind("custom"), "custom");
    }
}
