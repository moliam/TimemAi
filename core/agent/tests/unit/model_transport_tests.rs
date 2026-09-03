use super::*;
use crate::{
    api_audit_stream_path, is_retryable_model_system_error, read_api_audit_doc, LocalLLMKeyFile,
};
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
fn connection_closed_before_response_headers_is_retryable_network_error() {
    let listener = match TcpListener::bind("127.0.0.1:0") {
        Ok(listener) => listener,
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => return,
        Err(error) => panic!("local test server bind failed: {error}"),
    };
    let addr = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let request = read_http_request(&mut stream);
        assert!(!request.is_empty());
        // Drop the socket without sending an HTTP status line or headers.
    });

    let config = local_config(addr, 2);
    let audit_file = test_audit_file("closed-before-headers");
    let error = call_model(&config, "retry this transport failure", &audit_file).unwrap_err();
    assert!(
        error.starts_with("model_network_error: stage=response_headers"),
        "{error}"
    );
    assert!(is_retryable_model_system_error(&error), "{error}");
    server.join().unwrap();
    let _ = std::fs::remove_file(audit_file);
}

#[test]
fn response_body_connection_close_is_retryable_body_error() {
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
                b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 100\r\nConnection: close\r\n\r\n{",
            )
            .unwrap();
        stream.flush().unwrap();
    });

    let config = local_config(addr, 2);
    let audit_file = test_audit_file("closed-response-body");
    let error = call_model(&config, "retry truncated body", &audit_file).unwrap_err();
    assert!(
        error.starts_with("model_body_error: stage=response_body"),
        "{error}"
    );
    assert!(is_retryable_model_system_error(&error), "{error}");
    server.join().unwrap();
    let _ = std::fs::remove_file(audit_file);
}

#[test]
fn transport_failure_markers_exclude_permanent_request_and_tls_errors() {
    for transient in [
        "can't assign requested address (os error 49)",
        "cannot assign requested address (os error 99)",
        "address not available (os error 10049)",
        "EADDRNOTAVAIL while opening socket",
        "connection closed before message completed",
        "connection reset by peer",
        "broken pipe",
        "unexpected eof while reading response",
        "incomplete message",
        "http2 framing layer failure",
        "h2 protocol error",
    ] {
        assert!(is_transient_connection_failure(transient), "{transient}");
    }
    let addr_not_available = std::io::Error::from(std::io::ErrorKind::AddrNotAvailable);
    assert!(has_retryable_socket_error(&addr_not_available));
    let invalid_input = std::io::Error::from(std::io::ErrorKind::InvalidInput);
    assert!(!has_retryable_socket_error(&invalid_input));

    for permanent in [
        "builder error: invalid header value",
        "relative url without a base",
        "invalid peer certificate: unknown issuer",
        "dns lookup failed",
        "proxy authentication required",
    ] {
        assert!(!is_transient_connection_failure(permanent), "{permanent}");
    }
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
    assert_eq!(
        error,
        "model_timeout: stage=response_body no progress for 1 seconds"
    );
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
fn malformed_endpoint_is_a_request_url_error() {
    let mut config = local_config("127.0.0.1:1".parse().unwrap(), 2);
    config.base_url = "http://[invalid-host".to_string();
    let audit_file = test_audit_file("network-error");
    let error = call_model(&config, "invalid endpoint", &audit_file).unwrap_err();
    assert!(error.starts_with("model_request_url_error:"), "{error}");
    let _ = std::fs::remove_file(audit_file);
}

fn read_http_request(stream: &mut impl Read) -> String {
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

fn http_redirect_response(location: &str) -> Vec<u8> {
    format!("HTTP/1.1 307 Temporary Redirect\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").into_bytes()
}

#[test]
fn cross_origin_redirect_strips_sensitive_headers_when_enabled() {
    let source = match TcpListener::bind("127.0.0.1:0") {
        Ok(listener) => listener,
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => return,
        Err(error) => panic!("bind failed: {error}"),
    };
    let target = TcpListener::bind("127.0.0.1:0").unwrap();
    let source_addr = source.local_addr().unwrap();
    let target_addr = target.local_addr().unwrap();
    let redirector = thread::spawn(move || {
        let (mut stream, _) = source.accept().unwrap();
        let _ = read_http_request(&mut stream);
        stream
            .write_all(&http_redirect_response(&format!(
                "http://{target_addr}/v1/target"
            )))
            .unwrap();
    });
    let receiver = thread::spawn(move || {
        let (mut stream, _) = target.accept().unwrap();
        let request = read_http_request(&mut stream);
        let body = success_body("redirect-ok");
        stream
            .write_all(&http_json_response("200 OK", &body))
            .unwrap();
        request
    });
    let mut config = local_config(source_addr, 2);
    config.http_transport.allow_cross_origin_redirects = true;
    config
        .http_headers
        .insert("X-Tenant".into(), "secret-tenant".into());
    let audit = test_audit_file("cross-origin-strip");
    assert_eq!(
        call_model(&config, "redirect", &audit).unwrap().content,
        "redirect-ok"
    );
    redirector.join().unwrap();
    let request = receiver.join().unwrap().to_ascii_lowercase();
    assert!(!request.contains("authorization:"));
    assert!(!request.contains("x-tenant:"));
    assert!(request.contains("content-type: application/json"));
    let _ = std::fs::remove_file(audit);
}

#[test]
fn cross_origin_redirect_is_blocked_before_target_contact_by_default() {
    let source = match TcpListener::bind("127.0.0.1:0") {
        Ok(listener) => listener,
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => return,
        Err(error) => panic!("bind failed: {error}"),
    };
    let target = TcpListener::bind("127.0.0.1:0").unwrap();
    target.set_nonblocking(true).unwrap();
    let source_addr = source.local_addr().unwrap();
    let target_addr = target.local_addr().unwrap();
    let redirector = thread::spawn(move || {
        let (mut stream, _) = source.accept().unwrap();
        let _ = read_http_request(&mut stream);
        stream
            .write_all(&http_redirect_response(&format!(
                "http://{target_addr}/v1/target"
            )))
            .unwrap();
    });
    let config = local_config(source_addr, 2);
    let audit = test_audit_file("cross-origin-block");
    let error = call_model(&config, "redirect", &audit).unwrap_err();
    assert!(error.starts_with("model_redirect_blocked:"), "{error}");
    redirector.join().unwrap();
    thread::sleep(Duration::from_millis(30));
    assert!(
        matches!(target.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
    );
    let _ = std::fs::remove_file(audit);
}

#[test]
fn oversized_request_is_rejected_before_connecting() {
    let listener = match TcpListener::bind("127.0.0.1:0") {
        Ok(listener) => listener,
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => return,
        Err(error) => panic!("bind failed: {error}"),
    };
    listener.set_nonblocking(true).unwrap();
    let config = local_config(listener.local_addr().unwrap(), 2);
    let audit = test_audit_file("oversized-request");
    let error =
        call_model(&config, &"x".repeat(MAX_MODEL_REQUEST_BYTES + 1024), &audit).unwrap_err();
    assert_eq!(error, model_request_too_large());
    assert!(
        matches!(listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
    );
    let _ = std::fs::remove_file(audit);
}

#[test]
fn response_audit_contains_sanitized_transport_metrics() {
    let listener = match TcpListener::bind("127.0.0.1:0") {
        Ok(listener) => listener,
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => return,
        Err(error) => panic!("bind failed: {error}"),
    };
    let addr = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let _ = read_http_request(&mut stream);
        let body = success_body("audit-ok");
        write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nX-Request-Id: request-123\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
    });
    let config = local_config(addr, 2);
    let audit = test_audit_file("transport-metrics");
    call_model(&config, "audit", &audit).unwrap();
    server.join().unwrap();
    let stream_path = api_audit_stream_path(&audit);
    let document = read_api_audit_doc(&stream_path).unwrap();
    let events = document["events"].as_array().unwrap();
    let transport = events
        .iter()
        .find_map(|event| event.get("transport"))
        .unwrap();
    assert_eq!(transport["request_id"], "request-123");
    assert!(transport["ttfb_ms"].is_u64());
    assert!(transport["elapsed_ms"].is_u64());
    assert!(transport["response_bytes"].as_u64().unwrap() > 0);
    assert!(!document.to_string().contains("native-http-test-key"));
    let _ = std::fs::remove_file(audit);
    let _ = std::fs::remove_file(stream_path);
}

#[test]
fn private_ca_enables_self_signed_https_endpoint() {
    use rcgen::{Certificate as GeneratedCertificate, CertificateParams, SanType};
    use rustls::{
        Certificate as RustlsCertificate, PrivateKey, ServerConfig, ServerConnection, StreamOwned,
    };
    use std::net::IpAddr;
    use std::sync::Arc;

    let mut params = CertificateParams::new(vec!["localhost".to_string()]);
    params
        .subject_alt_names
        .push(SanType::IpAddress(IpAddr::from([127, 0, 0, 1])));
    let generated = GeneratedCertificate::from_params(params).unwrap();
    let certificate_der = generated.serialize_der().unwrap();
    let certificate_pem = generated.serialize_pem().unwrap();
    let private_key = PrivateKey(generated.serialize_private_key_der());
    let server_config = Arc::new(
        ServerConfig::builder()
            .with_safe_defaults()
            .with_no_client_auth()
            .with_single_cert(vec![RustlsCertificate(certificate_der)], private_key)
            .unwrap(),
    );
    let listener = match TcpListener::bind("127.0.0.1:0") {
        Ok(listener) => listener,
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => return,
        Err(error) => panic!("bind failed: {error}"),
    };
    let addr = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let mut successful_requests = 0;
        for _ in 0..2 {
            let (tcp, _) = listener.accept().unwrap();
            let connection = ServerConnection::new(server_config.clone()).unwrap();
            let mut tls = StreamOwned::new(connection, tcp);
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                read_http_request(&mut tls)
            })) {
                Ok(request) if !request.is_empty() => {
                    successful_requests += 1;
                    let body = success_body("private-ca-ok");
                    tls.write_all(&http_json_response("200 OK", &body)).unwrap();
                }
                _ => {}
            }
        }
        successful_requests
    });

    let mut config = local_config(addr, 2);
    config.base_url = format!("https://{addr}/v1");
    let audit_without_ca = test_audit_file("private-ca-missing");
    let error = call_model(&config, "tls", &audit_without_ca).unwrap_err();
    assert!(error.starts_with("model_tls_error:"), "{error}");

    config.http_transport.private_ca_pem = Some(certificate_pem);
    let audit_with_ca = test_audit_file("private-ca-configured");
    let response = call_model(&config, "tls", &audit_with_ca).unwrap();
    assert_eq!(response.content, "private-ca-ok");
    assert_eq!(server.join().unwrap(), 1);
    let _ = std::fs::remove_file(audit_without_ca);
    let _ = std::fs::remove_file(audit_with_ca);
}

#[test]
fn native_http_client_reuses_keep_alive_connection() {
    let listener = match TcpListener::bind("127.0.0.1:0") {
        Ok(listener) => listener,
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => return,
        Err(error) => panic!("bind failed: {error}"),
    };
    let addr = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        for content in ["first", "second"] {
            let request = read_http_request(&mut stream);
            assert!(!request.is_empty());
            let body = success_body(content);
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n{body}", body.len()).unwrap();
            stream.flush().unwrap();
        }
        listener.set_nonblocking(true).unwrap();
        thread::sleep(Duration::from_millis(30));
        matches!(listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
    });
    let config = local_config(addr, 2);
    let mut client = HttpModelClient::default();
    let first_audit = test_audit_file("reuse-first");
    let second_audit = test_audit_file("reuse-second");
    assert_eq!(
        client
            .call_model(&config, "one", &first_audit, &mut || false)
            .unwrap()
            .content,
        "first"
    );
    assert_eq!(
        client
            .call_model(&config, "two", &second_audit, &mut || false)
            .unwrap()
            .content,
        "second"
    );
    assert!(
        server.join().unwrap(),
        "client opened an unexpected second TCP connection"
    );
    let _ = std::fs::remove_file(first_audit);
    let _ = std::fs::remove_file(second_audit);
}

#[test]
#[ignore = "mutates process proxy environment; run serially"]
fn proxy_and_no_proxy_environment_smoke() {
    use std::collections::BTreeMap;
    use std::sync::{Mutex, OnceLock};

    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    let _guard = ENV_LOCK.get_or_init(|| Mutex::new(())).lock().unwrap();
    const KEYS: &[&str] = &[
        "HTTP_PROXY",
        "http_proxy",
        "HTTPS_PROXY",
        "https_proxy",
        "ALL_PROXY",
        "all_proxy",
        "NO_PROXY",
        "no_proxy",
    ];
    struct RestoreEnv(BTreeMap<&'static str, Option<String>>);
    impl Drop for RestoreEnv {
        fn drop(&mut self) {
            for (key, value) in &self.0 {
                match value {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }
            }
        }
    }
    let _restore = RestoreEnv(
        KEYS.iter()
            .map(|key| (*key, std::env::var(key).ok()))
            .collect(),
    );
    for key in KEYS {
        std::env::remove_var(key);
    }

    let proxy_port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let target = TcpListener::bind("127.0.0.1:0").unwrap();
    target.set_nonblocking(true).unwrap();
    let target_addr = target.local_addr().unwrap();
    for key in ["HTTP_PROXY", "http_proxy"] {
        std::env::set_var(key, format!("http://127.0.0.1:{proxy_port}"));
    }

    let config = local_config(target_addr, 1);
    let proxy_audit = test_audit_file("proxy-smoke");
    let proxy_error = HttpModelClient::default()
        .call_model(&config, "proxy", &proxy_audit, &mut || false)
        .unwrap_err();
    assert!(
        proxy_error.starts_with("model_connect_error:")
            || proxy_error.starts_with("model_proxy_error:"),
        "{proxy_error}"
    );
    assert!(
        matches!(target.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
    );

    for key in ["NO_PROXY", "no_proxy"] {
        std::env::set_var(key, "127.0.0.1,localhost");
    }
    let no_proxy_audit = test_audit_file("no-proxy-smoke");
    let no_proxy_error = HttpModelClient::default()
        .call_model(&config, "no proxy", &no_proxy_audit, &mut || false)
        .unwrap_err();
    assert!(
        no_proxy_error.starts_with("model_timeout: stage=response_headers"),
        "{no_proxy_error}"
    );
    assert!(target.accept().is_ok(), "NO_PROXY did not bypass the proxy");
    let _ = std::fs::remove_file(proxy_audit);
    let _ = std::fs::remove_file(no_proxy_audit);
}
