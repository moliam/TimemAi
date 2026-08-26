use serde_json::{Map, Value};

/// Conservatively simplifies provider-facing JSON Schemas without changing
/// constraints that cannot be proven redundant from the local structure.
pub(crate) fn optimize_provider_schema(value: Value) -> Value {
    match value {
        Value::Object(object) => {
            let mut object = object
                .into_iter()
                .map(|(key, value)| (key, optimize_provider_schema(value)))
                .collect::<Map<_, _>>();
            let one_of = object.get("oneOf").and_then(Value::as_array).cloned();
            if let Some(all_of) = object
                .remove("allOf")
                .and_then(|value| value.as_array().cloned())
            {
                let mut optimized = Vec::new();
                for clause in all_of {
                    if one_of.as_ref().is_some_and(|branches| {
                        required_any_clause_fields(&clause)
                            .is_some_and(|fields| one_of_requires_any(branches, &fields))
                    }) || optimized.contains(&clause)
                    {
                        continue;
                    }
                    optimized.push(clause);
                }
                if !optimized.is_empty() {
                    object.insert("allOf".to_string(), Value::Array(optimized));
                }
            }
            Value::Object(object)
        }
        Value::Array(items) => {
            Value::Array(items.into_iter().map(optimize_provider_schema).collect())
        }
        value => value,
    }
}

fn required_any_clause_fields(clause: &Value) -> Option<Vec<String>> {
    let alternatives = clause.as_object()?.get("anyOf")?.as_array()?;
    if alternatives.is_empty() {
        return None;
    }
    alternatives
        .iter()
        .map(|alternative| {
            let required = alternative.as_object()?.get("required")?.as_array()?;
            if required.len() != 1 {
                return None;
            }
            required[0].as_str().map(str::to_string)
        })
        .collect()
}

fn one_of_requires_any(branches: &[Value], fields: &[String]) -> bool {
    !branches.is_empty()
        && branches.iter().all(|branch| {
            branch
                .get("required")
                .and_then(Value::as_array)
                .is_some_and(|required| {
                    required.iter().any(|field| {
                        field
                            .as_str()
                            .is_some_and(|field| fields.iter().any(|candidate| candidate == field))
                    })
                })
        })
}

#[cfg(test)]
#[path = "../tests/unit/schema_optimizer_tests.rs"]
mod tests;
