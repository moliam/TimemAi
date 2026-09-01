use crate::{
    append_audit_event, interpret_model_http_response, model_request_audit_event,
    model_response_audit_event, prepare_model_http_request, prepare_model_interaction_http_request,
    without_openai_compatible_cache_control, ApiProtocol, LlmResponse, ModelClient,
    ModelHttpResponseInterpretation, ModelInteractionRequest, ModelServiceConfig,
    OpenAiCompatibleCacheMode, PreparedModelHttpRequest,
};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, LOCATION};
use reqwest::{Certificate, StatusCode, Url};
use std::cell::RefCell;
use std::future::Future;
use std::io::{self, Write};
use std::path::Path;
use std::time::{Duration, Instant};
use tokio::runtime::{Builder, Runtime};
use tokio::time::sleep;

const CANCEL_POLL_INTERVAL: Duration = Duration::from_millis(10);
const MAX_MODEL_REQUEST_BYTES: usize = 32 * 1024 * 1024;
const MAX_MODEL_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
const MAX_MODEL_REDIRECTS: usize = 10;
const MAX_PRIVATE_CA_PEM_BYTES: usize = 256 * 1024;

thread_local! {
    static THREAD_HTTP_MODEL_CLIENT: RefCell<HttpModelClient> =
        RefCell::new(HttpModelClient::default());
}

#[derive(Default)]
pub struct HttpModelClient {
    transport: Option<NativeHttpTransport>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ClientTransportKey {
    private_ca_pem: Option<String>,
}

struct NativeHttpTransport {
    runtime: Runtime,
    client: Option<(ClientTransportKey, reqwest::Client)>,
}

#[derive(Debug)]
struct NativeHttpResponse {
    status: u16,
    body: String,
    request_id: Option<String>,
    ttfb: Duration,
    elapsed: Duration,
    response_bytes: usize,
    redirect_count: usize,
}

impl NativeHttpTransport {
    fn new() -> Result<Self, String> {
        let runtime = Builder::new_current_thread()
            .enable_io()
            .enable_time()
            .build()
            .map_err(|error| {
                format!("model_network_error: runtime initialization failed: {error}")
            })?;
        Ok(Self {
            runtime,
            client: None,
        })
    }

    fn client_for(&mut self, config: &ModelServiceConfig) -> Result<reqwest::Client, String> {
        let key = ClientTransportKey {
            private_ca_pem: config.http_transport.private_ca_pem.clone(),
        };
        if let Some((current_key, client)) = &self.client {
            if current_key == &key {
                return Ok(client.clone());
            }
        }

        let mut builder = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none());
        if let Some(pem) = &key.private_ca_pem {
            let certificates = parse_model_private_ca_pem(pem)?;
            for certificate in certificates {
                builder = builder.add_root_certificate(certificate);
            }
        }
        let client = builder.build().map_err(|error| {
            format!("model_network_error: client initialization failed: {error}")
        })?;
        self.client = Some((key, client.clone()));
        Ok(client)
    }

    fn execute(
        &mut self,
        config: &ModelServiceConfig,
        request: &PreparedModelHttpRequest,
        inactivity_timeout: Duration,
        should_cancel: &mut dyn FnMut() -> bool,
    ) -> Result<NativeHttpResponse, String> {
        let client = self.client_for(config)?;
        let headers = request_headers(request)?;
        let body = bounded_request_body(&request.model_request.body)?;
        let endpoint = Url::parse(&request.endpoint)
            .map_err(|error| format!("model_request_url_error: {error}"))?;
        self.runtime.block_on(execute_with_redirects(
            &client,
            endpoint,
            headers,
            body,
            config.http_transport.allow_cross_origin_redirects,
            inactivity_timeout,
            should_cancel,
        ))
    }
}

async fn execute_with_redirects(
    client: &reqwest::Client,
    mut url: Url,
    original_headers: HeaderMap,
    body: Vec<u8>,
    allow_cross_origin_redirects: bool,
    inactivity_timeout: Duration,
    should_cancel: &mut dyn FnMut() -> bool,
) -> Result<NativeHttpResponse, String> {
    let started = Instant::now();
    let mut redirect_count = 0;
    let mut include_configured_headers = true;

    loop {
        let headers = if include_configured_headers {
            original_headers.clone()
        } else {
            let mut headers = HeaderMap::new();
            headers.insert(
                reqwest::header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            );
            headers
        };
        let pending = client
            .post(url.clone())
            .headers(headers)
            .body(body.clone())
            .send();
        let mut response = wait_for_progress(
            pending,
            inactivity_timeout,
            "response_headers",
            should_cancel,
        )
        .await?
        .map_err(|error| map_reqwest_error(error, "response_headers"))?;
        let ttfb = started.elapsed();

        if is_redirect(response.status()) {
            if redirect_count >= MAX_MODEL_REDIRECTS {
                return Err(format!(
                    "model_redirect_error: exceeded {MAX_MODEL_REDIRECTS} redirects"
                ));
            }
            let location = response
                .headers()
                .get(LOCATION)
                .ok_or_else(|| {
                    "model_redirect_error: redirect missing Location header".to_string()
                })?
                .to_str()
                .map_err(|_| {
                    "model_redirect_error: Location header is not valid text".to_string()
                })?;
            let next = url
                .join(location)
                .map_err(|error| format!("model_redirect_error: invalid Location: {error}"))?;
            let crosses_origin = !same_origin(&url, &next);
            if crosses_origin && !allow_cross_origin_redirects {
                return Err(format!(
                    "model_redirect_blocked: cross-origin redirect from {} to {}",
                    redacted_origin(&url),
                    redacted_origin(&next)
                ));
            }
            if crosses_origin {
                include_configured_headers = false;
            }
            url = next;
            redirect_count += 1;
            continue;
        }

        let status = response.status().as_u16();
        let request_id = response_request_id(response.headers());
        if response
            .content_length()
            .is_some_and(|length| length > MAX_MODEL_RESPONSE_BYTES as u64)
        {
            return Err(model_response_too_large());
        }
        let mut response_body = Vec::new();
        loop {
            let chunk = wait_for_progress(
                response.chunk(),
                inactivity_timeout,
                "response_body",
                should_cancel,
            )
            .await?
            .map_err(|error| map_reqwest_error(error, "response_body"))?;
            match chunk {
                Some(bytes) => {
                    if response_body.len().saturating_add(bytes.len()) > MAX_MODEL_RESPONSE_BYTES {
                        return Err(model_response_too_large());
                    }
                    response_body.extend_from_slice(&bytes);
                }
                None => break,
            }
        }
        let response_bytes = response_body.len();
        return Ok(NativeHttpResponse {
            status,
            body: String::from_utf8_lossy(&response_body).into_owned(),
            request_id,
            ttfb,
            elapsed: started.elapsed(),
            response_bytes,
            redirect_count,
        });
    }
}

async fn wait_for_progress<F, T>(
    future: F,
    inactivity_timeout: Duration,
    timeout_stage: &str,
    should_cancel: &mut dyn FnMut() -> bool,
) -> Result<T, String>
where
    F: Future<Output = T>,
{
    let mut future = std::pin::pin!(future);
    let timeout = sleep(inactivity_timeout);
    let mut timeout = std::pin::pin!(timeout);

    loop {
        if should_cancel() {
            return Err("cancelled_by_user".to_string());
        }
        tokio::select! {
            output = &mut future => return Ok(output),
            _ = &mut timeout => {
                return Err(format!(
                    "model_timeout: stage={timeout_stage} no progress for {} seconds",
                    inactivity_timeout.as_secs()
                ));
            }
            _ = sleep(CANCEL_POLL_INTERVAL) => {}
        }
    }
}

fn request_headers(request: &PreparedModelHttpRequest) -> Result<HeaderMap, String> {
    let mut headers = HeaderMap::new();
    for (name, value) in &request.headers {
        let name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|error| format!("invalid_model_http_header_name: {error}"))?;
        let value = HeaderValue::from_str(value)
            .map_err(|error| format!("invalid_model_http_header_value: {error}"))?;
        headers.append(name, value);
    }
    Ok(headers)
}

fn is_redirect(status: StatusCode) -> bool {
    matches!(status.as_u16(), 301 | 302 | 303 | 307 | 308)
}

fn same_origin(left: &Url, right: &Url) -> bool {
    left.scheme() == right.scheme()
        && left.host_str() == right.host_str()
        && left.port_or_known_default() == right.port_or_known_default()
}

fn redacted_origin(url: &Url) -> String {
    let host = url.host_str().unwrap_or("<unknown>");
    match url.port() {
        Some(port) => format!("{}://{host}:{port}", url.scheme()),
        None => format!("{}://{host}", url.scheme()),
    }
}

fn response_request_id(headers: &HeaderMap) -> Option<String> {
    [
        "x-request-id",
        "request-id",
        "x-amzn-requestid",
        "x-amz-request-id",
        "cf-ray",
    ]
    .iter()
    .find_map(|name| {
        let value = headers.get(*name)?.to_str().ok()?.trim();
        (!value.is_empty()
            && value.len() <= 256
            && value.chars().all(|character| !character.is_control()))
        .then(|| value.to_string())
    })
}

struct BoundedBodyWriter {
    bytes: Vec<u8>,
}

impl BoundedBodyWriter {
    fn new() -> Self {
        Self {
            bytes: Vec::with_capacity(MAX_MODEL_REQUEST_BYTES.min(64 * 1024)),
        }
    }
}

impl Write for BoundedBodyWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        if self.bytes.len().saturating_add(buffer.len()) > MAX_MODEL_REQUEST_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::FileTooLarge,
                "model request body limit exceeded",
            ));
        }
        self.bytes.extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn bounded_request_body(body: &serde_json::Value) -> Result<Vec<u8>, String> {
    let mut writer = BoundedBodyWriter::new();
    serde_json::to_writer(&mut writer, body).map_err(|error| {
        if error.io_error_kind() == Some(io::ErrorKind::FileTooLarge) {
            model_request_too_large()
        } else {
            format!("model_request_serialization_failed: {error}")
        }
    })?;
    Ok(writer.bytes)
}

pub fn validate_model_private_ca_pem(pem: &str) -> Result<(), String> {
    parse_model_private_ca_pem(pem).map(|_| ())
}

fn parse_model_private_ca_pem(pem: &str) -> Result<Vec<Certificate>, String> {
    if pem.len() > MAX_PRIVATE_CA_PEM_BYTES {
        return Err(format!(
            "model_tls_error: private CA PEM exceeds {MAX_PRIVATE_CA_PEM_BYTES} bytes"
        ));
    }
    let certificates = Certificate::from_pem_bundle(pem.as_bytes())
        .map_err(|error| format!("model_tls_error: invalid private CA PEM: {error}"))?;
    if certificates.is_empty() {
        return Err("model_tls_error: private CA PEM contains no certificates".to_string());
    }
    Ok(certificates)
}

fn model_request_too_large() -> String {
    format!("model_request_too_large: request body exceeds {MAX_MODEL_REQUEST_BYTES} bytes")
}

fn model_response_too_large() -> String {
    format!("model_response_too_large: response exceeds {MAX_MODEL_RESPONSE_BYTES} bytes")
}

fn map_reqwest_error(error: reqwest::Error, stage: &str) -> String {
    let detail = error.to_string();
    let chain = error_chain_text(&error);
    let lower = chain.to_ascii_lowercase();
    if error.is_timeout() {
        format!("model_timeout: stage={stage} {detail}")
    } else if lower.contains("proxy error")
        || lower.contains("proxy connect")
        || lower.contains("proxy tunnel")
    {
        format!("model_proxy_error: stage={stage} {detail}")
    } else if lower.contains("dns")
        || lower.contains("failed to lookup address")
        || lower.contains("name or service not known")
        || lower.contains("nodename nor servname provided")
        || lower.contains("no such host")
    {
        format!("model_dns_error: stage={stage} {detail}")
    } else if lower.contains("certificate")
        || lower.contains("tls")
        || lower.contains("ssl")
        || lower.contains("invalid peer certificate")
    {
        format!("model_tls_error: stage={stage} {detail}")
    } else if error.is_connect() {
        format!("model_connect_error: stage={stage} {detail}")
    } else if error.is_body() || error.is_decode() {
        format!("model_body_error: stage={stage} {detail}")
    } else if error.is_request() {
        format!("model_request_error: stage={stage} {detail}")
    } else {
        format!("model_network_error: stage={stage} {detail}")
    }
}

fn error_chain_text(error: &(dyn std::error::Error + 'static)) -> String {
    let mut parts = Vec::new();
    let mut current = Some(error);
    while let Some(item) = current {
        parts.push(item.to_string());
        current = item.source();
    }
    parts.join(": ")
}

impl HttpModelClient {
    fn transport(&mut self) -> Result<&mut NativeHttpTransport, String> {
        if self.transport.is_none() {
            self.transport = Some(NativeHttpTransport::new()?);
        }
        self.transport
            .as_mut()
            .ok_or_else(|| "model_network_error: transport initialization failed".to_string())
    }

    fn execute_prepared_request_with_cache_fallback(
        &mut self,
        config: &ModelServiceConfig,
        http_request: PreparedModelHttpRequest,
        audit_file: &Path,
        should_cancel: &mut dyn FnMut() -> bool,
    ) -> Result<LlmResponse, String> {
        let first =
            self.execute_model_http_request(config, &http_request, audit_file, should_cancel)?;

        if should_retry_without_openai_cache_control(config, &http_request, &first) {
            let fallback_request = without_openai_compatible_cache_control(&http_request);
            return self
                .execute_model_http_request(config, &fallback_request, audit_file, should_cancel)?
                .result;
        }

        first.result
    }

    fn execute_model_http_request(
        &mut self,
        config: &ModelServiceConfig,
        http_request: &PreparedModelHttpRequest,
        audit_file: &Path,
        should_cancel: &mut dyn FnMut() -> bool,
    ) -> Result<ModelHttpResponseInterpretation, String> {
        let _ = append_audit_event(
            audit_file,
            &model_request_audit_event(config, &http_request.model_request),
        );
        let timeout = Duration::from_secs(config.timeout_secs);
        let response = self
            .transport()?
            .execute(config, http_request, timeout, should_cancel)?;
        let interpreted =
            interpret_model_http_response(config, response.status, &response.body, "");
        let mut response_audit =
            model_response_audit_event(interpreted.status, &interpreted.raw_json);
        if let Some(object) = response_audit.as_object_mut() {
            object.insert(
                "transport".to_string(),
                serde_json::json!({
                    "request_id": response.request_id,
                    "ttfb_ms": response.ttfb.as_millis(),
                    "elapsed_ms": response.elapsed.as_millis(),
                    "response_bytes": response.response_bytes,
                    "redirect_count": response.redirect_count,
                }),
            );
        }
        let _ = append_audit_event(audit_file, &response_audit);
        Ok(interpreted)
    }
}

impl ModelClient for HttpModelClient {
    fn call_model(
        &mut self,
        config: &ModelServiceConfig,
        prompt: &str,
        audit_file: &Path,
        should_cancel: &mut dyn FnMut() -> bool,
    ) -> Result<LlmResponse, String> {
        let http_request = prepare_model_http_request(config, prompt);
        self.execute_prepared_request_with_cache_fallback(
            config,
            http_request,
            audit_file,
            should_cancel,
        )
    }

    fn call_model_interaction(
        &mut self,
        config: &ModelServiceConfig,
        request: &ModelInteractionRequest,
        audit_file: &Path,
        should_cancel: &mut dyn FnMut() -> bool,
    ) -> Result<LlmResponse, String> {
        let http_request = prepare_model_interaction_http_request(config, request);
        self.execute_prepared_request_with_cache_fallback(
            config,
            http_request,
            audit_file,
            should_cancel,
        )
    }
}

pub fn call_model(
    config: &ModelServiceConfig,
    prompt: &str,
    audit_file: &Path,
) -> Result<LlmResponse, String> {
    call_model_with_cancel(config, prompt, audit_file, &mut || false)
}

pub fn call_model_with_cancel(
    config: &ModelServiceConfig,
    prompt: &str,
    audit_file: &Path,
    should_cancel: &mut dyn FnMut() -> bool,
) -> Result<LlmResponse, String> {
    THREAD_HTTP_MODEL_CLIENT.with(|client| {
        client
            .try_borrow_mut()
            .map_err(|_| "model_network_error: reentrant model HTTP call".to_string())?
            .call_model(config, prompt, audit_file, should_cancel)
    })
}

fn should_retry_without_openai_cache_control(
    config: &ModelServiceConfig,
    request: &PreparedModelHttpRequest,
    response: &ModelHttpResponseInterpretation,
) -> bool {
    if config.api_protocol != ApiProtocol::OpenAiCompatible
        || config.openai_compatible.cache_mode != OpenAiCompatibleCacheMode::Ephemeral
        || request.model_request.cache_fallback
        || request.model_request.cache_mark_count == 0
        || !(400..500).contains(&response.status)
    {
        return false;
    }

    let error = response.raw_json.to_string().to_ascii_lowercase();
    let names_cache_control = error.contains("cache_control") || error.contains("cache control");
    let rejects_schema = [
        "unknown field",
        "unknown parameter",
        "unknown property",
        "unrecognized field",
        "unrecognized parameter",
        "unsupported field",
        "unsupported parameter",
        "not supported",
        "extra field",
        "extra input",
        "additional properties",
        "not permitted",
        "not allowed",
        "unexpected field",
        "unexpected parameter",
        "unexpected property",
    ]
    .iter()
    .any(|indicator| error.contains(indicator));

    names_cache_control && rejects_schema
}

#[cfg(test)]
#[path = "../tests/unit/model_transport_tests.rs"]
mod tests;
