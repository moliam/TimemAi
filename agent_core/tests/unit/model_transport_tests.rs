use super::*;
use crate::LocalLLMKeyFile;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;
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
fn large_model_request_body_is_streamed_through_stdin_without_argv_limits() {
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
fn model_api_curl_command_does_not_expose_secret_or_body_in_argv() {
    let key_file = LocalLLMKeyFile {
        api_key: "sk-test-secret".to_string(),
        available_models: vec!["qwen-test".to_string()],
    };
    let config = key_file.to_model_service_config("qwen-test");
    let request = prepare_model_http_request(&config, "prompt with private body");
    let body = serde_json::to_string(&request.model_request.body).unwrap();
    let command = build_curl_command(config.timeout_secs);
    let argv = command
        .get_args()
        .map(|arg| arg.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ");

    assert!(!argv.contains("sk-test-secret"), "{argv}");
    assert!(!argv.contains("prompt with private body"), "{argv}");
    assert!(argv.contains("--config -"), "{argv}");

    let curl_config = build_curl_config(&request, &body);
    assert!(curl_config.contains("Authorization: Bearer sk-test-secret"));
    assert!(curl_config.contains("prompt with private body"));
}

#[test]
fn curl_config_escape_keeps_values_single_config_entries() {
    let escaped = curl_config_escape("quote\" slash\\ newline\n tab\t");
    assert_eq!(escaped, "quote\\\" slash\\\\ newline\\n tab\\t");
}

#[test]
fn curl_config_stdin_sends_headers_and_body_without_argv_payload() {
    let listener = match TcpListener::bind("127.0.0.1:0") {
        Ok(listener) => listener,
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => return,
        Err(error) => panic!("local test server bind failed: {error}"),
    };
    let addr = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        let mut buffer = [0_u8; 4096];
        loop {
            let read = stream.read(&mut buffer).unwrap();
            if read == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..read]);
            let text = String::from_utf8_lossy(&request);
            if text.contains("\r\n\r\n") && text.contains("\"model\"") {
                break;
            }
        }
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}")
            .unwrap();
        String::from_utf8_lossy(&request).to_string()
    });

    let key_file = LocalLLMKeyFile {
        api_key: "sk-local-curl-secret".to_string(),
        available_models: vec!["qwen-test".to_string()],
    };
    let expected_authorization = format!("Authorization: Bearer {}", key_file.api_key);
    let mut config = key_file.to_model_service_config("qwen-test");
    config.base_url = format!("http://{addr}/v1");
    let request = prepare_model_http_request(&config, "prompt body through curl config");
    let body = serde_json::to_string(&request.model_request.body).unwrap();
    let output = run_command_with_input_and_cancel(
        build_curl_command(config.timeout_secs),
        build_curl_config(&request, &body).into_bytes(),
        &mut || false,
    )
    .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let (_body, status) = split_curl_body_status(&stdout).unwrap();
    assert_eq!(status, 200);
    let captured = server.join().unwrap();
    assert!(captured.contains(&expected_authorization));
    assert!(captured.contains("prompt body through curl config"));
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
    let config = key_file.to_model_service_config(&model);
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
