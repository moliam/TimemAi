use crate::{
    append_audit_event, interpret_model_http_response, model_request_audit_event,
    model_response_audit_event, prepare_model_http_request, prepare_model_interaction_http_request,
    without_openai_compatible_cache_control, ApiProtocol, LlmResponse, ModelClient,
    ModelHttpResponseInterpretation, ModelInteractionRequest, ModelServiceConfig,
    OpenAiCompatibleCacheMode, PreparedModelHttpRequest,
};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use std::cell::RefCell;
use std::future::Future;
use std::path::Path;
use std::time::Duration;
use tokio::runtime::{Builder, Runtime};
use tokio::time::sleep;

const CANCEL_POLL_INTERVAL: Duration = Duration::from_millis(10);
const MAX_MODEL_RESPONSE_BYTES: usize = 16 * 1024 * 1024;

thread_local! {
    static THREAD_HTTP_MODEL_CLIENT: RefCell<HttpModelClient> =
        RefCell::new(HttpModelClient::default());
}

#[derive(Default)]
pub struct HttpModelClient {
    transport: Option<NativeHttpTransport>,
}

struct NativeHttpTransport {
    runtime: Runtime,
    client: reqwest::Client,
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
        let client = reqwest::Client::builder().build().map_err(|error| {
            format!("model_network_error: client initialization failed: {error}")
        })?;
        Ok(Self { runtime, client })
    }

    fn execute(
        &mut self,
        request: &PreparedModelHttpRequest,
        inactivity_timeout: Duration,
        should_cancel: &mut dyn FnMut() -> bool,
    ) -> Result<(u16, String), String> {
        let headers = request_headers(request)?;
        let body = serde_json::to_vec(&request.model_request.body)
            .map_err(|error| format!("model_request_serialization_failed: {error}"))?;
        let pending = self
            .client
            .post(&request.endpoint)
            .headers(headers)
            .body(body)
            .send();

        self.runtime.block_on(async move {
            let mut response = wait_for_progress(pending, inactivity_timeout, should_cancel)
                .await?
                .map_err(map_reqwest_error)?;
            let status = response.status().as_u16();
            if response
                .content_length()
                .is_some_and(|length| length > MAX_MODEL_RESPONSE_BYTES as u64)
            {
                return Err(model_response_too_large());
            }
            let mut response_body = Vec::new();

            loop {
                let chunk = wait_for_progress(response.chunk(), inactivity_timeout, should_cancel)
                    .await?
                    .map_err(map_reqwest_error)?;
                match chunk {
                    Some(bytes) => {
                        if response_body.len().saturating_add(bytes.len())
                            > MAX_MODEL_RESPONSE_BYTES
                        {
                            return Err(model_response_too_large());
                        }
                        response_body.extend_from_slice(&bytes);
                    }
                    None => break,
                }
            }

            Ok((status, String::from_utf8_lossy(&response_body).into_owned()))
        })
    }
}

async fn wait_for_progress<F, T>(
    future: F,
    inactivity_timeout: Duration,
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
                    "model_timeout: no response progress for {} seconds",
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

fn model_response_too_large() -> String {
    format!("model_response_too_large: response exceeds {MAX_MODEL_RESPONSE_BYTES} bytes")
}

fn map_reqwest_error(error: reqwest::Error) -> String {
    if error.is_timeout() {
        format!("model_timeout: {error}")
    } else {
        format!("model_network_error: {error}")
    }
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
        let (status, raw_text) = self
            .transport()?
            .execute(http_request, timeout, should_cancel)?;
        let interpreted = interpret_model_http_response(config, status, &raw_text, "");
        let _ = append_audit_event(
            audit_file,
            &model_response_audit_event(interpreted.status, &interpreted.raw_json),
        );
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
