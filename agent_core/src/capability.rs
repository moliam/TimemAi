use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::prompt_spec::replace_json_string_field_with_value;

const MEMMGR_MANIFEST: &str = include_str!("../../resources/capabilities/tools/memmgr.yaml");
const CAPMGR_MANIFEST: &str = include_str!("../../resources/capabilities/tools/capmgr.yaml");
const RUN_BASH_MANIFEST: &str = include_str!("../../resources/capabilities/tools/run_bash.yaml");
const SHELL_JOB_STATUS_MANIFEST: &str =
    include_str!("../../resources/capabilities/tools/shell_job_status.yaml");
const BUILTIN_BINDINGS: &[&str] = &["memmgr", "capmgr", "run_bash", "shell_job_status"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityBinding {
    pub binding_type: String,
    pub name: String,
    pub command_path: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityPrompt {
    pub when: String,
    pub input: String,
    pub result: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityInputSchema {
    pub schema_type: String,
    pub required: Vec<String>,
    pub required_any: Vec<Vec<String>>,
    pub required_when: Vec<CapabilityRequiredWhen>,
    pub enum_fields: BTreeMap<String, Vec<String>>,
    pub properties: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityOutputSchema {
    pub schema_type: String,
    pub properties: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityRequiredWhen {
    pub field: String,
    pub values: Vec<String>,
    pub required: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolManifest {
    pub kind: String,
    pub id: String,
    pub binding: CapabilityBinding,
    pub description: String,
    pub prompt: CapabilityPrompt,
    pub input_schema: CapabilityInputSchema,
    pub output_schema: CapabilityOutputSchema,
    pub example: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillManifest {
    pub kind: String,
    pub id: String,
    pub title: String,
    pub summary: String,
    pub when_to_use: String,
    pub entry: String,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityRegistry {
    tools: BTreeMap<String, ToolManifest>,
    skills: BTreeMap<String, SkillManifest>,
}

impl CapabilityRegistry {
    pub fn from_manifests(
        tool_manifests: &[&str],
        skill_manifests: &[(&str, &str)],
    ) -> Result<Self, String> {
        let mut tools = BTreeMap::new();
        for raw in tool_manifests {
            let manifest = parse_tool_manifest(raw)?;
            validate_manifest(&manifest)?;
            if tools.insert(manifest.id.clone(), manifest).is_some() {
                return Err("duplicate_tool_manifest_id".to_string());
            }
        }
        let mut skills = BTreeMap::new();
        for (raw, body) in skill_manifests {
            let manifest = parse_skill_manifest(raw, body)?;
            validate_skill_manifest(&manifest)?;
            if skills.insert(manifest.id.clone(), manifest).is_some() {
                return Err("duplicate_skill_manifest_id".to_string());
            }
        }
        Ok(Self { tools, skills })
    }

    pub fn builtin() -> Self {
        Self::from_manifests(
            &[
                MEMMGR_MANIFEST,
                CAPMGR_MANIFEST,
                RUN_BASH_MANIFEST,
                SHELL_JOB_STATUS_MANIFEST,
            ],
            &[],
        )
        .expect("builtin capability manifests must be valid")
    }

    pub fn builtin_with_overlay_dir(dir: impl AsRef<Path>) -> Result<Self, String> {
        let mut registry = Self::builtin();
        registry.load_overlay_dir(dir.as_ref())?;
        Ok(registry)
    }

    pub fn load_overlay_dir(&mut self, dir: &Path) -> Result<(), String> {
        if !dir.exists() {
            return Err(format!("capability_dir_not_found:{}", dir.display()));
        }
        if !dir.is_dir() {
            return Err(format!("capability_dir_not_directory:{}", dir.display()));
        }
        for path in sorted_files_with_extension(&dir.join("tools"), "yaml")? {
            let raw = fs::read_to_string(&path)
                .map_err(|err| format!("read_tool_manifest_failed:{}:{err}", path.display()))?;
            let mut manifest = parse_tool_manifest(&raw)
                .map_err(|err| format!("parse_tool_manifest_failed:{}:{err}", path.display()))?;
            validate_overlay_manifest(&mut manifest, dir)?;
            self.tools.insert(manifest.id.clone(), manifest);
        }
        for skill_dir in sorted_dirs(&dir.join("skills"))? {
            let manifest_path = skill_dir.join("skill.yaml");
            if !manifest_path.exists() {
                continue;
            }
            let raw = fs::read_to_string(&manifest_path).map_err(|err| {
                format!(
                    "read_skill_manifest_failed:{}:{err}",
                    manifest_path.display()
                )
            })?;
            let entry = skill_entry_from_manifest(&raw).map_err(|err| {
                format!(
                    "parse_skill_manifest_failed:{}:{err}",
                    manifest_path.display()
                )
            })?;
            let body_path = normalize_child_path(&skill_dir, &entry)?;
            let body = fs::read_to_string(&body_path)
                .map_err(|err| format!("read_skill_body_failed:{}:{err}", body_path.display()))?;
            let manifest = parse_skill_manifest(&raw, &body).map_err(|err| {
                format!(
                    "parse_skill_manifest_failed:{}:{err}",
                    manifest_path.display()
                )
            })?;
            validate_skill_manifest(&manifest)?;
            self.skills.insert(manifest.id.clone(), manifest);
        }
        Ok(())
    }

    pub fn contains_tool(&self, action: &str) -> bool {
        self.tools.contains_key(action)
    }

    pub fn binding_name(&self, action: &str) -> Option<&str> {
        self.tools
            .get(action)
            .map(|manifest| manifest.binding.name.as_str())
    }

    pub fn binding(&self, action: &str) -> Option<&CapabilityBinding> {
        self.tools.get(action).map(|manifest| &manifest.binding)
    }

    pub fn validate_action_input(&self, action: &str, input: &Value) -> Result<(), String> {
        let Some(manifest) = self.tools.get(action) else {
            return Err(format!("unsupported_action:{action}"));
        };
        for key in &manifest.input_schema.required {
            if is_missing_input_value(input.get(key)) {
                return Err(format!("input.{key}_required"));
            }
        }
        for group in &manifest.input_schema.required_any {
            if group
                .iter()
                .all(|key| is_missing_input_value(input.get(key)))
            {
                return Err(format!("input.any_required:{}", group.join("|")));
            }
        }
        for rule in &manifest.input_schema.required_when {
            let Some(value) = input.get(&rule.field).and_then(Value::as_str) else {
                continue;
            };
            let normalized = value.trim().to_lowercase();
            if !rule.values.iter().any(|allowed| allowed == &normalized) {
                continue;
            }
            for key in &rule.required {
                if is_missing_input_value(input.get(key)) {
                    return Err(format!(
                        "input.{key}_required_when_{}={normalized}",
                        rule.field
                    ));
                }
            }
        }
        for (key, allowed_values) in &manifest.input_schema.enum_fields {
            let Some(value) = input.get(key) else {
                continue;
            };
            if is_missing_input_value(Some(value)) {
                continue;
            }
            let Some(text) = value
                .as_str()
                .map(str::trim)
                .filter(|text| !text.is_empty())
            else {
                return Err(format!("input.{key}_must_be_string"));
            };
            let normalized = text.to_lowercase();
            if !allowed_values.iter().any(|allowed| allowed == &normalized) {
                return Err(format!("input.{key}_unsupported:{text}"));
            }
        }
        Ok(())
    }

    pub fn skill_headers_value(&self) -> Value {
        let mut headers = Map::new();
        for (id, skill) in &self.skills {
            headers.insert(
                id.clone(),
                json_object([
                    ("title", Value::String(skill.title.clone())),
                    ("summary", Value::String(skill.summary.clone())),
                    ("when_to_use", Value::String(skill.when_to_use.clone())),
                ]),
            );
        }
        Value::Object(headers)
    }

    pub fn list_text(&self, kind: &str) -> String {
        match kind {
            "tool" => {
                let rows = self
                    .tools
                    .values()
                    .map(|tool| {
                        format!(
                            "- id={} binding={}:{} description={}",
                            tool.id, tool.binding.binding_type, tool.binding.name, tool.description
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                format!("Action result: capmgr\nop: list\nkind: tool\nresults:\n{rows}")
            }
            "skill" => {
                let rows = self
                    .skills
                    .values()
                    .map(|skill| {
                        format!(
                            "- id={} title={} summary={} when_to_use={}",
                            skill.id, skill.title, skill.summary, skill.when_to_use
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                format!("Action result: capmgr\nop: list\nkind: skill\nresults:\n{rows}")
            }
            "" | "all" => format!("{}\n\n{}", self.list_text("tool"), self.list_text("skill")),
            other => {
                format!("Action result: capmgr\nop: list\nkind: {other}\nerror: unsupported_kind")
            }
        }
    }

    pub fn load_text(&self, kind: &str, id: &str) -> String {
        match kind {
            "tool" => match self.tools.get(id) {
                Some(tool) => format!(
                    "Action result: capmgr\nop: load\nkind: tool\nid: {}\ndescription: {}\nbinding: {}:{}\ninput:\n{}\noutput:\n{}\nexample:\n{}",
                    tool.id,
                    tool.description,
                    tool.binding.binding_type,
                    tool.binding.name,
                    serde_json::to_string_pretty(&input_properties_value(tool)).unwrap_or_default(),
                    serde_json::to_string_pretty(&output_properties_value(tool)).unwrap_or_default(),
                    serde_json::to_string_pretty(&tool.example).unwrap_or_default()
                ),
                None => format!("Action result: capmgr\nop: load\nkind: tool\nid: {id}\nerror: not_found"),
            },
            "skill" => match self.skills.get(id) {
                Some(skill) => format!(
                    "Action result: capmgr\nop: load\nkind: skill\nid: {}\ntitle: {}\nsummary: {}\nbody:\n{}",
                    skill.id, skill.title, skill.summary, skill.body
                ),
                None => format!("Action result: capmgr\nop: load\nkind: skill\nid: {id}\nerror: not_found"),
            },
            other => format!(
                "Action result: capmgr\nop: load\nkind: {other}\nid: {id}\nerror: unsupported_kind"
            ),
        }
    }

    pub fn tool_catalog_value(&self) -> Value {
        let mut catalog = Map::new();
        for (id, manifest) in &self.tools {
            let mut item = Map::new();
            item.insert(
                "when".to_string(),
                Value::String(manifest.prompt.when.clone()),
            );
            item.insert(
                "input".to_string(),
                Value::String(manifest.prompt.input.clone()),
            );
            item.insert(
                "result".to_string(),
                Value::String(manifest.prompt.result.clone()),
            );
            if !manifest.input_schema.required.is_empty() {
                item.insert(
                    "required".to_string(),
                    Value::Array(
                        manifest
                            .input_schema
                            .required
                            .iter()
                            .cloned()
                            .map(Value::String)
                            .collect(),
                    ),
                );
            }
            if !manifest.input_schema.required_any.is_empty() {
                item.insert(
                    "required_any".to_string(),
                    Value::Array(
                        manifest
                            .input_schema
                            .required_any
                            .iter()
                            .map(|group| {
                                Value::Array(group.iter().cloned().map(Value::String).collect())
                            })
                            .collect(),
                    ),
                );
            }
            if !manifest.input_schema.required_when.is_empty() {
                item.insert(
                    "required_when".to_string(),
                    Value::Array(
                        manifest
                            .input_schema
                            .required_when
                            .iter()
                            .map(required_when_value)
                            .collect(),
                    ),
                );
            }
            item.insert("example".to_string(), manifest.example.clone());
            catalog.insert(id.clone(), Value::Object(item));
        }
        Value::Object(catalog)
    }

    pub fn render_tool_catalog_json(&self) -> String {
        serde_json::to_string_pretty(&self.tool_catalog_value())
            .expect("tool catalog must render as JSON")
    }

    pub fn enrich_static_prompt(&self, static_prompt: &str) -> String {
        if let Some(with_catalog) = replace_json_string_field_with_value(
            static_prompt,
            "tool_catalog",
            &self.tool_catalog_value(),
        ) {
            if let Some(with_skills) = replace_json_string_field_with_value(
                &with_catalog,
                "skill_headers",
                &self.skill_headers_value(),
            ) {
                return with_skills;
            }
        }

        let Ok(mut value) = serde_json::from_str::<Value>(static_prompt) else {
            return static_prompt.to_string();
        };
        if let Some(tool_capability) = value
            .get_mut("Tool_capability")
            .and_then(Value::as_object_mut)
        {
            tool_capability.insert("tool_catalog".to_string(), self.tool_catalog_value());
            tool_capability.insert("skill_headers".to_string(), self.skill_headers_value());
        }
        serde_json::to_string_pretty(&value).unwrap_or_else(|_| static_prompt.to_string())
    }
}

fn parse_tool_manifest(raw: &str) -> Result<ToolManifest, String> {
    let mut top = BTreeMap::<String, String>::new();
    let mut input_properties = BTreeMap::<String, Value>::new();
    let mut output_properties = BTreeMap::<String, Value>::new();
    let mut required = Vec::<String>::new();
    let mut required_any = Vec::<Vec<String>>::new();
    let mut required_when = Vec::<CapabilityRequiredWhen>::new();
    let mut enum_fields = BTreeMap::<String, Vec<String>>::new();
    let mut prompt_when = String::new();
    let mut prompt_input = String::new();
    let mut prompt_result = String::new();
    let mut input_schema_json = String::new();
    let mut output_schema_json = String::new();
    let mut example_json = String::new();
    let mut section: Option<&str> = None;

    for line in raw.lines() {
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        if line == "prompt_when: |" {
            section = Some("prompt_when");
            continue;
        }
        if line == "prompt_input: |" {
            section = Some("prompt_input");
            continue;
        }
        if line == "prompt_result: |" {
            section = Some("prompt_result");
            continue;
        }
        if line == "input_properties:" {
            section = Some("input_properties");
            continue;
        }
        if line == "output_properties:" {
            section = Some("output_properties");
            continue;
        }
        if line == "input_schema: |" {
            section = Some("input_schema");
            continue;
        }
        if line == "output_schema: |" {
            section = Some("output_schema");
            continue;
        }
        if line == "example_json: |" {
            section = Some("example_json");
            continue;
        }
        if line == "required:" {
            section = Some("required");
            continue;
        }
        if line == "required_any:" {
            section = Some("required_any");
            continue;
        }
        if line == "required_when:" {
            section = Some("required_when");
            continue;
        }
        if line == "enum_fields:" {
            section = Some("enum_fields");
            continue;
        }
        match section {
            Some("prompt_when") if line.starts_with("  ") => {
                if !prompt_when.is_empty() {
                    prompt_when.push('\n');
                }
                prompt_when.push_str(line.trim());
            }
            Some("prompt_input") if line.starts_with("  ") => {
                if !prompt_input.is_empty() {
                    prompt_input.push('\n');
                }
                prompt_input.push_str(line.trim());
            }
            Some("prompt_result") if line.starts_with("  ") => {
                if !prompt_result.is_empty() {
                    prompt_result.push('\n');
                }
                prompt_result.push_str(line.trim());
            }
            Some("example_json") if line.starts_with("  ") => {
                example_json.push_str(line.strip_prefix("  ").unwrap_or(line));
                example_json.push('\n');
            }
            Some("input_schema") if line.starts_with("  ") => {
                input_schema_json.push_str(line.strip_prefix("  ").unwrap_or(line));
                input_schema_json.push('\n');
            }
            Some("output_schema") if line.starts_with("  ") => {
                output_schema_json.push_str(line.strip_prefix("  ").unwrap_or(line));
                output_schema_json.push('\n');
            }
            Some("input_properties") if line.starts_with("  ") => {
                let trimmed = line.trim();
                let Some((key, value)) = trimmed.split_once(':') else {
                    return Err(format!("input_property_must_use_key_colon_value:{trimmed}"));
                };
                input_properties.insert(
                    key.trim().to_string(),
                    Value::String(value.trim().to_string()),
                );
            }
            Some("output_properties") if line.starts_with("  ") => {
                let trimmed = line.trim();
                let Some((key, value)) = trimmed.split_once(':') else {
                    return Err(format!(
                        "output_property_must_use_key_colon_value:{trimmed}"
                    ));
                };
                output_properties.insert(
                    key.trim().to_string(),
                    Value::String(value.trim().to_string()),
                );
            }
            Some("required") if line.starts_with("  - ") => {
                let key = line.trim_start().trim_start_matches("- ").trim();
                if key.is_empty() {
                    return Err("required_field_empty".to_string());
                }
                required.push(key.to_string());
            }
            Some("required_any") if line.starts_with("  - ") => {
                let group = line
                    .trim_start()
                    .trim_start_matches("- ")
                    .split('|')
                    .map(str::trim)
                    .filter(|key| !key.is_empty())
                    .map(str::to_string)
                    .collect::<Vec<_>>();
                if group.len() < 2 {
                    return Err("required_any_group_needs_at_least_two_fields".to_string());
                }
                required_any.push(group);
            }
            Some("required_when") if line.starts_with("  ") => {
                let trimmed = line.trim();
                let Some((condition, fields)) = trimmed.split_once(':') else {
                    return Err(format!(
                        "required_when_must_use_condition_colon_fields:{trimmed}"
                    ));
                };
                let Some((field, values)) = condition.split_once('=') else {
                    return Err(format!(
                        "required_when_condition_must_use_equals:{condition}"
                    ));
                };
                let values = values
                    .split('|')
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(|value| value.to_lowercase())
                    .collect::<Vec<_>>();
                let required = fields
                    .split(',')
                    .map(str::trim)
                    .filter(|key| !key.is_empty())
                    .map(str::to_string)
                    .collect::<Vec<_>>();
                if field.trim().is_empty() || values.is_empty() || required.is_empty() {
                    return Err(format!("required_when_invalid:{trimmed}"));
                }
                required_when.push(CapabilityRequiredWhen {
                    field: field.trim().to_string(),
                    values,
                    required,
                });
            }
            Some("enum_fields") if line.starts_with("  ") => {
                let trimmed = line.trim();
                let Some((key, values)) = trimmed.split_once(':') else {
                    return Err(format!("enum_field_must_use_key_colon_value:{trimmed}"));
                };
                let allowed = values
                    .split('|')
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(|value| value.to_lowercase())
                    .collect::<Vec<_>>();
                if key.trim().is_empty() || allowed.is_empty() {
                    return Err(format!("enum_field_invalid:{trimmed}"));
                }
                enum_fields.insert(key.trim().to_string(), allowed);
            }
            _ if !line.starts_with(' ') => {
                section = None;
                let Some((key, value)) = line.split_once(':') else {
                    return Err(format!("top_level_key_must_use_colon:{line}"));
                };
                top.insert(key.trim().to_string(), value.trim().to_string());
            }
            _ => {
                return Err(format!("unsupported_manifest_line:{line}"));
            }
        }
    }

    let id = required_top(&top, "id")?;
    let example = serde_json::from_str(example_json.trim())
        .map_err(|err| format!("{id}:example_json_invalid:{err}"))?;
    let input_schema = if input_schema_json.trim().is_empty() {
        CapabilityInputSchema {
            schema_type: "object".to_string(),
            required,
            required_any,
            required_when,
            enum_fields,
            properties: input_properties,
        }
    } else {
        parse_input_schema_json(&id, input_schema_json.trim())?
    };
    let output_schema = if output_schema_json.trim().is_empty() {
        CapabilityOutputSchema {
            schema_type: "object".to_string(),
            properties: output_properties,
        }
    } else {
        parse_output_schema_json(&id, output_schema_json.trim())?
    };
    let prompt_input = if prompt_input.trim().is_empty() {
        "Use the fields shown in the example; load this tool with capmgr if full details are needed."
            .to_string()
    } else {
        prompt_input
    };
    let prompt_result = if prompt_result.trim().is_empty() {
        "Runtime returns an action result with either useful output or a short error string."
            .to_string()
    } else {
        prompt_result
    };
    Ok(ToolManifest {
        kind: required_top(&top, "kind")?,
        id,
        binding: CapabilityBinding {
            binding_type: required_top(&top, "binding_type")?,
            name: required_top(&top, "binding_name")?,
            command_path: None,
        },
        description: required_top(&top, "description")?,
        prompt: CapabilityPrompt {
            when: prompt_when,
            input: prompt_input,
            result: prompt_result,
        },
        input_schema,
        output_schema,
        example,
    })
}

fn parse_input_schema_json(tool_id: &str, raw: &str) -> Result<CapabilityInputSchema, String> {
    let value: Value = serde_json::from_str(raw)
        .map_err(|err| format!("{tool_id}:input_schema_json_invalid:{err}"))?;
    let object = value
        .as_object()
        .ok_or_else(|| format!("{tool_id}:input_schema_must_be_object"))?;
    let schema_type = object
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("object")
        .to_string();
    let properties = object
        .get("properties")
        .and_then(Value::as_object)
        .ok_or_else(|| format!("{tool_id}:input_schema_properties_required"))?
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect::<BTreeMap<_, _>>();
    let required = parse_string_array_field(object.get("required"), tool_id, "required")?;
    let required_any =
        parse_string_array_groups_field(object.get("x-required-any"), tool_id, "x-required-any")?;
    let required_when =
        parse_required_when_field(object.get("x-required-when"), tool_id, "x-required-when")?;
    let enum_fields = enum_fields_from_properties(&properties);
    Ok(CapabilityInputSchema {
        schema_type,
        required,
        required_any,
        required_when,
        enum_fields,
        properties,
    })
}

fn parse_output_schema_json(tool_id: &str, raw: &str) -> Result<CapabilityOutputSchema, String> {
    let value: Value = serde_json::from_str(raw)
        .map_err(|err| format!("{tool_id}:output_schema_json_invalid:{err}"))?;
    let object = value
        .as_object()
        .ok_or_else(|| format!("{tool_id}:output_schema_must_be_object"))?;
    let schema_type = object
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("object")
        .to_string();
    let properties = object
        .get("properties")
        .and_then(Value::as_object)
        .ok_or_else(|| format!("{tool_id}:output_schema_properties_required"))?
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect::<BTreeMap<_, _>>();
    Ok(CapabilityOutputSchema {
        schema_type,
        properties,
    })
}

fn parse_string_array_field(
    value: Option<&Value>,
    tool_id: &str,
    field: &str,
) -> Result<Vec<String>, String> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let Some(items) = value.as_array() else {
        return Err(format!("{tool_id}:{field}_must_be_array"));
    };
    items
        .iter()
        .map(|item| {
            item.as_str()
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(str::to_string)
                .ok_or_else(|| format!("{tool_id}:{field}_item_must_be_string"))
        })
        .collect()
}

fn parse_string_array_groups_field(
    value: Option<&Value>,
    tool_id: &str,
    field: &str,
) -> Result<Vec<Vec<String>>, String> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let Some(groups) = value.as_array() else {
        return Err(format!("{tool_id}:{field}_must_be_array"));
    };
    let mut parsed = Vec::new();
    for group in groups {
        let Some(items) = group.as_array() else {
            return Err(format!("{tool_id}:{field}_group_must_be_array"));
        };
        let fields = items
            .iter()
            .map(|item| {
                item.as_str()
                    .map(str::trim)
                    .filter(|text| !text.is_empty())
                    .map(str::to_string)
                    .ok_or_else(|| format!("{tool_id}:{field}_item_must_be_string"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        if fields.len() < 2 {
            return Err(format!("{tool_id}:{field}_group_needs_at_least_two_fields"));
        }
        parsed.push(fields);
    }
    Ok(parsed)
}

fn parse_required_when_field(
    value: Option<&Value>,
    tool_id: &str,
    field: &str,
) -> Result<Vec<CapabilityRequiredWhen>, String> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let Some(items) = value.as_array() else {
        return Err(format!("{tool_id}:{field}_must_be_array"));
    };
    let mut parsed = Vec::new();
    for item in items {
        let Some(object) = item.as_object() else {
            return Err(format!("{tool_id}:{field}_item_must_be_object"));
        };
        let Some(condition_field) = object
            .get("field")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
        else {
            return Err(format!("{tool_id}:{field}_field_required"));
        };
        let values = parse_string_array_field(object.get("values"), tool_id, "values")?
            .into_iter()
            .map(|value| value.to_lowercase())
            .collect::<Vec<_>>();
        let required = parse_string_array_field(object.get("required"), tool_id, "required")?;
        if values.is_empty() || required.is_empty() {
            return Err(format!("{tool_id}:{field}_values_and_required_required"));
        }
        parsed.push(CapabilityRequiredWhen {
            field: condition_field.to_string(),
            values,
            required,
        });
    }
    Ok(parsed)
}

fn enum_fields_from_properties(
    properties: &BTreeMap<String, Value>,
) -> BTreeMap<String, Vec<String>> {
    let mut enum_fields = BTreeMap::new();
    for (key, value) in properties {
        let Some(items) = value.get("enum").and_then(Value::as_array) else {
            continue;
        };
        let allowed = items
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(|text| text.to_lowercase())
            .collect::<Vec<_>>();
        if !allowed.is_empty() {
            enum_fields.insert(key.clone(), allowed);
        }
    }
    enum_fields
}

fn is_missing_input_value(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => true,
        Some(Value::String(text)) => text.trim().is_empty(),
        Some(Value::Array(items)) => items.is_empty(),
        Some(Value::Object(object)) => object.is_empty(),
        Some(Value::Bool(_)) | Some(Value::Number(_)) => false,
    }
}

fn parse_skill_manifest(raw: &str, body: &str) -> Result<SkillManifest, String> {
    let mut top = BTreeMap::<String, String>::new();
    let mut when_to_use = String::new();
    let mut section: Option<&str> = None;

    for line in raw.lines() {
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        if line == "when_to_use: |" {
            section = Some("when_to_use");
            continue;
        }
        match section {
            Some("when_to_use") if line.starts_with("  ") => {
                if !when_to_use.is_empty() {
                    when_to_use.push('\n');
                }
                when_to_use.push_str(line.trim());
            }
            _ if !line.starts_with(' ') => {
                section = None;
                let Some((key, value)) = line.split_once(':') else {
                    return Err(format!("top_level_key_must_use_colon:{line}"));
                };
                top.insert(key.trim().to_string(), value.trim().to_string());
            }
            _ => {
                return Err(format!("unsupported_skill_manifest_line:{line}"));
            }
        }
    }

    Ok(SkillManifest {
        kind: required_top(&top, "kind")?,
        id: required_top(&top, "id")?,
        title: required_top(&top, "title")?,
        summary: required_top(&top, "summary")?,
        when_to_use,
        entry: required_top(&top, "entry")?,
        body: body.trim().to_string(),
    })
}

fn required_top(top: &BTreeMap<String, String>, key: &str) -> Result<String, String> {
    top.get(key)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("{key}_required"))
}

fn json_object(items: impl IntoIterator<Item = (&'static str, Value)>) -> Value {
    let mut object = Map::new();
    for (key, value) in items {
        object.insert(key.to_string(), value);
    }
    Value::Object(object)
}

fn input_properties_value(manifest: &ToolManifest) -> Value {
    let mut object = Map::new();
    for (key, value) in &manifest.input_schema.properties {
        object.insert(key.clone(), value.clone());
    }
    Value::Object(object)
}

fn output_properties_value(manifest: &ToolManifest) -> Value {
    let mut object = Map::new();
    for (key, value) in &manifest.output_schema.properties {
        object.insert(key.clone(), value.clone());
    }
    Value::Object(object)
}

fn required_when_value(rule: &CapabilityRequiredWhen) -> Value {
    json_object([
        ("field", Value::String(rule.field.clone())),
        (
            "values",
            Value::Array(rule.values.iter().cloned().map(Value::String).collect()),
        ),
        (
            "required",
            Value::Array(rule.required.iter().cloned().map(Value::String).collect()),
        ),
    ])
}

fn validate_manifest(manifest: &ToolManifest) -> Result<(), String> {
    if manifest.kind != "tool" {
        return Err(format!("{}:kind_must_be_tool", manifest.id));
    }
    if manifest.id.trim().is_empty() {
        return Err("tool_id_required".to_string());
    }
    if manifest.binding.binding_type != "builtin" {
        return Err(format!("{}:unsupported_binding_type", manifest.id));
    }
    if manifest.binding.name.trim().is_empty() {
        return Err(format!("{}:binding_name_required", manifest.id));
    }
    if !BUILTIN_BINDINGS.contains(&manifest.binding.name.as_str()) {
        return Err(format!("{}:unsupported_builtin_binding", manifest.id));
    }
    if manifest.prompt.when.trim().is_empty() {
        return Err(format!("{}:prompt_when_required", manifest.id));
    }
    if manifest.input_schema.schema_type != "object" {
        return Err(format!("{}:input_schema_must_be_object", manifest.id));
    }
    if manifest.output_schema.schema_type != "object" {
        return Err(format!("{}:output_schema_must_be_object", manifest.id));
    }
    if manifest.output_schema.properties.is_empty() {
        return Err(format!("{}:output_properties_required", manifest.id));
    }
    for key in manifest.input_schema.enum_fields.keys() {
        if !input_property_declared(&manifest.input_schema.properties, key) {
            return Err(format!("{}:enum_field_without_property:{key}", manifest.id));
        }
    }
    validate_manifest_rule_fields(manifest)?;
    if !manifest.example.is_object() {
        return Err(format!("{}:example_must_be_object", manifest.id));
    }
    Ok(())
}

fn validate_overlay_manifest(
    manifest: &mut ToolManifest,
    overlay_dir: &Path,
) -> Result<(), String> {
    if manifest.kind != "tool" {
        return Err(format!("{}:kind_must_be_tool", manifest.id));
    }
    if manifest.id.trim().is_empty() {
        return Err("tool_id_required".to_string());
    }
    if manifest.binding.name.trim().is_empty() {
        return Err(format!("{}:binding_name_required", manifest.id));
    }
    match manifest.binding.binding_type.as_str() {
        "builtin" => {
            if !BUILTIN_BINDINGS.contains(&manifest.binding.name.as_str()) {
                return Err(format!("{}:unsupported_builtin_binding", manifest.id));
            }
        }
        "command" => {
            let path = normalize_child_path(overlay_dir, &manifest.binding.name)?;
            if !path.is_file() {
                return Err(format!("{}:command_binding_not_file", manifest.id));
            }
            manifest.binding.command_path = Some(path);
        }
        _ => return Err(format!("{}:unsupported_binding_type", manifest.id)),
    }
    if manifest.prompt.when.trim().is_empty() {
        return Err(format!("{}:prompt_when_required", manifest.id));
    }
    if manifest.input_schema.schema_type != "object" {
        return Err(format!("{}:input_schema_must_be_object", manifest.id));
    }
    if manifest.output_schema.schema_type != "object" {
        return Err(format!("{}:output_schema_must_be_object", manifest.id));
    }
    for key in manifest.input_schema.enum_fields.keys() {
        if !input_property_declared(&manifest.input_schema.properties, key) {
            return Err(format!("{}:enum_field_without_property:{key}", manifest.id));
        }
    }
    validate_manifest_rule_fields(manifest)?;
    if !manifest.example.is_object() {
        return Err(format!("{}:example_must_be_object", manifest.id));
    }
    Ok(())
}

fn validate_skill_manifest(manifest: &SkillManifest) -> Result<(), String> {
    if manifest.kind != "skill" {
        return Err(format!("{}:kind_must_be_skill", manifest.id));
    }
    if manifest.id.trim().is_empty() {
        return Err("skill_id_required".to_string());
    }
    if manifest.title.trim().is_empty() {
        return Err(format!("{}:title_required", manifest.id));
    }
    if manifest.summary.trim().is_empty() {
        return Err(format!("{}:summary_required", manifest.id));
    }
    if manifest.when_to_use.trim().is_empty() {
        return Err(format!("{}:when_to_use_required", manifest.id));
    }
    if manifest.entry.trim().is_empty() {
        return Err(format!("{}:entry_required", manifest.id));
    }
    if manifest.body.trim().is_empty() {
        return Err(format!("{}:body_required", manifest.id));
    }
    Ok(())
}

fn input_property_declared(properties: &BTreeMap<String, Value>, key: &str) -> bool {
    properties.contains_key(key) || properties.contains_key(&format!("{key}?"))
}

fn validate_manifest_rule_fields(manifest: &ToolManifest) -> Result<(), String> {
    for group in &manifest.input_schema.required_any {
        for key in group {
            if !input_property_declared(&manifest.input_schema.properties, key) {
                return Err(format!(
                    "{}:required_any_without_property:{key}",
                    manifest.id
                ));
            }
        }
    }
    for rule in &manifest.input_schema.required_when {
        if !input_property_declared(&manifest.input_schema.properties, &rule.field) {
            return Err(format!(
                "{}:required_when_field_without_property:{}",
                manifest.id, rule.field
            ));
        }
        for key in &rule.required {
            if !input_property_declared(&manifest.input_schema.properties, key) {
                return Err(format!(
                    "{}:required_when_required_without_property:{key}",
                    manifest.id
                ));
            }
        }
    }
    Ok(())
}

fn sorted_files_with_extension(dir: &Path, extension: &str) -> Result<Vec<PathBuf>, String> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    if !dir.is_dir() {
        return Err(format!(
            "capability_subpath_not_directory:{}",
            dir.display()
        ));
    }
    let mut paths = Vec::new();
    for entry in
        fs::read_dir(dir).map_err(|err| format!("read_dir_failed:{}:{err}", dir.display()))?
    {
        let path = entry
            .map_err(|err| format!("read_dir_entry_failed:{}:{err}", dir.display()))?
            .path();
        if path.is_file()
            && path
                .extension()
                .and_then(|value| value.to_str())
                .is_some_and(|value| value == extension)
        {
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths)
}

fn sorted_dirs(dir: &Path) -> Result<Vec<PathBuf>, String> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    if !dir.is_dir() {
        return Err(format!(
            "capability_subpath_not_directory:{}",
            dir.display()
        ));
    }
    let mut paths = Vec::new();
    for entry in
        fs::read_dir(dir).map_err(|err| format!("read_dir_failed:{}:{err}", dir.display()))?
    {
        let path = entry
            .map_err(|err| format!("read_dir_entry_failed:{}:{err}", dir.display()))?
            .path();
        if path.is_dir() {
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths)
}

fn skill_entry_from_manifest(raw: &str) -> Result<String, String> {
    let mut top = BTreeMap::<String, String>::new();
    let mut section: Option<&str> = None;
    for line in raw.lines() {
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        if line == "when_to_use: |" {
            section = Some("when_to_use");
            continue;
        }
        match section {
            Some("when_to_use") if line.starts_with("  ") => {}
            _ if !line.starts_with(' ') => {
                section = None;
                let Some((key, value)) = line.split_once(':') else {
                    return Err(format!("top_level_key_must_use_colon:{line}"));
                };
                top.insert(key.trim().to_string(), value.trim().to_string());
            }
            _ => return Err(format!("unsupported_skill_manifest_line:{line}")),
        }
    }
    required_top(&top, "entry")
}

fn normalize_child_path(parent: &Path, child: &str) -> Result<PathBuf, String> {
    let path = Path::new(child);
    if path.is_absolute()
        || child
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(format!("invalid_relative_resource_path:{child}"));
    }
    Ok(parent.join(path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn builtin_registry_loads_manifest_tools() {
        let registry = CapabilityRegistry::builtin();

        assert!(registry.contains_tool("memmgr"));
        assert!(registry.contains_tool("capmgr"));
        assert!(registry.contains_tool("run_bash"));
        assert!(registry.contains_tool("shell_job_status"));
        assert!(!registry.contains_tool("query_memory"));
    }

    #[test]
    fn registry_renders_prompt_tool_catalog_from_manifests() {
        let registry = CapabilityRegistry::builtin();
        let rendered = registry.render_tool_catalog_json();

        assert!(rendered.contains("\"memmgr\""));
        assert!(rendered.contains("\"capmgr\""));
        assert!(rendered.contains("\"run_bash\""));
        assert!(rendered.contains("\"shell_job_status\""));
        assert!(rendered.contains("Unified local memory manager"));
        assert!(rendered.contains("\"required_any\""));
        assert!(rendered.contains("\"required_when\""));
        assert!(rendered.contains("\"result\""));
        assert!(rendered.contains("\"command\""));
        assert!(rendered.contains("\"read_back_command\""));
        assert!(rendered.contains("background=true"));
        assert!(rendered.contains("Foreground returns status and bounded output"));
        assert!(rendered.contains("\"op\""));
        assert!(rendered.contains("\"inspect\""));
        assert!(rendered.contains("memory_conflict"));
        assert!(!rendered.contains("\"output\": {"));
        assert!(!rendered.contains("\"description\""));
        assert!(!rendered.contains("Shell command to execute."));
        assert!(!rendered.contains("Background job id when background=true."));
    }

    #[test]
    fn registry_exposes_executor_binding_names() {
        let registry = CapabilityRegistry::builtin();

        assert_eq!(registry.binding_name("memmgr"), Some("memmgr"));
        assert_eq!(registry.binding_name("capmgr"), Some("capmgr"));
        assert_eq!(registry.binding_name("run_bash"), Some("run_bash"));
        assert_eq!(
            registry.binding_name("shell_job_status"),
            Some("shell_job_status")
        );
        assert_eq!(registry.binding_name("future_tool"), None);
    }

    #[test]
    fn registry_validates_required_input_fields_from_manifest() {
        let registry = CapabilityRegistry::builtin();

        assert!(registry
            .validate_action_input("capmgr", &json_object([]))
            .unwrap_err()
            .contains("input.op_required"));
        assert!(registry
            .validate_action_input(
                "memmgr",
                &json_object([("type", Value::String("durable".to_string()))])
            )
            .unwrap_err()
            .contains("input.op_required"));
        assert!(registry
            .validate_action_input(
                "shell_job_status",
                &json_object([("job_id", Value::String("job_1".to_string()))])
            )
            .unwrap_err()
            .contains("input.timeout_ms_required"));
        assert!(registry
            .validate_action_input("run_bash", &json_object([]))
            .unwrap_err()
            .contains("input.any_required:command|read_back_command"));
        assert!(registry
            .validate_action_input(
                "run_bash",
                &json_object([("read_back_command", Value::String("pwd".to_string()))])
            )
            .is_ok());
        assert!(registry
            .validate_action_input(
                "capmgr",
                &json_object([("op", Value::String("load".to_string()))])
            )
            .unwrap_err()
            .contains("input.kind_required_when_op=load"));
        assert!(registry
            .validate_action_input(
                "capmgr",
                &json_object([
                    ("op", Value::String("inspect".to_string())),
                    ("kind", Value::String("skill".to_string())),
                ])
            )
            .unwrap_err()
            .contains("input.id_required_when_op=inspect"));
        assert!(registry
            .validate_action_input(
                "capmgr",
                &json_object([("op", Value::String("list".to_string()))])
            )
            .is_ok());
        assert!(registry
            .validate_action_input(
                "capmgr",
                &json_object([("op", Value::String("remove".to_string()))])
            )
            .unwrap_err()
            .contains("input.op_unsupported:remove"));
        assert!(registry
            .validate_action_input(
                "capmgr",
                &json_object([
                    ("op", Value::String("list".to_string())),
                    ("kind", Value::String("resource".to_string())),
                ])
            )
            .unwrap_err()
            .contains("input.kind_unsupported:resource"));
    }

    #[test]
    fn registry_derives_validation_rules_from_json_schema_idl() {
        let registry = CapabilityRegistry::builtin();
        let catalog = registry.tool_catalog_value();
        let capmgr = catalog
            .get("capmgr")
            .and_then(Value::as_object)
            .expect("capmgr catalog entry");

        assert!(capmgr
            .get("input")
            .and_then(Value::as_str)
            .is_some_and(|text| text.contains("op=list") && text.contains("op=load")));
        assert!(capmgr
            .get("required_when")
            .and_then(Value::as_array)
            .is_some_and(|rules| !rules.is_empty()));
        assert!(registry
            .validate_action_input(
                "capmgr",
                &json_object([("op", Value::String("load".to_string()))])
            )
            .unwrap_err()
            .contains("input.kind_required_when_op=load"));
        assert!(registry
            .validate_action_input(
                "capmgr",
                &json_object([("op", Value::String("inspect".to_string()))])
            )
            .unwrap_err()
            .contains("input.kind_required_when_op=inspect"));
        assert!(registry
            .validate_action_input(
                "run_bash",
                &json_object([
                    ("command", Value::String("pwd".to_string())),
                    ("mode", Value::String("daemon".to_string())),
                ])
            )
            .unwrap_err()
            .contains("input.mode_unsupported:daemon"));
    }

    #[test]
    fn registry_enriches_static_prompt_tool_catalog() {
        let registry = CapabilityRegistry::builtin();
        let enriched = registry.enrich_static_prompt(
            r#"{"Tool_capability":{"tool_catalog":{"stale_tool":{"when":"old"}}}}"#,
        );

        assert!(enriched.contains("\"memmgr\""));
        assert!(enriched.contains("\"skill_headers\""));
        assert!(!enriched.contains("\"release_quality_gate\""));
        assert!(enriched.contains("\"run_bash\""));
        assert!(!enriched.contains("stale_tool"));
    }

    #[test]
    fn capmgr_can_list_and_load_skill_content() {
        let dir = temp_release_quality_skill_overlay("capmgr_skill_load");
        let registry = CapabilityRegistry::builtin_with_overlay_dir(&dir).unwrap();

        let list = registry.list_text("skill");
        assert!(list.contains("Action result: capmgr"));
        assert!(list.contains("release_quality_gate"));
        assert!(list.contains("Release quality gate"));

        let loaded = registry.load_text("skill", "release_quality_gate");
        assert!(loaded.contains("op: load"));
        assert!(loaded.contains("# Release Quality Gate"));
        assert!(loaded.contains("Run the relevant local tests"));

        let loaded_tool = registry.load_text("tool", "run_bash");
        assert!(loaded_tool.contains("kind: tool"));
        assert!(loaded_tool.contains("input:"));
        assert!(loaded_tool.contains("output:"));
        assert!(loaded_tool.contains("approval_status"));
        assert!(loaded_tool.contains("approved_by_user"));
    }

    #[test]
    fn registry_loads_runtime_overlay_tools_and_skills_from_files() {
        let dir = temp_capability_dir("runtime_overlay");
        let tools_dir = dir.join("tools");
        let skill_dir = dir.join("skills").join("log_review");
        fs::create_dir_all(&tools_dir).unwrap();
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            tools_dir.join("local_echo.yaml"),
            r#"kind: tool
id: local_echo
binding_type: builtin
binding_name: run_bash
description: Echo a bounded local string through Bash.
prompt_when: |
  Use this runtime overlay tool only when a bounded echo command is enough.
input_properties:
  command: string
required:
  - command
example_json: |
  {
    "action": "local_echo",
    "intent": "Echo a short string.",
    "input": {
      "command": "printf hello"
    }
  }
"#,
        )
        .unwrap();
        fs::write(
            skill_dir.join("skill.yaml"),
            r#"kind: skill
id: log_review
title: Log review
summary: Runtime-loaded log review checklist.
entry: instructions.md
when_to_use: |
  Use for structured log review.
"#,
        )
        .unwrap();
        fs::write(
            skill_dir.join("instructions.md"),
            "# Runtime Log Review\n\nCheck timestamps.",
        )
        .unwrap();

        let registry = CapabilityRegistry::builtin_with_overlay_dir(&dir).unwrap();

        assert_eq!(registry.binding_name("local_echo"), Some("run_bash"));
        assert!(registry
            .tool_catalog_value()
            .get("local_echo")
            .and_then(|tool| tool.get("output"))
            .is_none());
        assert!(registry
            .render_tool_catalog_json()
            .contains("\"local_echo\""));
        assert!(registry
            .load_text("skill", "log_review")
            .contains("# Runtime Log Review"));
        assert!(registry
            .skill_headers_value()
            .to_string()
            .contains("Runtime-loaded log review checklist"));
    }

    #[test]
    fn registry_rejects_overlay_tool_without_executor_binding() {
        let dir = temp_capability_dir("bad_runtime_overlay");
        let tools_dir = dir.join("tools");
        fs::create_dir_all(&tools_dir).unwrap();
        fs::write(
            tools_dir.join("ghost.yaml"),
            r#"kind: tool
id: ghost
binding_type: builtin
binding_name: missing_executor
description: This tool has no executor.
prompt_when: |
  Should not load.
input_properties:
  query: string
example_json: |
  {
    "action": "ghost",
    "intent": "Should not execute.",
    "input": {
      "query": "x"
    }
  }
"#,
        )
        .unwrap();

        let err = CapabilityRegistry::builtin_with_overlay_dir(&dir).unwrap_err();
        assert!(err.contains("ghost:unsupported_builtin_binding"));
    }

    fn temp_capability_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("timem_capability_test_{name}_{nanos}"))
    }

    fn temp_release_quality_skill_overlay(name: &str) -> PathBuf {
        let dir = temp_capability_dir(name);
        let skill_dir = dir.join("skills").join("release_quality_gate");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.yaml"),
            r#"kind: skill
id: release_quality_gate
title: Release quality gate
summary: Verify tests, CI, release notes, sensitive information, and version state before publishing a release.
entry: instructions.md
when_to_use: |
  Use when preparing, auditing, or deciding whether to publish a Timem release.
"#,
        )
        .unwrap();
        fs::write(
            skill_dir.join("instructions.md"),
            "# Release Quality Gate\n\nRun the relevant local tests.\n",
        )
        .unwrap();
        dir
    }
}
