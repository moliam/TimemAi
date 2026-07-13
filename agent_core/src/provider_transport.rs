use crate::{
    append_audit_event, interpret_provider_http_response, prepare_provider_http_request,
    provider_request_audit_event, provider_response_audit_event, LlmResponse, ModelClient,
    ProviderConfig,
};
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, ExitStatus, Output, Stdio};
use std::thread;
use std::time::Duration;

pub struct ProviderModelClient;

impl ModelClient for ProviderModelClient {
    fn call_model(
        &mut self,
        config: &ProviderConfig,
        prompt: &str,
        audit_file: &Path,
        should_cancel: &mut dyn FnMut() -> bool,
    ) -> Result<LlmResponse, String> {
        call_model_with_cancel(config, prompt, audit_file, should_cancel)
    }
}

pub fn call_model(
    config: &ProviderConfig,
    prompt: &str,
    audit_file: &Path,
) -> Result<LlmResponse, String> {
    call_model_with_cancel(config, prompt, audit_file, &mut || false)
}

pub fn call_model_with_cancel(
    config: &ProviderConfig,
    prompt: &str,
    audit_file: &Path,
    should_cancel: &mut dyn FnMut() -> bool,
) -> Result<LlmResponse, String> {
    let http_request = prepare_provider_http_request(config, prompt);
    let _ = append_audit_event(
        audit_file,
        &provider_request_audit_event(config, &http_request.provider_request),
    );

    let mut command = Command::new("curl");
    command
        .arg("-sS")
        .arg("--max-time")
        .arg(config.timeout_secs.to_string())
        .arg("-w")
        .arg("\n%{http_code}")
        .arg("-X")
        .arg("POST")
        .arg(http_request.endpoint);
    for (key, value) in &http_request.headers {
        command.arg("-H").arg(format!("{key}: {value}"));
    }
    let body =
        serde_json::to_string(&http_request.provider_request.body).map_err(|e| e.to_string())?;
    command.arg("--data-binary").arg("@-");
    let output = run_command_with_input_and_cancel(command, body.into_bytes(), should_cancel)?;
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
    let interpreted = interpret_provider_http_response(config, status, &raw_text, &stderr);
    let _ = append_audit_event(
        audit_file,
        &provider_response_audit_event(interpreted.status, &interpreted.raw_json),
    );
    interpreted.result
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
            .map_err(|_| "provider_request_stdin_writer_panicked".to_string())
            .and_then(|result| {
                result.map_err(|err| format!("provider_request_stdin_failed: {err}"))
            })
    });
    let stdout = stdout_reader
        .join()
        .map_err(|_| "provider_stdout_reader_panicked".to_string())?
        .map_err(|err| format!("provider_stdout_read_failed: {err}"))?;
    let stderr = stderr_reader
        .join()
        .map_err(|_| "provider_stderr_reader_panicked".to_string())?
        .map_err(|err| format!("provider_stderr_read_failed: {err}"))?;
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
#[path = "../tests/unit/provider_transport_tests.rs"]
mod tests;
