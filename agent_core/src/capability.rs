use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::prompt_spec::replace_markdown_placeholder_with_text;

const MEMMGR_MANIFEST: &str = include_str!("../../resources/capabilities/tools/memmgr.yaml");
const CAPMGR_MANIFEST: &str = include_str!("../../resources/capabilities/tools/capmgr.yaml");
const RUN_BASH_MANIFEST: &str = include_str!("../../resources/capabilities/tools/run_bash.yaml");
const SHELL_JOB_STATUS_MANIFEST: &str =
    include_str!("../../resources/capabilities/tools/shell_job_status.yaml");
const SELF_TOOL_MANIFEST: &str = include_str!("../../resources/capabilities/tools/self_tool.yaml");

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityBinding {
    pub binding_type: String,
    pub name: String,
    pub command_path: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityPrompt {
    pub description: String,
    pub synopsis: String,
    pub input: String,
    pub result: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityInputSchema {
    pub schema_type: String,
    pub required: Vec<String>,
    pub required_any: Vec<Vec<String>>,
    pub required_when: Vec<CapabilityRequiredWhen>,
    pub required_any_when: Vec<CapabilityRequiredWhen>,
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
    pub conditions: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolManifest {
    pub kind: String,
    pub id: String,
    pub binding: CapabilityBinding,
    pub requires_host: Option<String>,
    pub summary: String,
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
    host_profile: CapabilityHostProfile,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapabilityHostProfile {
    pub local_command_execution: bool,
}

impl CapabilityHostProfile {
    pub fn detect() -> Self {
        Self {
            local_command_execution: local_command_execution_available(),
        }
    }

    pub fn with_local_command_execution() -> Self {
        Self {
            local_command_execution: true,
        }
    }

    pub fn without_local_command_execution() -> Self {
        Self {
            local_command_execution: false,
        }
    }

    fn supports(self, requirement: Option<&str>) -> bool {
        match requirement.map(str::trim).filter(|value| !value.is_empty()) {
            None => true,
            Some("local_command_execution") => self.local_command_execution,
            Some(_) => false,
        }
    }

    fn supports_tool(self, manifest: &ToolManifest) -> bool {
        if !self.supports(manifest.requires_host.as_deref()) {
            return false;
        }
        if manifest.binding.binding_type == "command" {
            return self.local_command_execution;
        }
        true
    }
}

fn local_command_execution_available() -> bool {
    #[cfg(unix)]
    {
        Path::new("/bin/sh").is_file()
    }
    #[cfg(not(unix))]
    {
        false
    }
}

impl CapabilityRegistry {
    pub fn from_manifests(
        tool_manifests: &[&str],
        skill_manifests: &[(&str, &str)],
    ) -> Result<Self, String> {
        Self::from_manifests_for_host(
            tool_manifests,
            skill_manifests,
            CapabilityHostProfile::detect(),
        )
    }

    pub fn from_manifests_for_host(
        tool_manifests: &[&str],
        skill_manifests: &[(&str, &str)],
        profile: CapabilityHostProfile,
    ) -> Result<Self, String> {
        let mut tools = BTreeMap::new();
        for raw in tool_manifests {
            let manifest = parse_tool_manifest(raw)?;
            validate_manifest(&manifest)?;
            if !profile.supports_tool(&manifest) {
                continue;
            }
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
        Ok(Self {
            tools,
            skills,
            host_profile: profile,
        })
    }

    pub fn builtin() -> Self {
        Self::builtin_for_host(CapabilityHostProfile::detect())
    }

    pub fn builtin_for_host(profile: CapabilityHostProfile) -> Self {
        Self::from_manifests_for_host(
            &[
                MEMMGR_MANIFEST,
                CAPMGR_MANIFEST,
                RUN_BASH_MANIFEST,
                SHELL_JOB_STATUS_MANIFEST,
                SELF_TOOL_MANIFEST,
            ],
            &[],
            profile,
        )
        .expect("builtin capability manifests must be valid")
    }

    pub fn builtin_with_overlay_dir(dir: impl AsRef<Path>) -> Result<Self, String> {
        Self::builtin_with_overlay_dir_for_host(dir, CapabilityHostProfile::detect())
    }

    pub fn builtin_with_overlay_dir_for_host(
        dir: impl AsRef<Path>,
        profile: CapabilityHostProfile,
    ) -> Result<Self, String> {
        let mut registry = Self::builtin_for_host(profile);
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
            if !self.host_profile.supports_tool(&manifest) {
                continue;
            }
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

    pub fn tool_count(&self) -> usize {
        self.tools.len()
    }

    pub fn skill_count(&self) -> usize {
        self.skills.len()
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
        if let Some(object) = input.as_object() {
            for key in object.keys() {
                if !input_property_declared(&manifest.input_schema.properties, key) {
                    return Err(format!("input.{key}_unsupported"));
                }
            }
        }
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
            if !required_when_matches(rule, input) {
                continue;
            }
            for key in &rule.required {
                if is_missing_input_value(input.get(key)) {
                    return Err(required_when_error(key, rule, input));
                }
            }
        }
        for rule in &manifest.input_schema.required_any_when {
            if !required_when_matches(rule, input) {
                continue;
            }
            if rule
                .required
                .iter()
                .all(|key| is_missing_input_value(input.get(key)))
            {
                return Err(required_any_when_error(rule, input));
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

    pub fn render_skill_headers_markdown(&self) -> String {
        if self.skills.is_empty() {
            return "- No optional skills are currently loaded.".to_string();
        }
        self.skills
            .values()
            .map(|skill| {
                format!(
                    "#### `{}`\n- Title: {}\n- Summary: {}\n- Use when: {}",
                    skill.id,
                    one_line(&skill.title),
                    one_line(&skill.summary),
                    one_line(&skill.when_to_use)
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    pub fn list_text(&self, kind: &str) -> String {
        match kind {
            "tool" => {
                let rows = self
                    .tools
                    .values()
                    .map(|tool| {
                        format!(
                            "- id={} binding={}:{} summary={}",
                            tool.id, tool.binding.binding_type, tool.binding.name, tool.summary
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
                    "Action result: capmgr\nop: load\nkind: tool\nid: {}\nbinding: {}:{}\nmanual:\n{}",
                    tool.id,
                    tool.binding.binding_type,
                    tool.binding.name,
                    render_tool_manifest_markdown(tool)
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
                "description".to_string(),
                Value::String(manifest.prompt.description.clone()),
            );
            item.insert(
                "input".to_string(),
                Value::String(manifest.prompt.input.clone()),
            );
            item.insert(
                "result".to_string(),
                Value::String(manifest.prompt.result.clone()),
            );
            item.insert(
                "input_schema".to_string(),
                input_schema_value(&manifest.input_schema),
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
            if !manifest.input_schema.required_any_when.is_empty() {
                item.insert(
                    "required_any_when".to_string(),
                    Value::Array(
                        manifest
                            .input_schema
                            .required_any_when
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

    pub fn render_tool_catalog_markdown(&self) -> String {
        self.tools
            .values()
            .map(render_tool_manifest_markdown)
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    pub fn enrich_static_prompt(&self, static_prompt: &str) -> String {
        if let Some(with_catalog) = replace_markdown_placeholder_with_text(
            static_prompt,
            "{{TOOL_CATALOG}}",
            &self.render_tool_catalog_markdown(),
        ) {
            if let Some(with_skills) = replace_markdown_placeholder_with_text(
                &with_catalog,
                "{{SKILL_HEADERS}}",
                &self.render_skill_headers_markdown(),
            ) {
                return with_skills;
            }
        }

        static_prompt.to_string()
    }
}

fn render_tool_manifest_markdown(manifest: &ToolManifest) -> String {
    let mut lines = Vec::new();
    lines.push(format!("#### `{}`", manifest.id));
    lines.push(String::new());
    lines.push("**Name**".to_string());
    lines.push(format!(
        "`{}` - {}",
        manifest.id,
        one_line(&manifest.summary)
    ));
    lines.push(String::new());
    lines.push("**Synopsis**".to_string());
    lines.extend(render_synopsis(manifest));
    lines.push(String::new());
    lines.push("**Description**".to_string());
    lines.push(one_line(&manifest.prompt.description));
    lines.push(String::new());
    if !manifest.prompt.input.trim().is_empty() {
        lines.push("**Usage**".to_string());
        lines.push(one_line(&manifest.prompt.input));
        lines.push(String::new());
    }
    lines.push("**Options**".to_string());
    for (name, property) in &manifest.input_schema.properties {
        lines.push(format!("- `{name}`: {}", property_option_text(property)));
    }
    if !manifest.input_schema.required.is_empty() {
        lines.push(format!(
            "- Required: {}",
            inline_code_list(&manifest.input_schema.required)
        ));
    }
    if !manifest.input_schema.required_any.is_empty() {
        let groups = manifest
            .input_schema
            .required_any
            .iter()
            .map(|group| inline_code_list(group))
            .collect::<Vec<_>>()
            .join("; ");
        lines.push(format!("- Required one of: {groups}"));
    }
    if !manifest.input_schema.required_when.is_empty() {
        let rules = manifest
            .input_schema
            .required_when
            .iter()
            .map(required_when_markdown)
            .collect::<Vec<_>>()
            .join("; ");
        lines.push(format!("- Conditional: {rules}"));
    }
    if !manifest.input_schema.required_any_when.is_empty() {
        let rules = manifest
            .input_schema
            .required_any_when
            .iter()
            .map(required_any_when_markdown)
            .collect::<Vec<_>>()
            .join("; ");
        lines.push(format!("- Conditional one of: {rules}"));
    }
    lines.push(String::new());
    lines.push("**Result**".to_string());
    lines.push(one_line(&manifest.prompt.result));
    lines.push(
        "If args do not match this tool spec, runtime asks you to repair the response before executing the tool."
            .to_string(),
    );
    lines.join("\n")
}

fn render_synopsis(manifest: &ToolManifest) -> Vec<String> {
    if manifest.prompt.synopsis.trim().is_empty() {
        return vec![format!("`{}`", synopsis_from_schema(manifest))];
    }
    manifest
        .prompt
        .synopsis
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| format!("`{line}`"))
        .collect()
}

fn synopsis_from_schema(manifest: &ToolManifest) -> String {
    let mut parts = vec![manifest.id.clone()];
    let schema = &manifest.input_schema;
    let mut used = Vec::<String>::new();

    for name in &schema.required {
        if let Some(property) = schema.properties.get(name) {
            parts.push(format!("{name}={}", synopsis_placeholder(property)));
            used.push(name.clone());
        }
    }

    for group in &schema.required_any {
        let alternatives = group
            .iter()
            .filter_map(|name| {
                schema
                    .properties
                    .get(name)
                    .map(|property| format!("{name}={}", synopsis_placeholder(property)))
            })
            .collect::<Vec<_>>();
        if !alternatives.is_empty() {
            parts.push(format!("({})", alternatives.join("|")));
            used.extend(group.iter().cloned());
        }
    }

    for name in schema.properties.keys() {
        if used.iter().any(|used_name| used_name == name) {
            continue;
        }
        if synopsis_omits_optional_field(name) {
            continue;
        }
        if let Some(property) = schema.properties.get(name) {
            parts.push(format!("[{name}={}]", synopsis_placeholder(property)));
        }
    }
    parts.join(" ")
}

fn synopsis_omits_optional_field(_name: &str) -> bool {
    false
}

fn synopsis_placeholder(property: &Value) -> String {
    if let Some(values) = property.get("enum").and_then(Value::as_array) {
        let allowed = values.iter().filter_map(Value::as_str).collect::<Vec<_>>();
        if !allowed.is_empty() {
            return format!("<{}>", allowed.join("|"));
        }
    }
    match property
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("value")
    {
        "string" => "<string>".to_string(),
        "integer" | "number" => "<n>".to_string(),
        "boolean" => "<true|false>".to_string(),
        "array" => "<list>".to_string(),
        "object" => "<object>".to_string(),
        other => format!("<{other}>"),
    }
}

fn property_option_text(property: &Value) -> String {
    let mut text = property
        .get("description")
        .and_then(Value::as_str)
        .map(one_line)
        .unwrap_or_else(|| "Tool input field.".to_string());
    if let Some(values) = property.get("enum").and_then(Value::as_array) {
        let allowed = values
            .iter()
            .filter_map(Value::as_str)
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        if !allowed.is_empty() {
            text.push_str(&format!(" Allowed: {}.", inline_code_list(&allowed)));
        }
    }
    text
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn inline_code_list(items: &[String]) -> String {
    items
        .iter()
        .map(|item| format!("`{}`", item))
        .collect::<Vec<_>>()
        .join(", ")
}

fn parse_tool_manifest(raw: &str) -> Result<ToolManifest, String> {
    let mut top = BTreeMap::<String, String>::new();
    let mut input_properties = BTreeMap::<String, Value>::new();
    let mut output_properties = BTreeMap::<String, Value>::new();
    let mut required = Vec::<String>::new();
    let mut required_any = Vec::<Vec<String>>::new();
    let mut required_when = Vec::<CapabilityRequiredWhen>::new();
    let mut required_any_when = Vec::<CapabilityRequiredWhen>::new();
    let mut enum_fields = BTreeMap::<String, Vec<String>>::new();
    let mut prompt_description = String::new();
    let mut prompt_synopsis = String::new();
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
        if line == "description: |" {
            section = Some("description");
            continue;
        }
        if line == "prompt_input: |" {
            section = Some("prompt_input");
            continue;
        }
        if line == "prompt_synopsis: |" {
            section = Some("prompt_synopsis");
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
        if line == "required_any_when:" {
            section = Some("required_any_when");
            continue;
        }
        if line == "enum_fields:" {
            section = Some("enum_fields");
            continue;
        }
        match section {
            Some("description") if line.starts_with("  ") => {
                if !prompt_description.is_empty() {
                    prompt_description.push('\n');
                }
                prompt_description.push_str(line.trim());
            }
            Some("prompt_input") if line.starts_with("  ") => {
                if !prompt_input.is_empty() {
                    prompt_input.push('\n');
                }
                prompt_input.push_str(line.trim());
            }
            Some("prompt_synopsis") if line.starts_with("  ") => {
                if !prompt_synopsis.is_empty() {
                    prompt_synopsis.push('\n');
                }
                prompt_synopsis.push_str(line.trim());
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
            Some("required_when") | Some("required_any_when") if line.starts_with("  ") => {
                let trimmed = line.trim();
                let Some((condition, fields)) = trimmed.split_once(':') else {
                    return Err(format!(
                        "{}_must_use_condition_colon_fields:{trimmed}",
                        section.unwrap_or("required_when")
                    ));
                };
                let Some((field, values)) = condition.split_once('=') else {
                    return Err(format!(
                        "{}_condition_must_use_equals:{condition}",
                        section.unwrap_or("required_when")
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
                    return Err(format!(
                        "{}_invalid:{trimmed}",
                        section.unwrap_or("required_when")
                    ));
                }
                let rule = CapabilityRequiredWhen {
                    field: field.trim().to_string(),
                    values,
                    required,
                    conditions: BTreeMap::new(),
                };
                if section == Some("required_any_when") {
                    required_any_when.push(rule);
                } else {
                    required_when.push(rule);
                }
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
            required_any_when,
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
        requires_host: optional_top(&top, "requires_host"),
        summary: required_top(&top, "summary")?,
        prompt: CapabilityPrompt {
            description: prompt_description,
            synopsis: prompt_synopsis,
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
    let required_any_when = parse_required_when_field(
        object.get("x-required-any-when"),
        tool_id,
        "x-required-any-when",
    )?;
    let enum_fields = enum_fields_from_properties(&properties);
    Ok(CapabilityInputSchema {
        schema_type,
        required,
        required_any,
        required_when,
        required_any_when,
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
        let condition_field = object
            .get("field")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_string);
        let values = parse_string_array_field(object.get("values"), tool_id, "values")?
            .into_iter()
            .map(|value| value.to_lowercase())
            .collect::<Vec<_>>();
        let conditions = parse_required_when_conditions(object.get("when"), tool_id, field)?;
        let required = parse_string_array_field(object.get("required"), tool_id, "required")?;
        if condition_field.is_none() && conditions.is_empty() {
            return Err(format!("{tool_id}:{field}_condition_required"));
        }
        if condition_field.is_some() && values.is_empty() {
            return Err(format!("{tool_id}:{field}_values_required"));
        }
        if required.is_empty() {
            return Err(format!("{tool_id}:{field}_values_and_required_required"));
        }
        parsed.push(CapabilityRequiredWhen {
            field: condition_field.unwrap_or_default(),
            values,
            required,
            conditions,
        });
    }
    Ok(parsed)
}

fn parse_required_when_conditions(
    value: Option<&Value>,
    tool_id: &str,
    field: &str,
) -> Result<BTreeMap<String, Vec<String>>, String> {
    let Some(value) = value else {
        return Ok(BTreeMap::new());
    };
    let Some(object) = value.as_object() else {
        return Err(format!("{tool_id}:{field}_when_must_be_object"));
    };
    let mut conditions = BTreeMap::new();
    for (key, raw_values) in object {
        let values = if let Some(text) = raw_values.as_str() {
            vec![text.trim().to_lowercase()]
        } else {
            parse_string_array_field(Some(raw_values), tool_id, "when")?
                .into_iter()
                .map(|value| value.to_lowercase())
                .collect::<Vec<_>>()
        };
        let values = values
            .into_iter()
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>();
        if values.is_empty() {
            return Err(format!("{tool_id}:{field}_when_values_required:{key}"));
        }
        conditions.insert(key.trim().to_string(), values);
    }
    Ok(conditions)
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

fn required_when_matches(rule: &CapabilityRequiredWhen, input: &Value) -> bool {
    if !rule.field.is_empty() {
        let Some(value) = input.get(&rule.field).and_then(Value::as_str) else {
            return false;
        };
        let normalized = value.trim().to_lowercase();
        if !allowed_values_match(&rule.values, &normalized) {
            return false;
        }
    }
    for (field, allowed_values) in &rule.conditions {
        let Some(value) = input.get(field).and_then(Value::as_str) else {
            return false;
        };
        let normalized = value.trim().to_lowercase();
        if !allowed_values_match(allowed_values, &normalized) {
            return false;
        }
    }
    true
}

fn allowed_values_match(allowed_values: &[String], normalized: &str) -> bool {
    allowed_values
        .iter()
        .any(|allowed| allowed == "*" || allowed == normalized)
}

fn required_when_error(key: &str, rule: &CapabilityRequiredWhen, input: &Value) -> String {
    if !rule.conditions.is_empty() {
        let condition = rule
            .conditions
            .iter()
            .map(|(field, values)| {
                let value = input
                    .get(field)
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .unwrap_or_else(|| values.first().map(String::as_str).unwrap_or(""));
                format!("{field}={value}")
            })
            .collect::<Vec<_>>()
            .join(",");
        return format!("input.{key}_required_when_{condition}");
    }
    let value = input
        .get(&rule.field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| rule.values.first().map(String::as_str).unwrap_or(""));
    format!("input.{key}_required_when_{}={value}", rule.field)
}

fn required_any_when_error(rule: &CapabilityRequiredWhen, input: &Value) -> String {
    let fields = rule.required.join("|");
    if !rule.conditions.is_empty() {
        let condition = rule
            .conditions
            .iter()
            .map(|(field, values)| {
                let value = input
                    .get(field)
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .unwrap_or_else(|| values.first().map(String::as_str).unwrap_or(""));
                format!("{field}={value}")
            })
            .collect::<Vec<_>>()
            .join(",");
        return format!("input.any_required_when_{fields}_when_{condition}");
    }
    let value = input
        .get(&rule.field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| rule.values.first().map(String::as_str).unwrap_or(""));
    format!(
        "input.any_required_when_{fields}_when_{}={value}",
        rule.field
    )
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

fn optional_top(top: &BTreeMap<String, String>, key: &str) -> Option<String> {
    top.get(key)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn json_object(items: impl IntoIterator<Item = (&'static str, Value)>) -> Value {
    let mut object = Map::new();
    for (key, value) in items {
        object.insert(key.to_string(), value);
    }
    Value::Object(object)
}

fn required_when_value(rule: &CapabilityRequiredWhen) -> Value {
    let mut object = Map::new();
    if !rule.field.is_empty() {
        object.insert("field".to_string(), Value::String(rule.field.clone()));
        object.insert(
            "values".to_string(),
            Value::Array(rule.values.iter().cloned().map(Value::String).collect()),
        );
    }
    if !rule.conditions.is_empty() {
        let mut when = Map::new();
        for (field, values) in &rule.conditions {
            when.insert(
                field.clone(),
                Value::Array(values.iter().cloned().map(Value::String).collect()),
            );
        }
        object.insert("when".to_string(), Value::Object(when));
    }
    object.insert(
        "required".to_string(),
        Value::Array(rule.required.iter().cloned().map(Value::String).collect()),
    );
    Value::Object(object)
}

fn required_when_markdown(rule: &CapabilityRequiredWhen) -> String {
    let condition = if !rule.conditions.is_empty() {
        let mut conditions = rule.conditions.iter().collect::<Vec<_>>();
        conditions.sort_by_key(|(field, _)| condition_field_order(field));
        conditions
            .into_iter()
            .map(|(field, values)| format!("{field}={}", pipe_values(values)))
            .collect::<Vec<_>>()
            .join(", ")
    } else {
        format!("{}={}", rule.field, pipe_values(&rule.values))
    };
    format!("({condition}) requires {}", rule.required.join(", "))
}

fn required_any_when_markdown(rule: &CapabilityRequiredWhen) -> String {
    let condition = if !rule.conditions.is_empty() {
        let mut conditions = rule.conditions.iter().collect::<Vec<_>>();
        conditions.sort_by_key(|(field, _)| condition_field_order(field));
        conditions
            .into_iter()
            .map(|(field, values)| format!("{field}={}", pipe_values(values)))
            .collect::<Vec<_>>()
            .join(", ")
    } else {
        format!("{}={}", rule.field, pipe_values(&rule.values))
    };
    format!("({condition}) requires one of {}", rule.required.join("|"))
}

fn pipe_values(values: &[String]) -> String {
    values.join("|")
}

fn condition_field_order(field: &str) -> (usize, &str) {
    let rank = match field {
        "type" => 0,
        "op" => 1,
        "kind" => 2,
        _ => 3,
    };
    (rank, field)
}

fn input_schema_value(schema: &CapabilityInputSchema) -> Value {
    let mut object = Map::new();
    object.insert(
        "type".to_string(),
        Value::String(schema.schema_type.clone()),
    );
    object.insert(
        "properties".to_string(),
        Value::Object(
            schema
                .properties
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
        ),
    );
    if !schema.required.is_empty() {
        object.insert(
            "required".to_string(),
            Value::Array(schema.required.iter().cloned().map(Value::String).collect()),
        );
    }
    if !schema.required_any.is_empty() {
        object.insert(
            "required_any".to_string(),
            Value::Array(
                schema
                    .required_any
                    .iter()
                    .map(|group| Value::Array(group.iter().cloned().map(Value::String).collect()))
                    .collect(),
            ),
        );
    }
    if !schema.required_when.is_empty() {
        object.insert(
            "required_when".to_string(),
            Value::Array(
                schema
                    .required_when
                    .iter()
                    .map(required_when_value)
                    .collect(),
            ),
        );
    }
    if !schema.required_any_when.is_empty() {
        object.insert(
            "required_any_when".to_string(),
            Value::Array(
                schema
                    .required_any_when
                    .iter()
                    .map(required_when_value)
                    .collect(),
            ),
        );
    }
    Value::Object(object)
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
    if !crate::tool_registry::BUILTIN_TOOL_BINDINGS.contains(&manifest.binding.name.as_str()) {
        return Err(format!("{}:unsupported_builtin_binding", manifest.id));
    }
    validate_host_requirement(manifest)?;
    if manifest.prompt.description.trim().is_empty() {
        return Err(format!("{}:description_required", manifest.id));
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
    validate_host_requirement(manifest)?;
    match manifest.binding.binding_type.as_str() {
        "builtin" => {
            if !crate::tool_registry::BUILTIN_TOOL_BINDINGS
                .contains(&manifest.binding.name.as_str())
            {
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
    if manifest.prompt.description.trim().is_empty() {
        return Err(format!("{}:description_required", manifest.id));
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

fn validate_host_requirement(manifest: &ToolManifest) -> Result<(), String> {
    match manifest.requires_host.as_deref() {
        None | Some("local_command_execution") => Ok(()),
        Some(requirement) => Err(format!(
            "{}:unsupported_requires_host:{requirement}",
            manifest.id
        )),
    }
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
        if !rule.field.is_empty()
            && !input_property_declared(&manifest.input_schema.properties, &rule.field)
        {
            return Err(format!(
                "{}:required_when_field_without_property:{}",
                manifest.id, rule.field
            ));
        }
        for key in rule.conditions.keys() {
            if !input_property_declared(&manifest.input_schema.properties, key) {
                return Err(format!(
                    "{}:required_when_condition_without_property:{key}",
                    manifest.id
                ));
            }
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
    for rule in &manifest.input_schema.required_any_when {
        if !rule.field.is_empty()
            && !input_property_declared(&manifest.input_schema.properties, &rule.field)
        {
            return Err(format!(
                "{}:required_any_when_field_without_property:{}",
                manifest.id, rule.field
            ));
        }
        for key in rule.conditions.keys() {
            if !input_property_declared(&manifest.input_schema.properties, key) {
                return Err(format!(
                    "{}:required_any_when_condition_without_property:{key}",
                    manifest.id
                ));
            }
        }
        for key in &rule.required {
            if !input_property_declared(&manifest.input_schema.properties, key) {
                return Err(format!(
                    "{}:required_any_when_required_without_property:{key}",
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
        assert!(registry.contains_tool("self_tool"));
        assert!(!registry.contains_tool("tool_job_status"));
        assert!(!registry.contains_tool("query_memory"));
    }

    #[test]
    fn host_profile_filters_local_command_capabilities() {
        let registry = CapabilityRegistry::builtin_for_host(
            CapabilityHostProfile::without_local_command_execution(),
        );

        assert!(registry.contains_tool("memmgr"));
        assert!(registry.contains_tool("capmgr"));
        assert!(registry.contains_tool("self_tool"));
        assert!(!registry.contains_tool("run_bash"));
        assert!(!registry.contains_tool("shell_job_status"));
        assert!(!registry.contains_tool("tool_job_status"));
        assert_eq!(registry.binding_name("run_bash"), None);
        assert!(registry
            .validate_action_input(
                "run_bash",
                &json_object([("cmd", Value::String("pwd".to_string()))])
            )
            .unwrap_err()
            .contains("unsupported_action:run_bash"));

        let rendered = registry.render_tool_catalog_markdown();
        assert!(!rendered.contains("#### `run_bash`"));
        assert!(!rendered.contains("#### `shell_job_status`"));
        assert!(!rendered.contains("#### `tool_job_status`"));
    }

    #[test]
    fn host_profile_can_enable_local_command_capabilities_without_shell_ui() {
        let registry = CapabilityRegistry::builtin_for_host(
            CapabilityHostProfile::with_local_command_execution(),
        );

        assert!(registry.contains_tool("run_bash"));
        assert!(registry.contains_tool("shell_job_status"));
        assert!(!registry.contains_tool("tool_job_status"));
        assert_eq!(registry.binding_name("run_bash"), Some("run_bash"));
    }

    #[test]
    fn registry_renders_prompt_tool_catalog_from_manifests() {
        let registry = CapabilityRegistry::builtin();
        let rendered = registry.render_tool_catalog_markdown();

        assert!(rendered.contains("#### `memmgr`"));
        assert!(rendered.contains("#### `capmgr`"));
        assert!(rendered.contains("#### `run_bash`"));
        assert!(rendered.contains("#### `shell_job_status`"));
        assert!(!rendered.contains("#### `tool_job_status`"));
        assert!(rendered.contains("#### `self_tool`"));
        assert!(rendered.contains("interval_ms"));
        assert!(rendered.contains("exits with code 0"));
        assert!(rendered.contains("**Synopsis**"));
        assert!(rendered.contains("**Options**"));
        assert!(rendered.contains("Unified local memory manager"));
        assert!(rendered.contains("Use when the user asks about Timem itself"));
        assert!(rendered.contains("Conditional:"));
        assert!(rendered.contains("(type=durable, op=query) requires query"));
        assert!(rendered.contains("Conditional one of:"));
        assert!(rendered.contains("(type=context, op=shrink) requires one of delta_ids"));
        assert!(!rendered.contains("when `` is"));
        assert!(rendered.contains("**Result**"));
        assert!(!rendered.contains("```"));
        assert!(!rendered.contains("**Example action**"));
        assert!(!rendered.contains("read_back_command"));
        assert!(!rendered.contains("large_readback"));
        assert!(rendered.contains("`background`:"));
        assert!(rendered.contains("Foreground returns status and bounded output"));
        assert!(rendered.contains("Use loop_cmd with interval_ms"));
        assert!(rendered.contains("`op`:"));
        assert!(rendered.contains("`kind`:"));
        assert!(rendered.contains("`id`:"));
        assert!(rendered.contains("`inspect`"));
        assert!(rendered.contains("memory_conflict"));
        assert!(!rendered.contains("\"output\": {"));
        assert!(!rendered.contains("\"description\""));
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
        assert_eq!(registry.binding_name("tool_job_status"), None);
        assert_eq!(registry.binding_name("self_tool"), Some("self_tool"));
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
                "memmgr",
                &json_object([
                    ("type", Value::String("durable".to_string())),
                    ("op", Value::String("query".to_string())),
                ])
            )
            .unwrap_err()
            .contains("input.query_required_when_op=query,type=durable"));
        assert!(registry
            .validate_action_input(
                "memmgr",
                &json_object([
                    ("type", Value::String("scratch".to_string())),
                    ("op", Value::String("write".to_string())),
                    ("kind", Value::String("notes".to_string())),
                    ("content", Value::String("checkpoint".to_string())),
                ])
            )
            .unwrap_err()
            .contains("input.label_required_when_op=write,type=scratch"));
        assert!(registry
            .validate_action_input(
                "memmgr",
                &json_object([
                    ("type", Value::String("scratch".to_string())),
                    ("op", Value::String("write".to_string())),
                    ("kind", Value::String("notes".to_string())),
                    ("label", Value::String("checkpoint".to_string())),
                    ("content", Value::String("checkpoint".to_string())),
                ])
            )
            .is_ok());
        assert!(registry
            .validate_action_input(
                "memmgr",
                &json_object([
                    ("type", Value::String("scratch".to_string())),
                    ("op", Value::String("write".to_string())),
                    ("kind", Value::String("context_offload".to_string())),
                    ("label", Value::String("large context".to_string())),
                ])
            )
            .unwrap_err()
            .contains("input.any_required_when_delta_ids_when_kind=context_offload,type=scratch"));
        assert!(registry
            .validate_action_input(
                "memmgr",
                &json_object([
                    ("type", Value::String("scratch".to_string())),
                    ("op", Value::String("write".to_string())),
                    ("kind", Value::String("context_offload".to_string())),
                    ("label", Value::String("large context".to_string())),
                    (
                        "delta_ids",
                        Value::Array(vec![Value::String("pd_1".to_string())])
                    ),
                ])
            )
            .is_ok());
        assert!(registry
            .validate_action_input(
                "memmgr",
                &json_object([
                    ("type", Value::String("context".to_string())),
                    ("op", Value::String("shrink".to_string())),
                ])
            )
            .unwrap_err()
            .contains("input.any_required_when_delta_ids_when_op=shrink,type=context"));
        assert!(registry
            .validate_action_input(
                "shell_job_status",
                &json_object([("job_id", Value::String("job_1".to_string()))])
            )
            .is_ok());
        assert!(registry
            .validate_action_input(
                "capmgr",
                &json_object([
                    ("op", Value::String("job_cancel".to_string())),
                    ("job_id", Value::String("tool_job_1".to_string())),
                ])
            )
            .is_ok());
        assert!(registry
            .validate_action_input(
                "shell_job_status",
                &json_object([
                    ("job_id", Value::String("job_1".to_string())),
                    ("op", Value::String("cancel".to_string())),
                ])
            )
            .is_ok());
        assert!(registry
            .validate_action_input(
                "capmgr",
                &json_object([
                    ("op", Value::String("job_status".to_string())),
                    ("job_id", Value::String("tool_job_1".to_string())),
                ])
            )
            .is_ok());
        assert!(registry
            .validate_action_input(
                "tool_job_status",
                &json_object([("job_id", Value::String("tool_job_1".to_string()))])
            )
            .unwrap_err()
            .contains("unsupported_action:tool_job_status"));
        assert!(registry
            .validate_action_input("run_bash", &json_object([]))
            .unwrap_err()
            .contains("input.any_required:cmd|loop_cmd"));
        assert!(registry
            .validate_action_input("self_tool", &json_object([]))
            .unwrap_err()
            .contains("input.type_required"));
        assert!(registry
            .validate_action_input(
                "self_tool",
                &json_object([
                    ("type", Value::String("env".to_string())),
                    ("op", Value::String("write".to_string())),
                    ("key", Value::String("TIMEM_TEST_FLAG".to_string())),
                ])
            )
            .unwrap_err()
            .contains("input.value_required_when_op=write"));
        assert!(registry
            .validate_action_input(
                "self_tool",
                &json_object([
                    ("type", Value::String("mem_path".to_string())),
                    ("op", Value::String("read".to_string())),
                ])
            )
            .is_ok());
        assert!(registry
            .validate_action_input(
                "run_bash",
                &json_object([("read_back_command", Value::String("pwd".to_string()))])
            )
            .unwrap_err()
            .contains("input.read_back_command_unsupported"));
        assert!(registry
            .validate_action_input(
                "run_bash",
                &json_object([
                    ("cmd", Value::String("pwd".to_string())),
                    (
                        "large_readback_opt_in",
                        Value::String("need full output".to_string())
                    ),
                ])
            )
            .unwrap_err()
            .contains("input.large_readback_opt_in_unsupported"));
        assert!(registry
            .validate_action_input(
                "run_bash",
                &json_object([
                    ("cmd", Value::String("test -s output.txt".to_string())),
                    ("timeout_ms", Value::Number(5000.into())),
                ])
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

        let op_enum = capmgr
            .get("input_schema")
            .and_then(Value::as_object)
            .and_then(|schema| schema.get("properties"))
            .and_then(Value::as_object)
            .and_then(|properties| properties.get("op"))
            .and_then(Value::as_object)
            .and_then(|op| op.get("enum"))
            .and_then(Value::as_array)
            .expect("capmgr op enum");
        assert!(op_enum.contains(&Value::String("list".to_string())));
        assert!(op_enum.contains(&Value::String("load".to_string())));
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
                    ("cmd", Value::String("pwd".to_string())),
                    ("mode", Value::String("daemon".to_string())),
                ])
            )
            .unwrap_err()
            .contains("input.mode_unsupported:daemon"));
    }

    #[test]
    fn registry_enriches_static_prompt_tool_catalog() {
        let registry = CapabilityRegistry::builtin();
        let enriched = registry
            .enrich_static_prompt("## Tools\n{{TOOL_CATALOG}}\n## Skills\n{{SKILL_HEADERS}}");

        assert!(enriched.contains("#### `memmgr`"));
        assert!(!enriched.contains("\"release_quality_gate\""));
        assert!(enriched.contains("#### `run_bash`"));
        assert!(enriched.contains("No optional skills are currently loaded."));
        assert!(!enriched.contains("{{TOOL_CATALOG}}"));
        assert!(!enriched.contains("{{SKILL_HEADERS}}"));
    }

    #[test]
    fn run_bash_idl_uses_cmd_loop_cmd_without_removed_expect_fields() {
        let registry = CapabilityRegistry::builtin();
        let catalog = registry.tool_catalog_value();
        let run_bash = catalog
            .get("run_bash")
            .and_then(Value::as_object)
            .expect("run_bash catalog entry");
        let input_schema = run_bash
            .get("input_schema")
            .and_then(Value::as_object)
            .expect("run_bash input schema");
        let input_properties = input_schema
            .get("properties")
            .and_then(Value::as_object)
            .expect("run_bash input schema properties");

        assert!(input_properties.contains_key("cmd"));
        assert!(input_properties.contains_key("loop_cmd"));
        assert!(!input_properties.contains_key("command"));
        assert!(!input_properties.contains_key("read_back_command"));
        assert!(!input_properties.contains_key("large_readback_opt_in"));
        assert!(!input_properties.contains_key("expect"));
        assert!(!input_properties.contains_key("expect_timeout_ms"));

        let required_any = input_schema
            .get("required_any")
            .and_then(Value::as_array)
            .expect("run_bash required_any");
        assert!(required_any.iter().any(|group| {
            group
                .as_array()
                .map(|fields| fields.iter().any(|field| field == "cmd"))
                .unwrap_or(false)
        }));

        let prompt = registry.render_tool_catalog_markdown();
        assert!(prompt.contains("run_bash cmd=<shell_command>"));
        assert!(prompt.contains("run_bash loop_cmd=<check_command>"));
        assert!(!prompt.contains("`expect`:"));
        assert!(!prompt.contains("expect_timeout_ms"));
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
        assert!(loaded_tool.contains("manual:"));
        assert!(loaded_tool.contains("#### `run_bash`"));
        assert!(loaded_tool.contains("**Options**"));
        assert!(loaded_tool.contains("run_bash cmd=<shell_command>"));
        assert!(!loaded_tool.contains("read_back_command"));
        assert!(!loaded_tool.contains("large_readback"));
        assert!(!loaded_tool.contains("expect_timeout_ms"));
        assert!(!loaded_tool.contains("**Example action**"));
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
summary: Echo a bounded local string through Bash.
description: |
  Use this runtime overlay tool only when a bounded echo command is enough.
input_properties:
  command: string
required:
  - command
example_json: |
  {
    "action": "local_echo",
    "intent": "Echo a short string.",
    "args": {
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
            .render_tool_catalog_markdown()
            .contains("#### `local_echo`"));
        assert!(registry
            .load_text("skill", "log_review")
            .contains("# Runtime Log Review"));
        assert!(registry
            .render_skill_headers_markdown()
            .contains("Runtime-loaded log review checklist"));
    }

    #[test]
    fn no_local_command_profile_filters_overlay_command_tools() {
        let dir = temp_capability_dir("no_local_command_overlay");
        let tools_dir = dir.join("tools");
        fs::create_dir_all(&tools_dir).unwrap();
        fs::write(
            tools_dir.join("local_echo.yaml"),
            r#"kind: tool
id: local_echo
binding_type: command
binding_name: echo.sh
summary: Echo through a local command.
description: |
  Uses a host command process.
input_properties:
  text: string
required:
  - text
example_json: |
  {
    "action": "local_echo",
    "intent": "Echo a short string.",
    "args": {
      "text": "hello"
    }
  }
"#,
        )
        .unwrap();
        fs::write(
            tools_dir.join("local_echo_builtin.yaml"),
            r#"kind: tool
id: local_echo_builtin
binding_type: builtin
binding_name: run_bash
requires_host: local_command_execution
summary: Echo through the built-in local command executor.
description: |
  Uses the built-in local command executor through an overlay alias.
input_properties:
  command: string
required:
  - command
example_json: |
  {
    "action": "local_echo_builtin",
    "intent": "Echo a short string.",
    "args": {
      "command": "printf hello"
    }
  }
"#,
        )
        .unwrap();
        fs::write(dir.join("echo.sh"), "#!/bin/sh\nprintf '%s\\n' \"$1\"\n").unwrap();

        let registry = CapabilityRegistry::builtin_with_overlay_dir_for_host(
            &dir,
            CapabilityHostProfile::without_local_command_execution(),
        )
        .unwrap();

        assert!(!registry.contains_tool("local_echo"));
        assert!(!registry.contains_tool("local_echo_builtin"));
        assert!(!registry.contains_tool("run_bash"));
        assert!(!registry.contains_tool("shell_job_status"));
        assert!(!registry
            .render_tool_catalog_markdown()
            .contains("local_echo"));
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
summary: This tool has no executor.
description: |
  Should not load.
input_properties:
  query: string
example_json: |
  {
    "action": "ghost",
    "intent": "Should not execute.",
    "args": {
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
