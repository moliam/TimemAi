use super::*;
use std::net::{TcpListener, TcpStream};

fn stdio_config(script: &str) -> McpServerConfig {
    McpServerConfig {
        id: "demo".to_string(),
        name: "Demo server".to_string(),
        enabled: true,
        transport: McpTransportConfig::Stdio {
            command: "/bin/sh".to_string(),
            args: vec!["-c".to_string(), script.to_string()],
            env: BTreeMap::new(),
        },
        request_timeout_ms: 2_000,
    }
}

fn fake_server_script() -> &'static str {
    r#"
while IFS= read -r line; do
  case "$line" in
    *\"method\":\"initialize\"*)
      printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-03-26","capabilities":{"tools":{}},"serverInfo":{"name":"fake","version":"1"},"instructions":"Always inspect metadata before using this server."}}'
      ;;
    *\"method\":\"tools/list\"*)
      printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"echo-value","description":"Echo a value","inputSchema":{"type":"object","properties":{"value":{"type":"string"}},"required":["value"]}}]}}'
      ;;
    *\"method\":\"tools/call\"*)
      printf '%s\n' '{"jsonrpc":"2.0","id":3,"result":{"content":[{"type":"text","text":"echoed by fake MCP"}],"isError":false}}'
      ;;
  esac
done
"#
}

#[test]
fn stdio_client_initializes_discovers_and_calls_tool() {
    let runtime = McpRuntime::default();
    let config = stdio_config(fake_server_script());
    let capabilities = runtime.connect_with_capabilities(&config).unwrap();
    assert_eq!(
        capabilities.instructions.as_deref(),
        Some("Always inspect metadata before using this server.")
    );
    assert_eq!(capabilities.tools.len(), 1);
    assert_eq!(capabilities.tools[0].action_name, "mcp.demo.echo-value");
    assert_eq!(capabilities.tools[0].input_schema["required"][0], "value");

    let result = runtime
        .call_tool(&config, "echo-value", &json!({ "value": "hello" }))
        .unwrap();
    assert!(result.contains("Action result: MCP Demo server/echo-value"));
    assert!(result.contains("echoed by fake MCP"));
}

#[test]
fn mcp_tool_error_has_structured_failed_status() {
    let script = r#"
while IFS= read -r line; do
  case "$line" in
    *\"method\":\"initialize\"*)
      printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-03-26","capabilities":{"tools":{}},"serverInfo":{"name":"fake","version":"1"}}}'
      ;;
    *\"method\":\"tools/list\"*)
      printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"fails","description":"Fails structurally","inputSchema":{"type":"object"}}]}}'
      ;;
    *\"method\":\"tools/call\"*)
      printf '%s\n' '{"jsonrpc":"2.0","id":3,"result":{"content":[{"type":"text","text":"ordinary payload text"}],"isError":true}}'
      ;;
  esac
done
"#;
    let runtime = McpRuntime::default();
    let config = stdio_config(script);
    runtime.connect(&config).unwrap();

    let outcome = runtime
        .call_tool_outcome(&config, "fails", &json!({}))
        .unwrap();

    assert_eq!(outcome.status, crate::ActionStatus::Failed);
    assert!(
        outcome.text.contains("status: tool_error"),
        "{}",
        outcome.text
    );
    assert!(
        outcome.text.contains("ordinary payload text"),
        "{}",
        outcome.text
    );
}

#[test]
fn legacy_sse_client_discovers_and_calls_tool() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (events_tx, events_rx) = mpsc::channel::<Value>();
    let events_rx = Arc::new(Mutex::new(events_rx));
    let server = thread::spawn(move || {
        for stream in listener.incoming().take(5) {
            let stream = stream.unwrap();
            let events_tx = events_tx.clone();
            let events_rx = events_rx.clone();
            thread::spawn(move || serve_legacy_sse_request(stream, events_tx, events_rx));
        }
    });
    let config = McpServerConfig {
        id: "legacy".to_string(),
        name: "Legacy SSE".to_string(),
        enabled: true,
        transport: McpTransportConfig::Sse {
            url: format!("http://{address}/sse"),
            headers: BTreeMap::new(),
        },
        request_timeout_ms: 2_000,
    };
    let runtime = McpRuntime::default();
    let capabilities = runtime.connect_with_capabilities(&config).unwrap();
    assert_eq!(
        capabilities.instructions.as_deref(),
        Some("Use the legacy workflow before calling tools.")
    );
    assert_eq!(capabilities.tools[0].action_name, "mcp.legacy.echo");
    let result = runtime
        .call_tool(&config, "echo", &json!({ "value": "hello" }))
        .unwrap();
    assert!(result.contains("legacy SSE result"), "{result}");
    runtime.disconnect_all();
    server.join().unwrap();
}

fn serve_legacy_sse_request(
    mut stream: TcpStream,
    events_tx: mpsc::Sender<Value>,
    events_rx: Arc<Mutex<mpsc::Receiver<Value>>>,
) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut request_line = String::new();
    reader.read_line(&mut request_line).unwrap();
    let mut content_length = 0_usize;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        if line == "\r\n" || line == "\n" || line.is_empty() {
            break;
        }
        if let Some(value) = line
            .split_once(':')
            .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
            .and_then(|(_, value)| value.trim().parse::<usize>().ok())
        {
            content_length = value;
        }
    }
    if request_line.starts_with("GET /sse ") {
        drop(events_tx);
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\nevent: endpoint\ndata: /messages\r\n\r\n",
            )
            .unwrap();
        while let Ok(message) = events_rx.lock().unwrap().recv() {
            let event = format!("event: message\r\ndata: {message}\r\n\r\n");
            if stream.write_all(event.as_bytes()).is_err() || stream.flush().is_err() {
                break;
            }
        }
        return;
    }
    let mut body = vec![0_u8; content_length];
    reader.read_exact(&mut body).unwrap();
    let request: Value = serde_json::from_slice(&body).unwrap();
    stream
        .write_all(b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
        .unwrap();
    let Some(id) = request.get("id").cloned() else {
        return;
    };
    let result = match request.get("method").and_then(Value::as_str) {
        Some("initialize") => json!({
            "protocolVersion": LEGACY_SSE_PROTOCOL_VERSION,
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "legacy-test", "version": "1" },
            "instructions": "Use the legacy workflow before calling tools."
        }),
        Some("tools/list") => json!({
            "tools": [{
                "name": "echo",
                "description": "Echo",
                "inputSchema": { "type": "object", "properties": {} }
            }]
        }),
        Some("tools/call") => json!({
            "content": [{ "type": "text", "text": "legacy SSE result" }]
        }),
        other => panic!("unexpected MCP method: {other:?}"),
    };
    events_tx
        .send(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
        .unwrap();
}

#[test]
fn stdio_client_reports_timeout_without_hanging() {
    let runtime = McpRuntime::default();
    let mut config = stdio_config("while IFS= read -r line; do :; done");
    config.request_timeout_ms = 30;
    assert_eq!(runtime.connect(&config).unwrap_err(), "mcp_request_timeout");
}

#[test]
fn stdio_notifications_do_not_extend_request_deadline() {
    let runtime = McpRuntime::default();
    let mut config = stdio_config(
        r#"while IFS= read -r line; do
  i=0
  while [ "$i" -lt 20 ]; do
    printf '%s\n' '{"jsonrpc":"2.0","method":"notifications/progress","params":{}}'
    sleep 0.01
    i=$((i + 1))
  done
done"#,
    );
    config.request_timeout_ms = 40;
    let started = Instant::now();
    assert_eq!(runtime.connect(&config).unwrap_err(), "mcp_request_timeout");
    assert!(started.elapsed() < Duration::from_millis(180));
}

#[test]
fn stalled_mcp_call_does_not_block_an_independent_server() {
    let runtime = McpRuntime::default();
    let mut stalled = stdio_config(
        r#"while IFS= read -r line; do
  case "$line" in
    *\"method\":\"initialize\"*) printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-03-26","capabilities":{},"serverInfo":{"name":"stalled","version":"1"}}}';;
    *\"method\":\"tools/list\"*) printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"wait","inputSchema":{"type":"object"}}]}}';;
    *\"method\":\"tools/call\"*) :;;
  esac
done"#,
    );
    stalled.id = "stalled".to_string();
    stalled.name = "Stalled server".to_string();
    stalled.request_timeout_ms = 150;

    let mut healthy = stdio_config(fake_server_script());
    healthy.id = "healthy".to_string();
    healthy.name = "Healthy server".to_string();
    runtime.connect(&stalled).unwrap();
    runtime.connect(&healthy).unwrap();

    let stalled_runtime = runtime.clone();
    let stalled_config = stalled.clone();
    let stalled_call =
        thread::spawn(move || stalled_runtime.call_tool(&stalled_config, "wait", &json!({})));
    thread::sleep(Duration::from_millis(20));

    let started = Instant::now();
    let result = runtime
        .call_tool(&healthy, "echo-value", &json!({ "value": "ready" }))
        .unwrap();
    assert!(started.elapsed() < Duration::from_millis(100));
    assert!(result.contains("echoed by fake MCP"));
    assert_eq!(
        stalled_call.join().unwrap().unwrap_err(),
        "mcp_request_timeout"
    );
}

#[test]
fn store_round_trips_server_definitions() {
    let root = std::env::temp_dir().join(format!("timem_mcp_store_{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let store = McpStore::new(&root);
    let config = stdio_config(fake_server_script());
    store.save(std::slice::from_ref(&config)).unwrap();
    assert_eq!(store.list().unwrap(), vec![config]);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn action_names_are_stable_and_namespaced() {
    assert_eq!(
        mcp_action_name("Git Hub", "search/issues"),
        "mcp.git_hub.search_issues"
    );
}

#[test]
fn server_config_rejects_ambiguous_ids_and_header_injection() {
    let runtime = McpRuntime::default();
    let mut config = stdio_config(fake_server_script());
    config.id = "Git Hub".to_string();
    assert_eq!(
        runtime.connect(&config).unwrap_err(),
        "mcp_server_id_must_be_canonical"
    );

    config.id = "remote".to_string();
    config.transport = McpTransportConfig::StreamableHttp {
        url: "https://example.invalid/mcp".to_string(),
        headers: BTreeMap::from([(
            "Authorization".to_string(),
            "Bearer token\r\nX-Injected: true".to_string(),
        )]),
    };
    assert_eq!(
        runtime.connect(&config).unwrap_err(),
        "mcp_http_header_value_invalid:Authorization"
    );
}

#[test]
fn discovery_rejects_tool_names_that_normalize_to_one_action() {
    let runtime = McpRuntime::default();
    let config = stdio_config(
        r#"while IFS= read -r line; do case "$line" in
  *\"method\":\"initialize\"*) printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-03-26","capabilities":{},"serverInfo":{"name":"fake","version":"1"}}}';;
  *\"method\":\"tools/list\"*) printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"same/name","inputSchema":{"type":"object"}},{"name":"same_name","inputSchema":{"type":"object"}}]}}';;
esac; done"#,
    );
    assert_eq!(
        runtime.connect(&config).unwrap_err(),
        "mcp_tool_action_name_collision:mcp.demo.same_name"
    );
}

#[test]
fn environment_expansion_supports_values_and_defaults() {
    std::env::set_var("TIMEM_MCP_TEST_VALUE", "available");
    assert_eq!(
        expand_env_text("${TIMEM_MCP_TEST_VALUE}/x").unwrap(),
        "available/x"
    );
    assert_eq!(
        expand_env_text("${TIMEM_MCP_MISSING:-fallback}").unwrap(),
        "fallback"
    );
    assert_eq!(
        expand_env_text("${TIMEM_MCP_MISSING}").unwrap_err(),
        "mcp_env_missing:TIMEM_MCP_MISSING"
    );
}

#[test]
fn http_parser_accepts_json_and_sse() {
    assert_eq!(parse_http_json_body("{\"result\":1}").unwrap()["result"], 1);
    assert_eq!(
        parse_http_json_body("event: message\ndata: {\"result\":2}\n\n").unwrap()["result"],
        2
    );
}

#[test]
fn curl_headers_split_before_sse_events_and_skips_interim_headers() {
    let raw = concat!(
        "HTTP/1.1 100 Continue\r\n\r\n",
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\n",
        "event: message\r\ndata: {\"result\":2}\r\n\r\n"
    );
    let (headers, body) = split_curl_headers(raw).unwrap();
    assert!(headers.starts_with("HTTP/1.1 200 OK"));
    assert_eq!(parse_http_json_body(body).unwrap()["result"], 2);
}

#[test]
fn legacy_sse_endpoint_resolution_handles_absolute_root_and_relative_urls() {
    assert_eq!(
        resolve_legacy_sse_endpoint("https://example.test/events", "https://other.test/messages")
            .unwrap(),
        "https://other.test/messages"
    );
    assert_eq!(
        resolve_legacy_sse_endpoint("https://example.test/events", "/messages?id=1").unwrap(),
        "https://example.test/messages?id=1"
    );
    assert_eq!(
        resolve_legacy_sse_endpoint("https://example.test/mcp/events", "messages?id=1").unwrap(),
        "https://example.test/mcp/messages?id=1"
    );
}

#[test]
fn call_requires_object_arguments() {
    let runtime = McpRuntime::default();
    let config = stdio_config(fake_server_script());
    runtime.connect(&config).unwrap();
    assert_eq!(
        runtime
            .call_tool(&config, "echo-value", &json!("wrong"))
            .unwrap_err(),
        "mcp_tool_arguments_must_be_object"
    );
}

#[test]
fn dynamic_tool_is_prompt_visible_and_protocol_parser_keeps_arguments_generic() {
    let tool = McpTool {
        server_id: "demo".to_string(),
        server_name: "Demo server".to_string(),
        name: "echo-value".to_string(),
        action_name: "mcp.demo.echo-value".to_string(),
        description: "Echo a value from MCP".to_string(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "value": {
                    "oneOf": [
                        {"type": "string"},
                        {"type": "integer"}
                    ]
                }
            },
            "required": ["value"],
            "additionalProperties": false
        }),
    };
    let registry = crate::capability::CapabilityRegistry::builtin()
        .with_mcp_tools(&[tool])
        .unwrap();
    assert!(registry
        .render_tool_catalog_markdown()
        .contains("mcp.demo.echo-value"));
    assert!(registry
        .render_mcp_tool_catalog_markdown_for_protocol("XML")
        .contains("mcp.demo.echo-value"));
    assert!(!registry
        .enrich_static_prompt("STATIC\n{{TOOL_CATALOG}}")
        .contains("mcp.demo.echo-value"));
    let builtin_tools = registry.native_builtin_tool_definitions();
    let dynamic_tools = registry.native_dynamic_tool_definitions();
    assert!(builtin_tools.iter().any(|tool| tool.name == "self_tool"));
    assert!(builtin_tools
        .iter()
        .all(|tool| !tool.name.starts_with("mcp.")));
    assert_eq!(
        dynamic_tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>(),
        vec!["mcp.demo.echo-value"]
    );
    assert_eq!(dynamic_tools[0].input_schema["additionalProperties"], false);
    assert_eq!(
        dynamic_tools[0].input_schema["properties"]["value"]["oneOf"]
            .as_array()
            .unwrap()
            .len(),
        2
    );

    let response = r#"<ASSISTANT><actions><mcp.demo.echo-value name="echo protocol-like value"><value><![CDATA[literal </ASSISTANT> and ```xml <ASSISTANT> text]]></value><nested><json>{"action":"not-a-call"}</json></nested></mcp.demo.echo-value></actions></ASSISTANT>"#;
    let parsed = crate::response_protocol::ResponseProtocolKind::Xml
        .suite()
        .parse(response, &registry);
    assert_eq!(parsed.repair_issue, None);
    assert_eq!(parsed.next_actions[0].action, "mcp.demo.echo-value");
    assert_eq!(
        parsed.next_actions[0].raw_input["value"],
        "literal </ASSISTANT> and ```xml <ASSISTANT> text"
    );
    assert_eq!(
        parsed.next_actions[0].raw_input["nested"]["json"],
        r#"{"action":"not-a-call"}"#
    );
    assert!(registry
        .validate_action_input("mcp.demo.echo-value", &json!({}))
        .is_ok());
}
