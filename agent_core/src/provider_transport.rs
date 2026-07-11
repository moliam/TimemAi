use crate::{
    append_audit_event, interpret_provider_http_response, prepare_provider_http_request,
    provider_request_audit_event, provider_response_audit_event, LlmResponse, ModelClient,
    ProviderConfig,
};
use std::path::Path;
use std::process::{Command, Output, Stdio};
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
    command.arg("--data").arg(body);
    let output = run_command_with_cancel(command, should_cancel)?;
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

fn run_command_with_cancel(
    mut command: Command,
    should_cancel: &mut dyn FnMut() -> bool,
) -> Result<Output, String> {
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    loop {
        if should_cancel() {
            let _ = child.kill();
            let _ = child.wait();
            return Err("cancelled_by_user".to_string());
        }
        match child.try_wait().map_err(|e| e.to_string())? {
            Some(_) => return child.wait_with_output().map_err(|e| e.to_string()),
            None => thread::sleep(Duration::from_millis(50)),
        }
    }
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
mod tests {
    use super::*;
    use crate::LocalLLMKeyFile;
    use std::time::Instant;

    fn local_llm_key_file_path() -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../key")
    }

    #[test]
    fn cancellable_command_returns_without_waiting_for_process_timeout() {
        let started = Instant::now();
        let cancel_after = Instant::now() + Duration::from_millis(80);
        let err = run_command_with_cancel(
            {
                let mut command = Command::new("sh");
                command.arg("-c").arg("sleep 5; echo done");
                command
            },
            &mut || Instant::now() >= cancel_after,
        )
        .unwrap_err();

        assert_eq!(err, "cancelled_by_user");
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn split_curl_body_status_parses_last_line_status() {
        let (body, status) = split_curl_body_status("{\"ok\":true}\n200").unwrap();
        assert_eq!(body, "{\"ok\":true}");
        assert_eq!(status, 200);
    }

    #[test]
    #[ignore = "requires rust/key with a real Aliyun-compatible API key and network access"]
    fn real_aliyun_model_from_key_file_returns_usage_and_text() {
        let key_file = LocalLLMKeyFile::load(&local_llm_key_file_path()).unwrap();
        let model = key_file.random_model().to_string();
        let config = key_file.to_provider_config(&model);
        let mut audit_file = std::env::temp_dir();
        audit_file.push(format!(
            "timem_real_llm_{}_{}.jsonl",
            model.replace('/', "_"),
            std::process::id()
        ));
        let _ = std::fs::remove_file(&audit_file);

        let response = call_model(
            &config,
            r#"Return exactly this JSON object and no markdown: {"status":"finished","final_answer":"pong"}"#,
            &audit_file,
        )
        .unwrap();

        assert_eq!(response.model_name, model);
        assert!(response.content.contains("free_talk") || response.content.contains("pong"));
        assert!(response.usage.llm_calls >= 1);
        assert!(response.usage.prompt_tokens > 0 || response.usage.total_tokens > 0);

        let audit_text = std::fs::read_to_string(&audit_file).unwrap();
        assert!(audit_text.contains("llm_request"));
        assert!(audit_text.contains("llm_response"));
        assert!(!audit_text.contains(&key_file.api_key));
        let _ = std::fs::remove_file(audit_file);
    }
}
