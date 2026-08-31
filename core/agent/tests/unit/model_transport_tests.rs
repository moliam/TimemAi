use super::*;
use crate::{api_audit_stream_path, read_api_audit_doc, LocalLLMKeyFile};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::{Duration, Instant};

fn local_llm_key_file_path() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../key")
}

fn test_audit_file(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "timem_native_http_{label}_{}_{}.jsonl",
        std::process::id(),
        crate::now_ms()
    ))
}

fn success_body(content: &str) -> String {
    serde_json::json!({
        "choices": [{"message": {"content": content}, "finish_reason": "stop"}],
        "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}
    })
    .to_string()
}

fn local_config(addr: std::net::SocketAddr, timeout_secs: u64) -> ModelServiceConfig {
    let mut config = LocalLLMKeyFile {
        api_key: "native-http-test-key".to_string(),
        available_models: vec!["native-http-test-model".to_string()],
    }
    .to_model_service_config("native-http-test-model");
    config.base_url = format!("http://{addr}/v1");
    config.timeout_secs = timeout_secs;
    config
}

#[test]
fn cancellation_interrupts_waiting_for_response_headers() {
    let listener = match TcpListener::bind("127.0.0.1:0") {
        Ok(listener) => listener,
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => return,
        Err(error) => panic!("local test server bind failed: {error}"),
    };
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let _ = read_http_request(&mut stream);
        thread::sleep(Duration::from_secs(2));
    });

    let config = local_config(addr, 5);
    let audit_file = test_audit_file("cancel");
    let cancel_after = Instant::now() + Duration::from_millis(80);
    let started = Instant::now();
    let error = call_model_with_cancel(&config, "cancel me", &audit_file, &mut || {
        Instant::now() >= cancel_after
    })
    .unwrap_err();

    assert_eq!(error, "cancelled_by_user");
    assert!(
        started.elapsed() < Duration::from_millis(500),
        "native HTTP cancellation took {:?}",
        started.elapsed()
    );
    let _ = std::fs::remove_file(audit_file);
}

#[test]
fn native_http_sends_custom_headers_and_json_body() {
    let listener = match TcpListener::bind("127.0.0.1:0") {
        Ok(listener) => listener,
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => return,
        Err(error) => panic!("local test server bind failed: {error}"),
    };
    let addr = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let request = read_http_request(&mut stream);
        let body = success_body("ok");
        stream
            .write_all(&http_json_response("200 OK", &body))
            .unwrap();
        request
    });

    let mut config = local_config(addr, 2);
    config.http_headers.insert(
        "X-Signature".to_string(),
        "quoted=\"yes\"; path=C:\\tmp; city=东京".to_string(),
    );
    let audit_file = test_audit_file("headers");
    let response = call_model(&config, "private prompt body", &audit_file).unwrap();
    assert_eq!(response.content, "ok");

    let captured = server.join().unwrap();
    let captured_lower = captured.to_lowercase();
    assert!(captured_lower.contains("authorization: bearer native-http-test-key"));
    assert!(captured_lower.contains("x-signature: quoted=\"yes\"; path=c:\\tmp; city=东京"));
    assert!(captured.contains("private prompt body"));
    assert!(captured.contains(r#""model":"native-http-test-model""#));
    let _ = std::fs::remove_file(audit_file);
}

#[test]
fn two_megabyte_model_body_reaches_http_server_intact() {
    let listener = match TcpListener::bind("127.0.0.1:0") {
        Ok(listener) => listener,
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => return,
        Err(error) => panic!("local test server bind failed: {error}"),
    };
    let addr = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let request = read_http_request(&mut stream);
        let body = success_body("large-ok");
        stream
            .write_all(&http_json_response("200 OK", &body))
            .unwrap();
        request
    });

    let config = local_config(addr, 5);
    let marker = "large-body-tail-marker";
    let prompt = format!("{}{}", "x".repeat(2 * 1024 * 1024), marker);
    let audit_file = test_audit_file("large");
    let response = call_model(&config, &prompt, &audit_file).unwrap();
    assert_eq!(response.content, "large-ok");

    let captured = server.join().unwrap();
    let (_, captured_body) = captured.split_once("\r\n\r\n").unwrap();
    assert!(captured_body.len() > 2 * 1024 * 1024);
    assert!(captured_body.contains(marker));
    assert!(captured_body.contains(r#""model":"native-http-test-model""#));
    let _ = std::fs::remove_file(audit_file);
}

#[test]
fn progressing_response_may_outlive_configured_timeout() {
    let listener = match TcpListener::bind("127.0.0.1:0") {
        Ok(listener) => listener,
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => return,
        Err(error) => panic!("local test server bind failed: {error}"),
    };
    let addr = listener.local_addr().unwrap();
    let body = success_body("progress-ok");
    let split_one = body.len() / 3;
    let split_two = split_one * 2;
    let body_bytes = body.as_bytes();
    let pieces = vec![
        body_bytes[..split_one].to_vec(),
        body_bytes[split_one..split_two].to_vec(),
        body_bytes[split_two..].to_vec(),
    ];
    let body_len = body.len();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let _ = read_http_request(&mut stream);
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {body_len}\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        for piece in pieces {
            stream.write_all(&piece).unwrap();
            stream.flush().unwrap();
            thread::sleep(Duration::from_millis(600));
        }
    });

    let config = local_config(addr, 1);
    let audit_file = test_audit_file("progress");
    let started = Instant::now();
    let response = call_model(&config, "slow but progressing", &audit_file).unwrap();
    assert_eq!(response.content, "progress-ok");
    assert!(started.elapsed() > Duration::from_secs(1));
    server.join().unwrap();
    let _ = std::fs::remove_file(audit_file);
}

#[test]
fn stalled_response_hits_configured_inactivity_timeout() {
    let listener = match TcpListener::bind("127.0.0.1:0") {
        Ok(listener) => listener,
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => return,
        Err(error) => panic!("local test server bind failed: {error}"),
    };
    let addr = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let _ = read_http_request(&mut stream);
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\n{")
            .unwrap();
        stream.flush().unwrap();
        thread::sleep(Duration::from_millis(1300));
    });

    let config = local_config(addr, 1);
    let audit_file = test_audit_file("stall");
    let error = call_model(&config, "stall", &audit_file).unwrap_err();
    assert_eq!(error, "model_timeout: no response progress for 1 seconds");
    server.join().unwrap();
    let _ = std::fs::remove_file(audit_file);
}

#[test]
fn declared_oversized_response_is_rejected_before_body_read() {
    let listener = match TcpListener::bind("127.0.0.1:0") {
        Ok(listener) => listener,
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => return,
        Err(error) => panic!("local test server bind failed: {error}"),
    };
    let addr = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let _ = read_http_request(&mut stream);
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            MAX_MODEL_RESPONSE_BYTES + 1
        )
        .unwrap();
        stream.flush().unwrap();
    });

    let config = local_config(addr, 2);
    let audit_file = test_audit_file("declared-oversized-response");
    let error = call_model(&config, "oversized", &audit_file).unwrap_err();
    assert_eq!(error, model_response_too_large());
    server.join().unwrap();
    let _ = std::fs::remove_file(audit_file);
}

#[test]
fn streaming_response_is_rejected_when_accumulated_body_crosses_limit() {
    let listener = match TcpListener::bind("127.0.0.1:0") {
        Ok(listener) => listener,
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => return,
        Err(error) => panic!("local test server bind failed: {error}"),
    };
    let addr = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let _ = read_http_request(&mut stream);
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n",
            )
            .unwrap();
        let chunk = vec![b'x'; 1024 * 1024];
        for _ in 0..=MAX_MODEL_RESPONSE_BYTES / chunk.len() {
            if stream.write_all(&chunk).is_err() {
                break;
            }
        }
    });

    let config = local_config(addr, 5);
    let audit_file = test_audit_file("streaming-oversized-response");
    let error = call_model(&config, "oversized stream", &audit_file).unwrap_err();
    assert_eq!(error, model_response_too_large());
    server.join().unwrap();
    let _ = std::fs::remove_file(audit_file);
}

#[test]
fn malformed_endpoint_is_a_model_network_error() {
    let mut config = local_config("127.0.0.1:1".parse().unwrap(), 2);
    config.base_url = "http://[invalid-host".to_string();
    let audit_file = test_audit_file("network-error");
    let error = call_model(&config, "invalid endpoint", &audit_file).unwrap_err();
    assert!(error.starts_with("model_network_error:"), "{error}");
    let _ = std::fs::remove_file(audit_file);
}

fn read_http_request(stream: &mut std::net::TcpStream) -> String {
    let mut request = Vec::new();
    let mut buffer = [0_u8; 4096];
    let mut expected_len = None;

    loop {
        let read = stream.read(&mut buffer).unwrap();
        if read == 0 {
            break;
        }
        request.extend_from_slice(&buffer[..read]);

        if expected_len.is_none() {
            if let Some(header_end) = request.windows(4).position(|part| part == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&request[..header_end]);
                let content_len = headers
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())
                            .flatten()
                    })
                    .unwrap_or(0);
                expected_len = Some(header_end + 4 + content_len);
            }
        }

        if expected_len.is_some_and(|length| request.len() >= length) {
            break;
        }
    }

    String::from_utf8_lossy(&request).to_string()
}

fn http_json_response(status: &str, body: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

#[test]
fn explicit_cache_schema_rejection_retries_once_without_cache_control() {
    let listener = match TcpListener::bind("127.0.0.1:0") {
        Ok(listener) => listener,
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => return,
        Err(error) => panic!("local test server bind failed: {error}"),
    };
    let addr = listener.local_addr().unwrap();

    let server = thread::spawn(move || {
        let mut requests = Vec::new();

        let (mut first, _) = listener.accept().unwrap();
        requests.push(read_http_request(&mut first));
        let first_body = r#"{"error":{"message":"Unknown field cache_control is not permitted"}}"#;
        first
            .write_all(&http_json_response("400 Bad Request", first_body))
            .unwrap();

        let (mut second, _) = listener.accept().unwrap();
        requests.push(read_http_request(&mut second));
        let second_body = r#"{
            "choices":[{"message":{"content":"ok"},"finish_reason":"stop"}],
            "usage":{"prompt_tokens":10,"completion_tokens":1,"total_tokens":11}
        }"#;
        second
            .write_all(&http_json_response("200 OK", second_body))
            .unwrap();

        requests
    });

    let mut config = LocalLLMKeyFile {
        api_key: "test-key".to_string(),
        available_models: vec!["test-model".to_string()],
    }
    .to_model_service_config("test-model");
    config.base_url = format!("http://{addr}/v1");
    config.openai_compatible.cache_mode = OpenAiCompatibleCacheMode::Ephemeral;

    let audit_file = std::env::temp_dir().join(format!(
        "timem_cache_fallback_audit_{}_{}.jsonl",
        std::process::id(),
        crate::now_ms()
    ));
    let _ = std::fs::remove_file(&audit_file);

    let response = call_model(
        &config,
        "[BEGIN SYSTEM PROMPT]\nstable prefix\n[END SYSTEM PROMPT]",
        &audit_file,
    )
    .unwrap();
    assert_eq!(response.content, "ok");

    let requests = server.join().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests[0].contains(r#""cache_control""#));
    assert!(!requests[1].contains(r#""cache_control""#));

    let audit = read_api_audit_doc(&api_audit_stream_path(&audit_file)).unwrap();
    let request_events = audit["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|event| event["type"] == "llm_request")
        .collect::<Vec<_>>();
    assert_eq!(request_events.len(), 2);
    assert_eq!(request_events[0]["prompt_cache_wire"]["mode"], "ephemeral");
    assert!(
        request_events[0]["prompt_cache_wire"]["mark_count"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert_eq!(request_events[0]["prompt_cache_wire"]["fallback"], false);
    assert_eq!(
        request_events[1]["prompt_cache_wire"]["mode"],
        "auto-fallback"
    );
    assert_eq!(request_events[1]["prompt_cache_wire"]["mark_count"], 0);
    assert_eq!(request_events[1]["prompt_cache_wire"]["fallback"], true);

    let _ = std::fs::remove_file(audit_file);
}

#[test]
fn unrelated_client_error_does_not_retry_without_cache_control() {
    let listener = match TcpListener::bind("127.0.0.1:0") {
        Ok(listener) => listener,
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => return,
        Err(error) => panic!("local test server bind failed: {error}"),
    };
    let addr = listener.local_addr().unwrap();

    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let request = read_http_request(&mut stream);
        let body = r#"{"error":{"message":"Invalid API key"}}"#;
        stream
            .write_all(&http_json_response("401 Unauthorized", body))
            .unwrap();
        request
    });

    let mut config = LocalLLMKeyFile {
        api_key: "test-key".to_string(),
        available_models: vec!["test-model".to_string()],
    }
    .to_model_service_config("test-model");
    config.base_url = format!("http://{addr}/v1");
    config.openai_compatible.cache_mode = OpenAiCompatibleCacheMode::Ephemeral;

    let audit_file = std::env::temp_dir().join(format!(
        "timem_cache_no_fallback_audit_{}_{}.jsonl",
        std::process::id(),
        crate::now_ms()
    ));
    let _ = std::fs::remove_file(&audit_file);

    let error = call_model(
        &config,
        "[BEGIN SYSTEM PROMPT]\nstable prefix\n[END SYSTEM PROMPT]",
        &audit_file,
    )
    .unwrap_err();
    assert!(error.starts_with("model_http_401:"));

    let request = server.join().unwrap();
    assert!(request.contains(r#""cache_control""#));

    let audit = read_api_audit_doc(&api_audit_stream_path(&audit_file)).unwrap();
    let request_count = audit["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|event| event["type"] == "llm_request")
        .count();
    assert_eq!(request_count, 1);

    let _ = std::fs::remove_file(audit_file);
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
