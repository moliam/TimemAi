use crate::response_protocol::{ParsedAction, ParsedContextCompact};
use serde_json::Value;

pub(crate) fn from_action(action: &ParsedAction) -> Result<ParsedContextCompact, String> {
    let input = action
        .raw_input
        .as_object()
        .ok_or_else(|| "context_compact.input_must_be_object".to_string())?;
    let discard_delta_ids = string_ids(input.get("discard"), "discard")?;
    let offload_delta_ids = string_ids(input.get("offload"), "offload")?;
    if discard_delta_ids.is_empty() && offload_delta_ids.is_empty() {
        return Err("context_compact.discard_or_offload_required".to_string());
    }
    let summary = input
        .get("summary")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "context_compact.summary_required".to_string())?
        .to_string();
    let mut delta_ids = discard_delta_ids.clone();
    for id in &offload_delta_ids {
        if !delta_ids.contains(id) {
            delta_ids.push(id.clone());
        }
    }
    Ok(ParsedContextCompact {
        discard_delta_ids,
        offload_delta_ids,
        delta_ids,
        slice_ids: Vec::new(),
        summary,
    })
}

fn string_ids(value: Option<&Value>, field: &str) -> Result<Vec<String>, String> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let items = value
        .as_array()
        .ok_or_else(|| format!("context_compact.{field}_must_be_array"))?;
    let mut ids = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let id = item
            .as_str()
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| format!("context_compact.{field}[{index}]_must_be_string"))?;
        if !ids.iter().any(|known| known == id) {
            ids.push(id.to_string());
        }
    }
    Ok(ids)
}

pub(crate) fn is_delta_list_field(name: &str) -> bool {
    matches!(name, "discard" | "offload")
}

pub(crate) fn inline_xml_delta_list(text: &str) -> Value {
    Value::Array(
        text.split(',')
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(|id| Value::String(id.to_string()))
            .collect(),
    )
}
