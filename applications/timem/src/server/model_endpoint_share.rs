//! Versioned endpoint sharing adapter. Reuses the Host's Core-backed validation
//! and owner-protected endpoint store; never publishes exported plaintext.
use super::*;
use base64::{engine::general_purpose::STANDARD, Engine as _};

const MAX_SHARE_BYTES: usize = 256 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Share {
    format: String,
    version: u8,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    basic: Option<Basic>,
    #[serde(skip_serializing_if = "Option::is_none")]
    advanced: Option<Advanced>,
    #[serde(skip_serializing_if = "Option::is_none")]
    personal: Option<Personal>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Basic {
    catalog_id: Option<String>,
    model: String,
    api_protocol: String,
    response_protocol: String,
    base_url: String,
    max_llm_input_tokens: u32,
    max_llm_output_tokens: u32,
    stream: bool,
    requirements: agent_core::model_requirements::EndpointRequirements,
    reasoning_effort: Option<String>,
    #[serde(default = "default_true")]
    function_calling: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Advanced {
    request_fields: BTreeMap<String, Value>,
    allow_cross_origin_redirects: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Personal {
    api_key: String,
    http_headers: BTreeMap<String, String>,
    private_ca_pem: String,
}

pub(super) fn export(
    endpoint: &ModelEndpointConfig,
    basic: bool,
    advanced: bool,
    personal: bool,
) -> Result<String, String> {
    if !basic && !advanced && !personal {
        return Err("model_endpoint_share_selection_empty".into());
    }
    let share = Share {
        format: "timem.endpoint".into(),
        version: 1,
        name: endpoint.name.clone(),
        basic: basic.then(|| Basic {
            catalog_id: endpoint.catalog_id.clone(),
            model: endpoint.model.clone(),
            api_protocol: endpoint.api_protocol.clone(),
            response_protocol: endpoint.response_protocol.clone(),
            base_url: endpoint.base_url.clone(),
            max_llm_input_tokens: endpoint.max_llm_input_tokens,
            max_llm_output_tokens: endpoint.max_llm_output_tokens,
            stream: endpoint.stream,
            requirements: endpoint.requirements.clone(),
            reasoning_effort: endpoint.reasoning_effort.clone(),
            function_calling: endpoint.function_calling,
        }),
        advanced: advanced.then(|| Advanced {
            request_fields: endpoint.request_fields.clone(),
            allow_cross_origin_redirects: endpoint.allow_cross_origin_redirects,
        }),
        personal: personal.then(|| Personal {
            api_key: endpoint.api_key.clone(),
            http_headers: endpoint.http_headers.clone(),
            private_ca_pem: endpoint.private_ca_pem.clone(),
        }),
    };
    let raw = serde_json::to_vec(&share).map_err(|_| "model_endpoint_share_encode_failed")?;
    let data = STANDARD.encode(raw);
    if data.len() > MAX_SHARE_BYTES {
        return Err("model_endpoint_share_too_large".into());
    }
    Ok(data)
}

fn decode(data: &str) -> Result<ModelEndpointInput, String> {
    if data.len() > MAX_SHARE_BYTES {
        return Err("model_endpoint_share_too_large".into());
    }
    let compact: String = data
        .chars()
        .filter(|ch| !ch.is_ascii_whitespace())
        .collect();
    let raw = STANDARD
        .decode(compact)
        .map_err(|_| "model_endpoint_share_invalid_base64")?;
    // Never return serde error text: malformed values may contain credentials.
    let share: Share =
        serde_json::from_slice(&raw).map_err(|_| "model_endpoint_share_invalid_format")?;
    if share.format != "timem.endpoint" || share.version != 1 {
        return Err("model_endpoint_share_unsupported_version".into());
    }
    let basic = share.basic.ok_or("model_endpoint_share_basic_required")?;
    let advanced = share.advanced;
    let personal = share.personal;
    Ok(ModelEndpointInput {
        id: None,
        name: share.name,
        catalog_id: basic.catalog_id,
        model: basic.model,
        api_protocol: basic.api_protocol,
        response_protocol: basic.response_protocol,
        base_url: basic.base_url,
        max_llm_input_tokens: basic.max_llm_input_tokens,
        max_llm_output_tokens: basic.max_llm_output_tokens,
        stream: basic.stream,
        requirements: basic.requirements,
        reasoning_effort: basic.reasoning_effort,
        function_calling: basic.function_calling,
        request_fields: advanced
            .as_ref()
            .map(|v| v.request_fields.clone())
            .unwrap_or_default(),
        allow_cross_origin_redirects: advanced
            .as_ref()
            .is_some_and(|v| v.allow_cross_origin_redirects),
        api_key: personal.as_ref().map(|v| v.api_key.clone()),
        http_headers: personal
            .as_ref()
            .map(|v| v.http_headers.clone())
            .unwrap_or_default(),
        private_ca_pem: personal.map(|v| v.private_ca_pem),
    })
}

pub(super) fn import(state: &AppState, data: &str) -> Result<String, String> {
    let input = decode(data)?;
    // Reuse Core validation, but do not echo imported values in error details.
    let mut endpoint = normalize_model_endpoint_input(None, input)
        .map_err(|_| "model_endpoint_share_invalid_config")?;
    let mut mem = state.mem.lock().map_err(|_| "mem_state_poisoned")?;
    let base = endpoint.name.clone();
    let taken: BTreeSet<&str> = mem
        .model_endpoints
        .iter()
        .map(|v| v.name.as_str())
        .collect();
    // At most N+1 probes for N existing names. Collision resolution and commit
    // share one lock; imported IDs can never select an existing endpoint.
    for suffix in 0..=taken.len() {
        let name = if suffix == 0 {
            base.clone()
        } else {
            format!("{base}{suffix}")
        };
        if !taken.contains(name.as_str()) {
            endpoint.name = name;
            break;
        }
    }
    let name = endpoint.name.clone();
    let mut next = mem.model_endpoints.clone();
    next.push(endpoint);
    next.sort_by(|a, b| a.name.cmp(&b.name));
    save_model_endpoints(&mem.layout.memory_dir(), &next)?;
    mem.model_endpoints = next;
    Ok(name)
}
