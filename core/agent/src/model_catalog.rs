//! Built-in provider/model catalog: UI-neutral projections and admission checks.
use serde::Serialize;
use serde_json::Value;
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningAdapter {
    EnumBodyField,
    ZhipuChatReasoning,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FunctionCallingSupport {
    Supported,
    Conditional,
    Unsupported,
    Unknown,
}

/// Provider-level capability declaration: the single admission table for
/// provider gates. The web projection serializes it so the endpoint editor
/// stays provider-agnostic (data-driven, no provider literals in UI logic).
#[derive(Debug, Clone, Serialize)]
pub struct ProviderSpec {
    pub id: &'static str,
    pub allowed_protocols: &'static [&'static str],
    pub requires_catalog: bool,
}

/// Admission stays closed: a provider appears here only after its adapters
/// and per-model catalog profiles are validated.
pub const PROVIDERS: &[ProviderSpec] = &[
    ProviderSpec {
        id: "openai",
        allowed_protocols: &["openai-responses", "openai-compatible"],
        requires_catalog: false,
    },
    ProviderSpec {
        id: "zhipu",
        allowed_protocols: &["openai-responses", "openai-compatible"],
        requires_catalog: true,
    },
];

pub fn provider_spec(id: &str) -> Option<&'static ProviderSpec> {
    PROVIDERS.iter().find(|spec| spec.id == id)
}

#[derive(Debug, Clone, Serialize)]
pub struct CatalogProtocol {
    pub protocol: String,
    /// Resolved protocol override, falling back to the model connection URL.
    pub base_url: String,
    pub reasoning_adapter: ReasoningAdapter,
    pub function_calling: FunctionCallingSupport,
    pub disabled_reason: Option<String>,
    pub fixed_effort: Option<String>,
    pub fixed_reason: Option<String>,
}
#[derive(Debug, Clone, Serialize)]
pub struct CatalogModel {
    pub id: String,
    pub revision: u64,
    pub provider: String,
    pub model: String,
    pub label: String,
    pub base_url: String,
    pub efforts: Vec<String>,
    pub default_effort: String,
    pub middle_default: bool,
    pub protocols: Vec<CatalogProtocol>,
    pub min_input: u32,
    pub max_input: u32,
    pub min_output: u32,
    pub max_output: u32,
    pub context_window: u32,
}

static CATALOG: OnceLock<Result<Vec<CatalogModel>, String>> = OnceLock::new();

fn loaded() -> &'static Result<Vec<CatalogModel>, String> {
    CATALOG.get_or_init(|| {
        let directory = std::env::var_os("TIMEM_MODEL_CATALOG_DIR").map(std::path::PathBuf::from);
        load_catalog(directory.as_deref())
    })
}

pub fn ensure_loaded() -> Result<(), String> {
    loaded().as_ref().map(|_| ()).map_err(Clone::clone)
}

pub fn models() -> &'static [CatalogModel] {
    loaded()
        .as_ref()
        .expect("catalog must be admitted before model service startup")
}

/// Startup snapshot. External JSON uses the same versioned descriptor contract
/// as bundled models. No executable paths, network discovery or hot mutation.
pub fn load_catalog(directory: Option<&std::path::Path>) -> Result<Vec<CatalogModel>, String> {
    let mut models = Vec::new();
    for resource in [
        include_str!("../../../resources/openai_model_catalog.json"),
        include_str!("../../../resources/zhipu_model_catalog.json"),
    ] {
        models.extend(parse(resource)?);
    }
    if let Some(directory) = directory {
        let mut paths = Vec::new();
        for entry in std::fs::read_dir(directory).map_err(|e| format!("catalog_directory:{e}"))? {
            let entry = entry.map_err(|e| format!("catalog_entry:{e}"))?;
            if entry.path().extension().is_some_and(|s| s == "json") {
                if !entry.file_type().map_err(|e| e.to_string())?.is_file() {
                    return Err("catalog_regular_files_only".into());
                }
                paths.push(entry.path());
                if paths.len() > 128 {
                    return Err("catalog_too_many_files".into());
                }
            }
        }
        paths.sort();
        for path in paths {
            use std::io::Read;
            let file = std::fs::File::open(&path).map_err(|e| format!("catalog_open:{e}"))?;
            let mut bytes = Vec::new();
            file.take(1_048_577)
                .read_to_end(&mut bytes)
                .map_err(|e| format!("catalog_read:{e}"))?;
            if bytes.len() > 1_048_576 {
                return Err("catalog_file_too_large".into());
            }
            let text = std::str::from_utf8(&bytes).map_err(|e| format!("catalog_utf8:{e}"))?;
            models.extend(parse(text).map_err(|e| {
                format!(
                    "catalog_file:{}:{e}",
                    path.file_name().unwrap().to_string_lossy()
                )
            })?);
            if models.len() > 512 {
                return Err("catalog_too_many_models".into());
            }
        }
    }
    let mut ids = std::collections::HashSet::new();
    let mut identities = std::collections::HashSet::new();
    for model in &models {
        if !ids.insert(&model.id) {
            return Err("catalog_duplicate_id".into());
        }
        if !identities.insert((&model.provider, &model.model)) {
            return Err("catalog_duplicate_identity".into());
        }
    }
    Ok(models)
}

pub fn parse(text: &str) -> Result<Vec<CatalogModel>, String> {
    if text.len() > 1_048_576 {
        return Err("catalog_file_too_large".into());
    }
    let root: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    validate_descriptor_shape(&root)?;
    if root["schema_version"] != 2 {
        return Err("unsupported_catalog_version".into());
    }
    let entries = root["models"].as_array().ok_or("catalog_models_missing")?;
    if entries.is_empty() || entries.len() > 512 {
        return Err("catalog_invalid_model_count".into());
    }
    let mut result = Vec::new();
    for entry in entries {
        if !matches!(entry["provider_id"].as_str(), Some("openai" | "zhipu")) {
            return Err("catalog_provider_adapter_not_implemented".into());
        }
        let string = |v: &Value| {
            v.as_str()
                .map(str::to_owned)
                .ok_or_else(|| "catalog_string_missing".to_string())
        };
        let dims = entry["dimensions"]
            .as_array()
            .ok_or("catalog_dimensions_missing")?;
        let effort = dims
            .iter()
            .find(|d| d["id"] == "reasoning.effort")
            .ok_or("catalog_effort_missing")?;
        let efforts = effort["options"]
            .as_array()
            .ok_or("catalog_options_missing")?
            .iter()
            .map(|o| string(&o["id"]))
            .collect::<Result<Vec<_>, _>>()?;
        if efforts.is_empty()
            || efforts.len() > 16
            || efforts.iter().enumerate().any(|(i, e)| {
                e.is_empty()
                    || e.len() > 32
                    || !e
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
                    || efforts[..i].contains(e)
            })
        {
            return Err("catalog_invalid_efforts".into());
        }
        let policy = &effort["default_policy"];
        let middle_default = policy["official_value"].is_null();
        let default_effort = if middle_default {
            let choices = policy["fallback"]["candidates"]
                .as_array()
                .filter(|a| !a.is_empty())
                .ok_or("catalog_candidates_missing")?;
            string(&choices[(choices.len() - 1) / 2])?
        } else {
            string(&policy["official_value"])?
        };
        if !efforts.contains(&default_effort) {
            return Err("catalog_invalid_default".into());
        }
        let mut protocols = Vec::new();
        for p in entry["profiles"]
            .as_array()
            .ok_or("catalog_profiles_missing")?
        {
            let protocol = string(&p["protocol"])?;
            let (route, path) = match protocol.as_str() {
                "openai-responses" => ("/responses", "/reasoning/effort"),
                "openai-compatible" => ("/chat/completions", "/reasoning_effort"),
                _ => return Err("catalog_unknown_protocol".into()),
            };
            let bindings = p["bindings"].as_array().ok_or("catalog_bindings_missing")?;
            if p["route"] != route
                || bindings.len() != 1
                || bindings[0]["dimension"] != "reasoning.effort"
                || bindings[0]["path"] != path
            {
                return Err("catalog_unsupported_binding".into());
            }

            let reasoning_adapter = match bindings[0]["handler"].as_str() {
                // Zhipu Responses uses the standard enum body field binding; its
                // chat protocol keeps the dedicated zhipu_chat_reasoning adapter.
                Some("enum_body_field")
                    if entry["provider_id"] == "openai"
                        || (entry["provider_id"] == "zhipu" && protocol == "openai-responses") =>
                {
                    ReasoningAdapter::EnumBodyField
                }
                Some("zhipu_chat_reasoning")
                    if entry["provider_id"] == "zhipu" && protocol == "openai-compatible" =>
                {
                    ReasoningAdapter::ZhipuChatReasoning
                }
                _ => return Err("catalog_unsupported_binding".into()),
            };
            let function_calling = match p["function_calling"].as_str() {
                Some("supported") => FunctionCallingSupport::Supported,
                Some("conditional") => FunctionCallingSupport::Conditional,
                Some("unsupported") => FunctionCallingSupport::Unsupported,
                Some("unknown") => FunctionCallingSupport::Unknown,
                _ => return Err("catalog_function_calling_missing".into()),
            };
            let mut projection = CatalogProtocol {
                protocol: protocol.clone(),
                base_url: string(
                    p.get("default_base_url")
                        .unwrap_or(&entry["connection"]["default_base_url"]),
                )?,
                reasoning_adapter,
                function_calling,
                disabled_reason: None,
                fixed_effort: None,
                fixed_reason: None,
            };
            for rule in entry["constraints"]
                .as_array()
                .ok_or("catalog_constraints_missing")?
            {
                let mut applies = true;
                for condition in rule["when"]["all"]
                    .as_array()
                    .ok_or("catalog_conditions_missing")?
                {
                    if condition["dimension"] == "api.protocol" {
                        applies &= condition["equals"] == protocol;
                    } else if condition["context"] == "tool_mode" {
                        applies &= condition["equals"] == "native_function_calling";
                    } else {
                        return Err("catalog_unknown_condition".into());
                    }
                }
                if !applies {
                    continue;
                }
                for effect in rule["effects"]
                    .as_array()
                    .ok_or("catalog_effects_missing")?
                {
                    let reason = string(&rule["reason"])?;
                    match effect["type"].as_str() {
                        Some("fixed_selection")
                            if effect["dimension"] == "reasoning.effort"
                                && effect["send_policy"] == "explicit" =>
                        {
                            let value = string(&effect["value"])?;
                            if !efforts.contains(&value)
                                || projection
                                    .fixed_effort
                                    .as_ref()
                                    .is_some_and(|v| v != &value)
                            {
                                return Err("catalog_fixed_conflict".into());
                            }
                            projection.fixed_effort = Some(value);
                            projection.fixed_reason = Some(reason);
                        }
                        Some("disable_option") if effect["dimension"] == "api.protocol" => {
                            if effect["value"] == protocol {
                                projection.disabled_reason = Some(reason);
                            }
                        }
                        _ => return Err("catalog_unknown_effect".into()),
                    }
                }
            }
            if function_calling == FunctionCallingSupport::Unsupported
                && projection.disabled_reason.is_none()
            {
                return Err("catalog_native_tools_unsupported_without_constraint".into());
            }
            protocols.push(projection);
        }
        let limit = |key: &str| {
            entry["limits"][key]
                .as_u64()
                .and_then(|n| u32::try_from(n).ok())
        };
        let context_window = limit("context_window_tokens").ok_or("catalog_context_missing")?;
        let max_input = limit("max_input_tokens").unwrap_or(context_window);
        let max_output = limit("max_output_tokens").ok_or("catalog_output_missing")?;
        if protocols.is_empty()
            || protocols.iter().all(|p| p.disabled_reason.is_some())
            || protocols
                .iter()
                .enumerate()
                .any(|(i, p)| protocols[..i].iter().any(|q| q.protocol == p.protocol))
            || context_window < 3512
            || max_input < 3000
            || max_input > context_window
            || max_output < 512
            || max_output > context_window
        {
            return Err("catalog_invalid_limits_or_profiles".into());
        }
        result.push(CatalogModel {
            id: string(&entry["id"])?,
            provider: string(&entry["provider_id"])?,
            revision: entry["revision"]
                .as_u64()
                .ok_or("catalog_revision_missing")?,
            model: string(&entry["model_id"])?,
            label: string(&entry["label"])?,
            base_url: string(&entry["connection"]["default_base_url"])?,
            efforts,
            default_effort,
            middle_default,
            protocols,
            min_input: 3000,
            max_input: limit("max_input_tokens").unwrap_or(context_window),
            min_output: 512,
            max_output: limit("max_output_tokens").ok_or("catalog_output_missing")?,
            context_window,
        });
    }
    Ok(result)
}

pub fn validate(
    id: &str,
    model: &str,
    protocol: &str,
    effort: Option<&str>,
    input: u32,
    output: u32,
) -> Result<(), String> {
    let m = models()
        .iter()
        .find(|m| m.id == id)
        .ok_or("catalog_model_not_found")?;
    if m.model != model {
        return Err("catalog_model_mismatch".into());
    }
    let p = m
        .protocols
        .iter()
        .find(|p| p.protocol == protocol)
        .ok_or("catalog_protocol_not_supported")?;
    if let Some(reason) = &p.disabled_reason {
        return Err(reason.clone());
    }
    if let Some(fixed) = &p.fixed_effort {
        if effort != Some(fixed.as_str()) {
            return Err(p.fixed_reason.clone().unwrap_or_default());
        }
    }
    if let Some(effort) = effort {
        if !m.efforts.iter().any(|v| v == effort) {
            return Err("catalog_reasoning_not_supported".into());
        }
    }
    if input < m.min_input
        || input > m.max_input
        || output < m.min_output
        || output > m.max_output
        || u64::from(input) + u64::from(output) > u64::from(m.context_window)
    {
        return Err("catalog_token_budget_out_of_range".into());
    }
    Ok(())
}

/// Revalidate every prepared request before network I/O. Catalog endpoints use
/// native-safe protocol projections; custom/legacy endpoints are unchanged.
pub fn validate_request(config: &crate::ModelServiceConfig, body: &Value) -> Result<(), String> {
    crate::model_requirements::validate_config(config)?;
    let model = crate::model_requirements::capabilities(config);
    let id = if config.openai_compatible.requirements.version > 0 {
        match model {
            Some(m) => m.id.as_str(),
            None => return Ok(()),
        }
    } else {
        match config.openai_compatible.catalog_id.as_deref() {
            Some(id) => id,
            None => return Ok(()),
        }
    };
    let path = match config.api_protocol {
        crate::ApiProtocol::OpenAiResponses => "/reasoning/effort",
        crate::ApiProtocol::OpenAiCompatible => "/reasoning_effort",
        _ => return Err("catalog_protocol_not_supported".into()),
    };
    let output_key = match config.api_protocol {
        crate::ApiProtocol::OpenAiResponses => "max_output_tokens",
        _ => "max_tokens",
    };
    if body.get("model").and_then(Value::as_str) != Some(config.model.as_str())
        || body.get(output_key).and_then(Value::as_u64)
            != Some(u64::from(config.max_llm_output_tokens))
    {
        return Err("catalog_request_configuration_mismatch".into());
    }
    if uses_zhipu_chat(config) {
        let effort = body.get("reasoning_effort").and_then(Value::as_str);
        let expected_type = if effort == Some("none") {
            "disabled"
        } else {
            "enabled"
        };
        if effort.is_none()
            || body.pointer("/thinking/type").and_then(Value::as_str) != Some(expected_type)
            || body.pointer("/thinking/clear_thinking") != Some(&Value::Bool(true))
            || body.get("enable_thinking").is_some()
            || body.get("stream_options").is_some()
        {
            return Err("catalog_zhipu_reasoning_mismatch".into());
        }
    }
    validate(
        id,
        &config.model,
        config.api_protocol.label(),
        body.pointer(path).and_then(Value::as_str),
        config.max_llm_input_tokens,
        config.max_llm_output_tokens,
    )
}

/// Unknown preset identifiers from persisted/imported configuration are custom.
pub fn known_id(id: &str) -> Option<String> {
    let id = id.trim();
    models().iter().find(|m| m.id == id).map(|m| m.id.clone())
}

/// Adapter identity comes from a known descriptor, never from a proxy URL or model-name heuristic.
pub fn uses_zhipu_chat(config: &crate::ModelServiceConfig) -> bool {
    crate::model_requirements::provider(config) == Some("zhipu")
        && config.api_protocol == crate::ApiProtocol::OpenAiCompatible
}

// Runtime admission is deliberately closed, not a general JSON transformation
// language. Metadata is admitted by schema; executable semantics are checked
// below and by parse(). Keep this schema interpreter limited to our bundled
// descriptor schema (no remote references or user-provided schemas).
fn validate_descriptor_shape(value: &Value) -> Result<(), String> {
    static SCHEMA: OnceLock<Value> = OnceLock::new();
    let schema = SCHEMA.get_or_init(|| {
        serde_json::from_str(include_str!(
            "../../../resources/provider_model_catalog.schema.json"
        ))
        .expect("bundled descriptor schema")
    });
    fn check(value: &Value, rule: &Value, root: &Value, depth: usize) -> Result<(), String> {
        if depth > 32 {
            return Err("catalog_schema_depth".into());
        }
        if let Some(reference) = rule.get("$ref").and_then(Value::as_str) {
            let target = reference
                .strip_prefix('#')
                .and_then(|p| root.pointer(p))
                .ok_or("catalog_schema_reference")?;
            return check(value, target, root, depth + 1);
        }
        if let Some(choices) = rule.get("oneOf").and_then(Value::as_array) {
            if choices
                .iter()
                .filter(|r| check(value, r, root, depth + 1).is_ok())
                .count()
                != 1
            {
                return Err("catalog_schema_one_of".into());
            }
        }
        if rule.get("const").is_some_and(|c| c != value) {
            return Err("catalog_schema_const".into());
        }
        if rule
            .get("enum")
            .and_then(Value::as_array)
            .is_some_and(|a| !a.contains(value))
        {
            return Err("catalog_schema_enum".into());
        }
        let matches_type = |name: &str| match name {
            "object" => value.is_object(),
            "array" => value.is_array(),
            "string" => value.is_string(),
            "integer" => value.is_u64() || value.is_i64(),
            "null" => value.is_null(),
            "boolean" => value.is_boolean(),
            _ => false,
        };
        if let Some(types) = rule.get("type") {
            let valid = types.as_str().map(matches_type).unwrap_or_else(|| {
                types
                    .as_array()
                    .is_some_and(|a| a.iter().any(|t| t.as_str().is_some_and(matches_type)))
            });
            if !valid {
                return Err("catalog_schema_type".into());
            }
        }
        if let Some(object) = value.as_object() {
            if let Some(required) = rule.get("required").and_then(Value::as_array) {
                if required
                    .iter()
                    .any(|k| !object.contains_key(k.as_str().unwrap_or("")))
                {
                    return Err("catalog_schema_required".into());
                }
            }
            for (key, field) in object {
                if let Some(child) = rule.get("properties").and_then(|p| p.get(key)) {
                    check(field, child, root, depth + 1)?;
                } else if rule.get("additionalProperties") == Some(&Value::Bool(false)) {
                    return Err(format!("catalog_unknown_field:{key}"));
                }
            }
        }
        if let Some(items) = value.as_array() {
            if items.len() > 512
                || rule
                    .get("minItems")
                    .and_then(Value::as_u64)
                    .is_some_and(|min| items.len() < min as usize)
            {
                return Err("catalog_schema_array_size".into());
            }
            if rule.get("uniqueItems") == Some(&Value::Bool(true))
                && items
                    .iter()
                    .enumerate()
                    .any(|(i, v)| items[..i].contains(v))
            {
                return Err("catalog_schema_duplicate".into());
            }
            if let Some(child) = rule.get("items") {
                for item in items {
                    check(item, child, root, depth + 1)?;
                }
            }
        }
        if let Some(text) = value.as_str() {
            if text.len() > 8192
                || rule
                    .get("minLength")
                    .and_then(Value::as_u64)
                    .is_some_and(|min| text.trim().chars().count() < min as usize)
            {
                return Err("catalog_schema_string_size".into());
            }
            if rule.get("format").and_then(Value::as_str) == Some("uri") {
                let url = reqwest::Url::parse(text).map_err(|_| "catalog_invalid_url")?;
                if !matches!(url.scheme(), "http" | "https")
                    || url.host_str().is_none()
                    || !url.username().is_empty()
                    || url.password().is_some()
                    || url.query().is_some()
                    || url.fragment().is_some()
                {
                    return Err("catalog_invalid_url".into());
                }
            }
        }
        if let (Some(n), Some(min)) = (value.as_f64(), rule.get("minimum").and_then(Value::as_f64))
        {
            if n < min {
                return Err("catalog_schema_minimum".into());
            }
        }
        Ok(())
    }
    check(value, schema, schema, 0)?;
    for model in value["models"].as_array().ok_or("catalog_models_missing")? {
        let dimensions = model["dimensions"]
            .as_array()
            .ok_or("catalog_dimensions_missing")?;
        if dimensions.len() != 2 {
            return Err("catalog_dimensions_unsupported".into());
        }
        let api = dimensions
            .iter()
            .find(|d| d["id"] == "api.protocol")
            .ok_or("catalog_api_dimension_missing")?;
        let effort = dimensions
            .iter()
            .find(|d| d["id"] == "reasoning.effort")
            .ok_or("catalog_effort_missing")?;
        if api["handler"] != "api_protocol"
            || api["ordered"] != false
            || effort["handler"] != "reasoning_effort"
            || effort["ordered"] != true
        {
            return Err("catalog_dimension_handler".into());
        }
        let options: Vec<_> = effort["options"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| &v["id"])
            .collect();
        let candidates = effort["default_policy"]["fallback"]["candidates"]
            .as_array()
            .ok_or("catalog_candidates_missing")?;
        let mut previous = None;
        for candidate in candidates {
            let index = options
                .iter()
                .position(|v| *v == candidate)
                .ok_or("catalog_candidate_unknown")?;
            if candidate == "none" || previous.is_some_and(|p| index <= p) {
                return Err("catalog_candidates_not_ordered".into());
            }
            previous = Some(index);
        }
        let profiles = model["profiles"].as_array().unwrap();
        let api_options: Vec<_> = api["options"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| &v["id"])
            .collect();
        if profiles.len() != api_options.len()
            || profiles
                .iter()
                .any(|p| !api_options.contains(&&p["protocol"]))
            || !api_options.contains(&&api["default_policy"]["value"])
        {
            return Err("catalog_protocol_options_mismatch".into());
        }
        for profile in profiles {
            if profile["dimension_ids"] != serde_json::json!(["reasoning.effort"]) {
                return Err("catalog_dimension_reference".into());
            }
        }
        let rules = model["constraints"].as_array().unwrap();
        let mut rule_ids = std::collections::HashSet::new();
        for rule in rules {
            if !rule_ids.insert(rule["id"].as_str().unwrap()) {
                return Err("catalog_duplicate_rule".into());
            }
            for effect in rule["effects"].as_array().unwrap() {
                if effect["type"] == "fixed_selection" && !options.contains(&&effect["value"]) {
                    return Err("catalog_fixed_unknown".into());
                }
                if effect["type"] == "disable_option" && !api_options.contains(&&effect["value"]) {
                    return Err("catalog_disabled_unknown".into());
                }
            }
        }
    }
    Ok(())
}
