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
    let err = run_command_with_optional_input_and_cancel(
        {
            let mut command = Command::new("sh");
            command.arg("-c").arg("sleep 5; echo done");
            command
        },
        None,
        &mut || Instant::now() >= cancel_after,
    )
    .unwrap_err();

    assert_eq!(err, "cancelled_by_user");
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[test]
fn large_provider_body_is_streamed_through_stdin_without_argv_limits() {
    let body = vec![b'x'; 4 * 1024 * 1024];
    let output = run_command_with_input_and_cancel(
        {
            let mut command = Command::new("sh");
            command
                .arg("-c")
                .arg("received=$(wc -c | tr -d ' '); printf '%s\\n200' \"$received\"");
            command
        },
        body,
        &mut || false,
    )
    .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let (received, status) = split_curl_body_status(&stdout).unwrap();
    assert_eq!(status, 200);
    assert_eq!(received, (4 * 1024 * 1024).to_string());
}

#[test]
fn large_stdout_and_stderr_are_drained_without_pipe_deadlock() {
    let output = run_command_with_optional_input_and_cancel(
        {
            let mut command = Command::new("sh");
            command
                .arg("-c")
                .arg("head -c 2097152 /dev/zero; head -c 2097152 /dev/zero >&2");
            command
        },
        None,
        &mut || false,
    )
    .unwrap();

    assert!(output.status.success());
    assert_eq!(output.stdout.len(), 2 * 1024 * 1024);
    assert_eq!(output.stderr.len(), 2 * 1024 * 1024);
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
