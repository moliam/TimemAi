//! Host-side model endpoint import from external CLI configuration stores.
//!
//! The Host owns this parsing: the browser cannot read arbitrary local
//! directories, and parsed secrets never round-trip through the client. A scan
//! returns a redacted preview plus a bounded, server-held candidate buffer that
//! a follow-up apply command converts into shared model endpoints.

use serde::Serialize;
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

/// Upper bound for candidates retained from one scan. A larger configuration
/// store is truncated with an explicit issue instead of growing host memory.
pub(super) const MAX_MODEL_ENDPOINT_IMPORT_CANDIDATES: usize = 32;
/// Per-file read bound so a corrupt or oversized external file cannot force an
/// unbounded read through the command path.
const MAX_IMPORT_CONFIG_FILE_BYTES: u64 = 1024 * 1024;
const MAX_CODEX_PROFILE_OVERLAY_FILES: usize = 32;
const CLAUDE_DEFAULT_BASE_URL: &str = "https://api.anthropic.com";

#[derive(Debug, Clone)]
pub(super) struct ModelEndpointImportCandidate {
    pub id: String,
    pub source: &'static str,
    pub name: String,
    pub model: String,
    pub api_protocol: String,
    pub response_protocol: String,
    pub base_url: String,
    pub max_llm_input_tokens: u32,
    pub max_llm_output_tokens: u32,
    pub stream: bool,
    pub api_key: String,
    pub reasoning_effort: Option<String>,
    pub http_headers: BTreeMap<String, String>,
    pub request_fields: BTreeMap<String, Value>,
}

/// Redacted scan projection. Parsed keys stay in the Host-held candidate only.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(super) struct ModelEndpointImportCandidateReport {
    pub id: String,
    pub source: String,
    pub name: String,
    pub model: String,
    pub api_protocol: String,
    pub response_protocol: String,
    pub base_url: String,
    pub max_llm_input_tokens: u32,
    pub max_llm_output_tokens: u32,
    pub stream: bool,
    pub api_key_configured: bool,
    pub reasoning_effort: Option<String>,
}

impl From<&ModelEndpointImportCandidate> for ModelEndpointImportCandidateReport {
    fn from(candidate: &ModelEndpointImportCandidate) -> Self {
        Self {
            id: candidate.id.clone(),
            source: candidate.source.to_string(),
            name: candidate.name.clone(),
            model: candidate.model.clone(),
            api_protocol: candidate.api_protocol.clone(),
            response_protocol: candidate.response_protocol.clone(),
            base_url: candidate.base_url.clone(),
            max_llm_input_tokens: candidate.max_llm_input_tokens,
            max_llm_output_tokens: candidate.max_llm_output_tokens,
            stream: candidate.stream,
            api_key_configured: !candidate.api_key.is_empty(),
            reasoning_effort: candidate.reasoning_effort.clone(),
        }
    }
}

#[derive(Debug, Default)]
pub(super) struct ModelEndpointImportScan {
    pub candidates: Vec<ModelEndpointImportCandidate>,
    pub issues: Vec<String>,
}

struct CandidateBuilder {
    next_ordinal: u32,
}

impl CandidateBuilder {
    fn new() -> Self {
        Self { next_ordinal: 0 }
    }

    fn next_id(&mut self, source: &str) -> String {
        self.next_ordinal += 1;
        format!("{source}_import_{}", self.next_ordinal)
    }
}

/// Expands a user-entered directory path. `~` refers to the platform user home;
/// other users' homes are rejected because the Host cannot resolve them.
pub(super) fn expand_import_path(input: &str) -> Result<PathBuf, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("model_endpoint_import_directory_empty".to_string());
    }
    if trimmed == "~" {
        return agent_core::os::user_home_dir()
            .ok_or_else(|| "home_directory_unavailable".to_string());
    }
    if let Some(rest) = trimmed.strip_prefix("~/") {
        if rest.is_empty() {
            return Err("model_endpoint_import_directory_empty".to_string());
        }
        let home = agent_core::os::user_home_dir()
            .ok_or_else(|| "home_directory_unavailable".to_string())?;
        return Ok(home.join(rest));
    }
    if trimmed.starts_with('~') {
        return Err("model_endpoint_import_home_user_unsupported".to_string());
    }
    Ok(PathBuf::from(trimmed))
}

pub(super) fn scan_model_endpoint_imports(
    codex_dir: Option<&str>,
    claude_dir: Option<&str>,
) -> Result<ModelEndpointImportScan, String> {
    let codex_dir = codex_dir
        .map(expand_import_path)
        .transpose()?
        .filter(|path| !path.as_os_str().is_empty());
    let claude_dir = claude_dir
        .map(expand_import_path)
        .transpose()?
        .filter(|path| !path.as_os_str().is_empty());
    let mut scan = ModelEndpointImportScan::default();
    if let Some(dir) = codex_dir.as_deref() {
        scan_codex_directory(dir, &mut scan);
    }
    if let Some(dir) = claude_dir.as_deref() {
        scan_claude_directory(dir, &mut scan);
    }
    if scan.candidates.len() > MAX_MODEL_ENDPOINT_IMPORT_CANDIDATES {
        scan.issues.push(format!(
            "model_endpoint_import_candidate_limit:{MAX_MODEL_ENDPOINT_IMPORT_CANDIDATES}"
        ));
        scan.candidates
            .truncate(MAX_MODEL_ENDPOINT_IMPORT_CANDIDATES);
    }
    Ok(scan)
}

fn read_bounded(path: &Path, code: &str) -> Result<Vec<u8>, String> {
    let metadata = std::fs::metadata(path).map_err(|error| format!("{code}:{error}"))?;
    if metadata.len() > MAX_IMPORT_CONFIG_FILE_BYTES {
        return Err(format!("{code}:file_too_large"));
    }
    std::fs::read(path).map_err(|error| format!("{code}:{error}"))
}

fn text_value<'a>(value: &'a toml::Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(toml::Value::as_str).map(str::trim)
}

fn string_headers(value: &toml::Value, key: &str) -> BTreeMap<String, String> {
    value
        .get(key)
        .and_then(toml::Value::as_table)
        .map(|table| {
            table
                .iter()
                .filter_map(|(name, value)| {
                    value.as_str().map(|text| (name.clone(), text.to_string()))
                })
                .collect()
        })
        .unwrap_or_default()
}

struct CodexProvider {
    name: String,
    base_url: String,
    api_protocol: String,
    http_headers: BTreeMap<String, String>,
    api_key: String,
}

fn codex_auth_key(dir: &Path) -> Option<String> {
    let raw = std::fs::read_to_string(dir.join("auth.json")).ok()?;
    let value: Value = serde_json::from_str(&raw).ok()?;
    let key = value.get("OPENAI_API_KEY")?.as_str()?.trim();
    if key.is_empty() {
        None
    } else {
        Some(key.to_string())
    }
}

fn codex_builtin_provider(id: &str) -> Option<CodexProvider> {
    if id != "openai" {
        return None;
    }
    Some(CodexProvider {
        name: "OpenAI".to_string(),
        base_url: "https://api.openai.com/v1".to_string(),
        api_protocol: "openai-responses".to_string(),
        http_headers: BTreeMap::new(),
        api_key: String::new(),
    })
}

fn codex_provider(
    id: &str,
    providers: Option<&toml::map::Map<String, toml::Value>>,
    auth_key: Option<&str>,
    issues: &mut Vec<String>,
) -> Option<CodexProvider> {
    let Some(entry) = providers.and_then(|table| table.get(id)) else {
        let mut provider = codex_builtin_provider(id)?;
        if let Some(key) = auth_key {
            provider.api_key = key.to_string();
        }
        return Some(provider);
    };
    let name = text_value(entry, "name")
        .filter(|value| !value.is_empty())
        .unwrap_or(id);
    let Some(base_url) = text_value(entry, "base_url").filter(|value| !value.is_empty()) else {
        issues.push(format!("codex_provider_base_url_missing:{id}"));
        return None;
    };
    let wire_api = text_value(entry, "wire_api").unwrap_or("chat");
    let api_protocol = match wire_api {
        "chat" => "openai-compatible",
        "responses" => "openai-responses",
        other => {
            issues.push(format!("codex_provider_wire_api_unsupported:{id}:{other}"));
            return None;
        }
    };
    let env_key = text_value(entry, "env_key").filter(|value| !value.is_empty());
    let inline_token =
        text_value(entry, "experimental_bearer_token").filter(|value| !value.is_empty());
    let mut api_key = inline_token.map(str::to_string).unwrap_or_default();
    if api_key.is_empty() {
        api_key = env_key
            .and_then(|key| std::env::var(key).ok())
            .filter(|key| !key.trim().is_empty())
            .unwrap_or_default();
    }
    if api_key.is_empty() && (id == "openai" || env_key.is_some_and(|key| key == "OPENAI_API_KEY"))
    {
        if let Some(key) = auth_key {
            api_key = key.to_string();
        }
    }
    let mut http_headers = string_headers(entry, "http_headers");
    for (name, env_name) in env_headers(entry, "env_http_headers") {
        if let Ok(value) = std::env::var(&env_name) {
            if !value.trim().is_empty() {
                http_headers.insert(name, value);
            }
        }
    }
    Some(CodexProvider {
        name: name.to_string(),
        base_url: base_url.to_string(),
        api_protocol: api_protocol.to_string(),
        http_headers,
        api_key,
    })
}

fn env_headers(value: &toml::Value, key: &str) -> Vec<(String, String)> {
    value
        .get(key)
        .and_then(toml::Value::as_table)
        .map(|table| {
            table
                .iter()
                .map(|(name, value)| (name.clone(), value.as_str().unwrap_or_default().to_string()))
                .collect()
        })
        .unwrap_or_default()
}

#[derive(Default)]
struct CodexModelOptions {
    reasoning_effort: Option<String>,
    disable_response_storage: bool,
    model_verbosity: Option<String>,
}

fn codex_model_options(config: &toml::Value, profile: Option<&toml::Value>) -> CodexModelOptions {
    let mut options = CodexModelOptions {
        reasoning_effort: text_value(config, "model_reasoning_effort")
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        disable_response_storage: config
            .get("disable_response_storage")
            .and_then(toml::Value::as_bool)
            .unwrap_or(false),
        model_verbosity: text_value(config, "model_verbosity")
            .filter(|value| !value.is_empty())
            .map(str::to_string),
    };
    let Some(profile) = profile else {
        return options;
    };
    if let Some(effort) =
        text_value(profile, "model_reasoning_effort").filter(|value| !value.is_empty())
    {
        options.reasoning_effort = Some(effort.to_string());
    }
    if let Some(verbosity) =
        text_value(profile, "model_verbosity").filter(|value| !value.is_empty())
    {
        options.model_verbosity = Some(verbosity.to_string());
    }
    options
}

fn codex_request_fields(
    provider: &CodexProvider,
    options: &CodexModelOptions,
) -> BTreeMap<String, Value> {
    let mut fields = BTreeMap::new();
    if provider.api_protocol == "openai-responses" {
        if options.disable_response_storage {
            fields.insert("store".to_string(), Value::Bool(false));
        }
        if let Some(verbosity) = &options.model_verbosity {
            fields.insert("text".to_string(), json!({ "verbosity": verbosity }));
        }
    }
    fields
}

fn codex_candidate(
    builder: &mut CandidateBuilder,
    provider: &CodexProvider,
    model: &str,
    name: &str,
    options: &CodexModelOptions,
) -> ModelEndpointImportCandidate {
    ModelEndpointImportCandidate {
        id: builder.next_id("codex"),
        source: "codex",
        name: name.to_string(),
        model: model.to_string(),
        api_protocol: provider.api_protocol.clone(),
        response_protocol: "xml".to_string(),
        base_url: provider.base_url.clone(),
        max_llm_input_tokens: 100_000,
        max_llm_output_tokens: 10_000,
        stream: provider.api_protocol == "openai-compatible",
        api_key: provider.api_key.clone(),
        reasoning_effort: options.reasoning_effort.clone(),
        http_headers: provider.http_headers.clone(),
        request_fields: codex_request_fields(provider, options),
    }
}

fn scan_codex_directory(dir: &Path, scan: &mut ModelEndpointImportScan) {
    let config_path = dir.join("config.toml");
    let raw = match read_bounded(&config_path, "codex_config_read_failed") {
        Ok(raw) => raw,
        Err(error) => {
            scan.issues.push(error);
            return;
        }
    };
    let config: toml::Value = match toml::from_str(&String::from_utf8_lossy(&raw)) {
        Ok(config) => config,
        Err(error) => {
            scan.issues.push(format!("codex_config_invalid:{error}"));
            return;
        }
    };
    let providers = config
        .get("model_providers")
        .and_then(toml::Value::as_table);
    if providers.is_none() && config.get("model_provider").is_none() {
        scan.issues
            .push("codex_model_providers_missing".to_string());
        return;
    }
    let auth_key = codex_auth_key(dir);
    let default_model = text_value(&config, "model").filter(|value| !value.is_empty());
    let default_provider_id = text_value(&config, "model_provider")
        .filter(|value| !value.is_empty())
        .unwrap_or("openai");
    let mut builder = CandidateBuilder::new();
    let mut referenced_providers = BTreeSet::new();
    referenced_providers.insert(default_provider_id.to_string());

    if let Some(model) = default_model {
        if let Some(provider) = codex_provider(
            default_provider_id,
            providers,
            auth_key.as_deref(),
            &mut scan.issues,
        ) {
            let options = codex_model_options(&config, None);
            scan.candidates.push(codex_candidate(
                &mut builder,
                &provider,
                model,
                &provider.name,
                &options,
            ));
        }
    } else {
        scan.issues.push("codex_default_model_missing".to_string());
    }

    for (profile_id, profile) in config
        .get("profiles")
        .and_then(toml::Value::as_table)
        .into_iter()
        .flatten()
    {
        let provider_id = text_value(profile, "model_provider")
            .filter(|value| !value.is_empty())
            .unwrap_or(default_provider_id);
        referenced_providers.insert(provider_id.to_string());
        let model = text_value(profile, "model")
            .filter(|value| !value.is_empty())
            .or(default_model);
        let Some(model) = model else {
            scan.issues
                .push(format!("codex_profile_model_missing:{profile_id}"));
            continue;
        };
        let Some(provider) = codex_provider(
            provider_id,
            providers,
            auth_key.as_deref(),
            &mut scan.issues,
        ) else {
            continue;
        };
        if scan.candidates.iter().any(|candidate| {
            candidate.source == "codex"
                && candidate.name == provider.name
                && candidate.model == model
                && candidate.base_url == provider.base_url
                && candidate.api_protocol == provider.api_protocol
        }) {
            continue;
        }
        let options = codex_model_options(&config, Some(profile));
        scan.candidates.push(codex_candidate(
            &mut builder,
            &provider,
            model,
            &provider.name,
            &options,
        ));
    }

    let mut overlay_paths = Vec::new();
    let mut overlay_limit_exceeded = false;
    match std::fs::read_dir(dir) {
        Ok(entries) => {
            for entry in entries.flatten() {
                let Ok(file_type) = entry.file_type() else {
                    continue;
                };
                if !file_type.is_file() {
                    continue;
                }
                let Some(file_name) = entry.file_name().into_string().ok() else {
                    continue;
                };
                if file_name.ends_with(".config.toml") {
                    if overlay_paths.len() >= MAX_CODEX_PROFILE_OVERLAY_FILES {
                        overlay_limit_exceeded = true;
                    } else {
                        overlay_paths.push(entry.path());
                    }
                }
            }
        }
        Err(error) => scan
            .issues
            .push(format!("codex_profile_overlay_read_failed:{error}")),
    }
    overlay_paths.sort();
    if overlay_limit_exceeded {
        scan.issues.push(format!(
            "codex_profile_overlay_limit:{MAX_CODEX_PROFILE_OVERLAY_FILES}"
        ));
    }

    for overlay_path in overlay_paths {
        let overlay_name = overlay_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("<invalid>");
        let raw = match read_bounded(&overlay_path, "codex_profile_overlay_read_failed") {
            Ok(raw) => raw,
            Err(error) => {
                scan.issues.push(error);
                continue;
            }
        };
        let overlay: toml::Value = match toml::from_str(&String::from_utf8_lossy(&raw)) {
            Ok(value) => value,
            Err(error) => {
                scan.issues.push(format!(
                    "codex_profile_overlay_invalid:{overlay_name}:{error}"
                ));
                continue;
            }
        };
        let provider_id = text_value(&overlay, "model_provider")
            .filter(|value| !value.is_empty())
            .unwrap_or(default_provider_id);
        referenced_providers.insert(provider_id.to_string());
        let Some(model) = text_value(&overlay, "model").filter(|value| !value.is_empty()) else {
            scan.issues.push(format!(
                "codex_profile_overlay_model_missing:{overlay_name}"
            ));
            continue;
        };
        let mut provider_table = providers.cloned().unwrap_or_default();
        if let Some(overlay_providers) = overlay
            .get("model_providers")
            .and_then(toml::Value::as_table)
        {
            for (id, value) in overlay_providers {
                provider_table.insert(id.clone(), value.clone());
            }
        }
        let Some(provider) = codex_provider(
            provider_id,
            Some(&provider_table),
            auth_key.as_deref(),
            &mut scan.issues,
        ) else {
            continue;
        };
        if scan.candidates.iter().any(|candidate| {
            candidate.source == "codex"
                && candidate.name == provider.name
                && candidate.model == model
                && candidate.base_url == provider.base_url
                && candidate.api_protocol == provider.api_protocol
        }) {
            continue;
        }
        let options = codex_model_options(&overlay, None);
        scan.candidates.push(codex_candidate(
            &mut builder,
            &provider,
            model,
            &provider.name,
            &options,
        ));
    }

    // Providers without a model source are not endpoints yet. Report them
    // instead of borrowing the default model from an unrelated provider.
    if let Some(provider_table) = providers {
        for (provider_id, _) in provider_table {
            if referenced_providers.contains(provider_id) {
                continue;
            }
            scan.issues
                .push(format!("codex_provider_model_missing:{provider_id}"));
        }
    }
}

fn claude_env_string(env: &serde_json::Map<String, Value>, key: &str) -> Option<String> {
    env.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn merge_json_object(base: &mut Value, overlay: &Value) {
    let (Some(base), Some(overlay)) = (base.as_object_mut(), overlay.as_object()) else {
        return;
    };
    for (key, value) in overlay {
        if let (Some(existing), Value::Object(incoming)) = (base.get_mut(key), value) {
            if existing.is_object() {
                let incoming = Value::Object(incoming.clone());
                merge_json_object(existing, &incoming);
                continue;
            }
        }
        base.insert(key.clone(), value.clone());
    }
}

fn scan_claude_directory(dir: &Path, scan: &mut ModelEndpointImportScan) {
    let settings_path = dir.join("settings.json");
    let local_path = dir.join("settings.local.json");
    let settings = read_bounded(&settings_path, "claude_settings_read_failed")
        .ok()
        .map(|raw| serde_json::from_slice::<Value>(&raw));
    let local = read_bounded(&local_path, "claude_settings_read_failed")
        .ok()
        .map(|raw| serde_json::from_slice::<Value>(&raw));
    let settings = match (settings, local) {
        (Some(Ok(settings)), local) => {
            let mut merged = settings;
            if let Some(Ok(local)) = local {
                merge_json_object(&mut merged, &local);
            }
            merged
        }
        (None, Some(Ok(local))) => local,
        (Some(Err(error)), _) => {
            scan.issues.push(format!("claude_settings_invalid:{error}"));
            return;
        }
        (None, None) => {
            scan.issues.push("claude_settings_missing".to_string());
            return;
        }
        (None, Some(Err(error))) => {
            scan.issues.push(format!("claude_settings_invalid:{error}"));
            return;
        }
    };
    let empty_env = serde_json::Map::new();
    let env = settings
        .get("env")
        .and_then(Value::as_object)
        .unwrap_or(&empty_env);
    let model = claude_env_string(env, "ANTHROPIC_MODEL").or_else(|| {
        settings
            .get("model")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    });
    let Some(model) = model else {
        scan.issues.push("claude_model_missing".to_string());
        return;
    };
    let base_url = claude_env_string(env, "ANTHROPIC_BASE_URL")
        .unwrap_or_else(|| CLAUDE_DEFAULT_BASE_URL.to_string());
    let api_key = claude_env_string(env, "ANTHROPIC_AUTH_TOKEN")
        .or_else(|| claude_env_string(env, "ANTHROPIC_API_KEY"))
        .unwrap_or_default();
    // Claude Code exposes reasoning as an effort level, but the Anthropic
    // Messages API has no effort parameter; only an explicit thinking token
    // budget can be mapped without inventing a conversion.
    if settings
        .get("effortLevel")
        .and_then(Value::as_str)
        .is_some()
    {
        scan.issues
            .push("claude_effort_level_not_imported".to_string());
    }
    let mut request_fields = BTreeMap::new();
    if let Some(budget) = claude_env_string(env, "MAX_THINKING_TOKENS") {
        match budget.parse::<u64>() {
            Ok(budget) if budget >= 1024 => {
                request_fields.insert(
                    "thinking".to_string(),
                    json!({ "type": "enabled", "budget_tokens": budget }),
                );
            }
            _ => scan
                .issues
                .push("claude_thinking_tokens_invalid".to_string()),
        }
    }
    let mut http_headers = BTreeMap::new();
    if let Some(custom) = claude_env_string(env, "ANTHROPIC_CUSTOM_HEADERS") {
        for line in custom.lines() {
            let Some((name, value)) = line.split_once(':') else {
                continue;
            };
            let (name, value) = (name.trim(), value.trim());
            if !name.is_empty() && !value.is_empty() {
                http_headers.insert(name.to_string(), value.to_string());
            }
        }
    }
    let mut builder = CandidateBuilder::new();
    scan.candidates.push(ModelEndpointImportCandidate {
        id: builder.next_id("claude"),
        source: "claude",
        name: "Claude Code".to_string(),
        model,
        api_protocol: "anthropic".to_string(),
        response_protocol: "xml".to_string(),
        base_url,
        max_llm_input_tokens: 200_000,
        max_llm_output_tokens: 20_000,
        stream: false,
        api_key,
        reasoning_effort: None,
        http_headers,
        request_fields,
    });
}
