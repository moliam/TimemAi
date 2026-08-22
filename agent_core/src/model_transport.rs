use crate::{
    append_audit_event, interpret_model_http_response, model_request_audit_event,
    model_response_audit_event, prepare_model_http_request, prepare_model_interaction_http_request,
    without_openai_compatible_cache_control, ApiProtocol, LlmResponse, ModelClient,
    ModelHttpResponseInterpretation, ModelInteractionRequest, ModelServiceConfig,
    OpenAiCompatibleCacheMode, PreparedModelHttpRequest,
};
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, ExitStatus, Output, Stdio};
use std::thread;
use std::time::Duration;

pub struct HttpModelClient;

impl ModelClient for HttpModelClient {
    fn call_model(
        &mut self,
        config: &ModelServiceConfig,
        prompt: &str,
        audit_file: &Path,
        should_cancel: &mut dyn FnMut() -> bool,
    ) -> Result<LlmResponse, String> {
        call_model_with_cancel(config, prompt, audit_file, should_cancel)
    }

    fn call_model_interaction(
        &mut self,
        config: &ModelServiceConfig,
        request: &ModelInteractionRequest,
        audit_file: &Path,
        should_cancel: &mut dyn FnMut() -> bool,
    ) -> Result<LlmResponse, String> {
        let http_request = prepare_model_interaction_http_request(config, request);
        execute_prepared_request_with_cache_fallback(
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
    let http_request = prepare_model_http_request(config, prompt);
    execute_prepared_request_with_cache_fallback(config, http_request, audit_file, should_cancel)
}

fn execute_prepared_request_with_cache_fallback(
    config: &ModelServiceConfig,
    http_request: PreparedModelHttpRequest,
    audit_file: &Path,
    should_cancel: &mut dyn FnMut() -> bool,
) -> Result<LlmResponse, String> {
    let first = execute_model_http_request(config, &http_request, audit_file, should_cancel)?;

    if should_retry_without_openai_cache_control(config, &http_request, &first) {
        let fallback_request = without_openai_compatible_cache_control(&http_request);
        return execute_model_http_request(config, &fallback_request, audit_file, should_cancel)?
            .result;
    }

    first.result
}

fn execute_model_http_request(
    config: &ModelServiceConfig,
    http_request: &PreparedModelHttpRequest,
    audit_file: &Path,
    should_cancel: &mut dyn FnMut() -> bool,
) -> Result<ModelHttpResponseInterpretation, String> {
    let _ = append_audit_event(
        audit_file,
        &model_request_audit_event(config, &http_request.model_request),
    );
    let body =
        serde_json::to_string(&http_request.model_request.body).map_err(|e| e.to_string())?;
    let command = build_curl_command(config.timeout_secs);
    let curl_config = build_curl_config(http_request, &body);
    let output =
        run_command_with_input_and_cancel(command, curl_config.into_bytes(), should_cancel)?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !output.status.success() && stdout.trim().is_empty() {
        return Err(if stderr.is_empty() {
            "curl_failed".to_string()
        } else {
            stderr
        });
    }
    let (raw_text, status) = split_curl_body_status(&stdout)?;
    let interpreted = interpret_model_http_response(config, status, &raw_text, &stderr);
    let _ = append_audit_event(
        audit_file,
        &model_response_audit_event(interpreted.status, &interpreted.raw_json),
    );
    Ok(interpreted)
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

fn build_curl_command(timeout_secs: u64) -> Command {
    let mut command = Command::new("curl");
    command
        .arg("-sS")
        .arg("--max-time")
        .arg(timeout_secs.to_string())
        .arg("-w")
        .arg("\n%{http_code}")
        .arg("--config")
        .arg("-");
    command
}

fn build_curl_config(http_request: &crate::PreparedModelHttpRequest, body: &str) -> String {
    let mut config = String::new();
    config.push_str("request = \"POST\"\n");
    config.push_str("url = \"");
    config.push_str(&curl_config_escape(&http_request.endpoint));
    config.push_str("\"\n");
    for (key, value) in &http_request.headers {
        config.push_str("header = \"");
        config.push_str(&curl_config_escape(&format!("{key}: {value}")));
        config.push_str("\"\n");
    }
    config.push_str("data-binary = \"");
    config.push_str(&curl_config_escape(body));
    config.push_str("\"\n");
    config
}

fn curl_config_escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            _ => escaped.push(character),
        }
    }
    escaped
}

fn run_command_with_input_and_cancel(
    command: Command,
    input: Vec<u8>,
    should_cancel: &mut dyn FnMut() -> bool,
) -> Result<Output, String> {
    run_command_with_optional_input_and_cancel(command, Some(input), should_cancel)
}

fn run_command_with_optional_input_and_cancel(
    mut command: Command,
    input: Option<Vec<u8>>,
    should_cancel: &mut dyn FnMut() -> bool,
) -> Result<Output, String> {
    command.stdin(if input.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    });
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    let stdin_writer = input.map(|input| {
        let mut stdin = child.stdin.take().expect("piped stdin is available");
        thread::spawn(move || stdin.write_all(&input))
    });
    let stdout_reader = spawn_reader(child.stdout.take().expect("piped stdout is available"));
    let stderr_reader = spawn_reader(child.stderr.take().expect("piped stderr is available"));
    loop {
        if should_cancel() {
            let _ = child.kill();
            let _ = child.wait();
            drop(stdin_writer);
            drop(stdout_reader);
            drop(stderr_reader);
            return Err("cancelled_by_user".to_string());
        }
        match child.try_wait().map_err(|e| e.to_string())? {
            Some(status) => {
                return join_io_threads(stdin_writer, stdout_reader, stderr_reader, status);
            }
            None => thread::sleep(Duration::from_millis(50)),
        }
    }
}

fn spawn_reader(
    mut reader: impl Read + Send + 'static,
) -> thread::JoinHandle<std::io::Result<Vec<u8>>> {
    thread::spawn(move || {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes)?;
        Ok(bytes)
    })
}

fn join_io_threads(
    stdin_writer: Option<thread::JoinHandle<std::io::Result<()>>>,
    stdout_reader: thread::JoinHandle<std::io::Result<Vec<u8>>>,
    stderr_reader: thread::JoinHandle<std::io::Result<Vec<u8>>>,
    status: ExitStatus,
) -> Result<Output, String> {
    let input_result = stdin_writer.map(|writer| {
        writer
            .join()
            .map_err(|_| "model_request_stdin_writer_panicked".to_string())
            .and_then(|result| result.map_err(|err| format!("model_request_stdin_failed: {err}")))
    });
    let stdout = stdout_reader
        .join()
        .map_err(|_| "model_stdout_reader_panicked".to_string())?
        .map_err(|err| format!("model_stdout_read_failed: {err}"))?;
    let stderr = stderr_reader
        .join()
        .map_err(|_| "model_stderr_reader_panicked".to_string())?
        .map_err(|err| format!("model_stderr_read_failed: {err}"))?;
    if status.success() {
        if let Some(Err(err)) = input_result {
            return Err(err);
        }
    }
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

fn split_curl_body_status(stdout: &str) -> Result<(String, u16), String> {
    let trimmed = stdout.trim_end();
    let split_at = trimmed
        .rfind('\n')
        .ok_or_else(|| "missing_http_status".to_string())?;
    let (body, status_text) = trimmed.split_at(split_at);
    let status = status_text
        .trim()
        .parse::<u16>()
        .map_err(|_| "invalid_http_status".to_string())?;
    Ok((body.to_string(), status))
}

#[cfg(test)]
#[path = "../tests/unit/model_transport_tests.rs"]
mod tests;
