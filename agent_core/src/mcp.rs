use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const MCP_PROTOCOL_VERSION: &str = "2025-03-26";
const LEGACY_SSE_PROTOCOL_VERSION: &str = "2024-11-05";
const DEFAULT_REQUEST_TIMEOUT_MS: u64 = 30_000;
const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpServerConfig {
    pub id: String,
    pub name: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub transport: McpTransportConfig,
    #[serde(default = "default_request_timeout_ms")]
    pub request_timeout_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum McpTransportConfig {
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        env: BTreeMap<String, String>,
    },
    StreamableHttp {
        url: String,
        #[serde(default)]
        headers: BTreeMap<String, String>,
    },
    Sse {
        url: String,
        #[serde(default)]
        headers: BTreeMap<String, String>,
    },
}

impl Default for McpTransportConfig {
    fn default() -> Self {
        Self::Stdio {
            command: String::new(),
            args: Vec::new(),
            env: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpTool {
    pub server_id: String,
    pub server_name: String,
    pub name: String,
    pub action_name: String,
    pub description: String,
    pub input_schema: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpServerReport {
    pub config: McpServerConfig,
    pub state: String,
    pub error: Option<String>,
    pub tools: Vec<McpTool>,
}

#[derive(Debug, Clone)]
pub struct McpStore {
    file: PathBuf,
}

impl McpStore {
    pub fn new(memory_dir: impl AsRef<Path>) -> Self {
        Self {
            file: memory_dir.as_ref().join("mcp_servers.json"),
        }
    }

    pub fn file(&self) -> &Path {
        &self.file
    }

    pub fn list(&self) -> Result<Vec<McpServerConfig>, String> {
        if !self.file.exists() {
            return Ok(Vec::new());
        }
        let raw =
            fs::read_to_string(&self.file).map_err(|err| format!("mcp_store_read_failed:{err}"))?;
        serde_json::from_str(&raw).map_err(|err| format!("mcp_store_parse_failed:{err}"))
    }

    pub fn save(&self, configs: &[McpServerConfig]) -> Result<(), String> {
        if let Some(parent) = self.file.parent() {
            fs::create_dir_all(parent).map_err(|err| format!("mcp_store_dir_failed:{err}"))?;
        }
        let temporary = self.file.with_extension("json.tmp");
        let raw = serde_json::to_vec_pretty(configs)
            .map_err(|err| format!("mcp_store_serialize_failed:{err}"))?;
        let mut options = OpenOptions::new();
        options.create(true).truncate(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&temporary)
            .map_err(|err| format!("mcp_store_open_failed:{err}"))?;
        file.write_all(&raw)
            .and_then(|_| file.sync_all())
            .map_err(|err| format!("mcp_store_write_failed:{err}"))?;
        fs::rename(&temporary, &self.file).map_err(|err| format!("mcp_store_replace_failed:{err}"))
    }
}

#[derive(Clone, Default)]
pub struct McpRuntime {
    inner: Arc<Mutex<BTreeMap<String, Arc<Mutex<McpConnection>>>>>,
}

impl std::fmt::Debug for McpRuntime {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let count = self
            .inner
            .lock()
            .map(|items| items.len())
            .unwrap_or_default();
        formatter
            .debug_struct("McpRuntime")
            .field("connections", &count)
            .finish()
    }
}

impl McpRuntime {
    pub fn disconnect(&self, server_id: &str) {
        if let Ok(mut connections) = self.inner.lock() {
            connections.remove(server_id);
        }
    }

    pub fn disconnect_all(&self) {
        if let Ok(mut connections) = self.inner.lock() {
            connections.clear();
        }
    }

    pub fn connect(&self, config: &McpServerConfig) -> Result<Vec<McpTool>, String> {
        validate_server_config(config)?;
        let mut connection = McpConnection::open(config.clone())?;
        connection.initialize()?;
        let tools = connection.list_tools()?;
        self.inner
            .lock()
            .map_err(|_| "mcp_runtime_poisoned".to_string())?
            .insert(config.id.clone(), Arc::new(Mutex::new(connection)));
        Ok(tools)
    }

    pub fn list_tools(&self, config: &McpServerConfig) -> Result<Vec<McpTool>, String> {
        let connection = self
            .inner
            .lock()
            .map_err(|_| "mcp_runtime_poisoned".to_string())?
            .get(&config.id)
            .cloned();
        let Some(connection) = connection else {
            return self.connect(config);
        };
        let mut connection = connection
            .lock()
            .map_err(|_| "mcp_connection_poisoned".to_string())?;
        connection.list_tools()
    }

    pub fn call_tool(
        &self,
        config: &McpServerConfig,
        tool_name: &str,
        args: &Value,
    ) -> Result<String, String> {
        let connection = self.connection(config)?;
        let result = connection
            .lock()
            .map_err(|_| "mcp_connection_poisoned".to_string())?
            .call_tool(tool_name, args);
        match result {
            Ok(result) => Ok(render_call_result(&config.name, tool_name, &result)),
            Err(error) => {
                // A failed transport may still deliver a late response for this
                // request, so it must not be reused by a later tool call.
                self.disconnect(&config.id);
                Err(error)
            }
        }
    }

    fn connection(&self, config: &McpServerConfig) -> Result<Arc<Mutex<McpConnection>>, String> {
        if let Some(connection) = self
            .inner
            .lock()
            .map_err(|_| "mcp_runtime_poisoned".to_string())?
            .get(&config.id)
            .cloned()
        {
            let matches_config = connection
                .lock()
                .map_err(|_| "mcp_connection_poisoned".to_string())?
                .config()
                == config;
            if matches_config {
                return Ok(connection);
            }
        }
        self.connect(config)?;
        self.inner
            .lock()
            .map_err(|_| "mcp_runtime_poisoned".to_string())?
            .get(&config.id)
            .cloned()
            .ok_or_else(|| "mcp_connection_missing_after_connect".to_string())
    }
}

enum McpConnection {
    Stdio(StdioConnection),
    Http(HttpConnection),
    Sse(SseConnection),
}

impl McpConnection {
    fn open(config: McpServerConfig) -> Result<Self, String> {
        match &config.transport {
            McpTransportConfig::Stdio { command, args, env } => Ok(Self::Stdio(
                StdioConnection::spawn(config.clone(), command, args, env)?,
            )),
            McpTransportConfig::StreamableHttp { url, headers } => Ok(Self::Http(HttpConnection {
                config: config.clone(),
                url: expand_env_text(url)?,
                headers: expand_env_map(headers)?,
                session_id: None,
                next_id: 1,
            })),
            McpTransportConfig::Sse { url, headers } => Ok(Self::Sse(SseConnection::open(
                config.clone(),
                &expand_env_text(url)?,
                &expand_env_map(headers)?,
            )?)),
        }
    }

    fn initialize(&mut self) -> Result<(), String> {
        let result = self.request(
            "initialize",
            json!({
                "protocolVersion": self.protocol_version(),
                "capabilities": {},
                "clientInfo": { "name": "TimemAi", "version": env!("CARGO_PKG_VERSION") }
            }),
        )?;
        if result
            .get("protocolVersion")
            .and_then(Value::as_str)
            .is_none()
        {
            return Err("mcp_initialize_missing_protocol_version".to_string());
        }
        self.notify("notifications/initialized", json!({}))
    }

    fn list_tools(&mut self) -> Result<Vec<McpTool>, String> {
        let config = self.config().clone();
        let mut cursor: Option<String> = None;
        let mut tools = Vec::new();
        let mut action_names = BTreeSet::new();
        loop {
            let params = cursor
                .as_ref()
                .map(|value| json!({ "cursor": value }))
                .unwrap_or_else(|| json!({}));
            let result = self.request("tools/list", params)?;
            let entries = result
                .get("tools")
                .and_then(Value::as_array)
                .ok_or_else(|| "mcp_tools_list_missing_tools".to_string())?;
            for entry in entries {
                let name = entry
                    .get("name")
                    .and_then(Value::as_str)
                    .filter(|value| !value.trim().is_empty())
                    .ok_or_else(|| "mcp_tool_name_required".to_string())?;
                let description = entry
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or("MCP tool")
                    .to_string();
                let input_schema = entry
                    .get("inputSchema")
                    .cloned()
                    .unwrap_or_else(|| json!({ "type": "object", "properties": {} }));
                let action_name = mcp_action_name(&config.id, name);
                if action_name.ends_with('.') || !action_names.insert(action_name.clone()) {
                    return Err(format!("mcp_tool_action_name_collision:{action_name}"));
                }
                tools.push(McpTool {
                    server_id: config.id.clone(),
                    server_name: config.name.clone(),
                    name: name.to_string(),
                    action_name,
                    description,
                    input_schema,
                });
            }
            cursor = result
                .get("nextCursor")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(str::to_string);
            if cursor.is_none() {
                break;
            }
        }
        Ok(tools)
    }

    fn call_tool(&mut self, tool_name: &str, args: &Value) -> Result<Value, String> {
        let arguments = args
            .as_object()
            .cloned()
            .ok_or_else(|| "mcp_tool_arguments_must_be_object".to_string())?;
        self.request(
            "tools/call",
            json!({ "name": tool_name, "arguments": arguments }),
        )
    }

    fn config(&self) -> &McpServerConfig {
        match self {
            Self::Stdio(connection) => &connection.config,
            Self::Http(connection) => &connection.config,
            Self::Sse(connection) => &connection.config,
        }
    }

    fn protocol_version(&self) -> &'static str {
        match self {
            Self::Sse(_) => LEGACY_SSE_PROTOCOL_VERSION,
            Self::Stdio(_) | Self::Http(_) => MCP_PROTOCOL_VERSION,
        }
    }

    fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        match self {
            Self::Stdio(connection) => connection.request(method, params),
            Self::Http(connection) => connection.request(method, params),
            Self::Sse(connection) => connection.request(method, params),
        }
    }

    fn notify(&mut self, method: &str, params: Value) -> Result<(), String> {
        match self {
            Self::Stdio(connection) => connection.notify(method, params),
            Self::Http(connection) => connection.notify(method, params),
            Self::Sse(connection) => connection.notify(method, params),
        }
    }
}

struct StdioConnection {
    config: McpServerConfig,
    child: Child,
    stdin: ChildStdin,
    messages: Receiver<Value>,
    next_id: u64,
}

impl StdioConnection {
    fn spawn(
        config: McpServerConfig,
        command: &str,
        args: &[String],
        env: &BTreeMap<String, String>,
    ) -> Result<Self, String> {
        let command = expand_env_text(command)?;
        let args = args
            .iter()
            .map(|value| expand_env_text(value))
            .collect::<Result<Vec<_>, _>>()?;
        let env = expand_env_map(env)?;
        let mut child = Command::new(&command)
            .args(&args)
            .envs(env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|err| format!("mcp_stdio_spawn_failed:{err}"))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "mcp_stdio_stdin_missing".to_string())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "mcp_stdio_stdout_missing".to_string())?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| "mcp_stdio_stderr_missing".to_string())?;
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if line.len() > MAX_RESPONSE_BYTES {
                    continue;
                }
                if let Ok(value) = serde_json::from_str::<Value>(&line) {
                    if tx.send(value).is_err() {
                        break;
                    }
                }
            }
        });
        thread::spawn(move || {
            let mut reader = BufReader::new(stderr);
            let mut sink = [0_u8; 4096];
            while let Ok(count) = reader.read(&mut sink) {
                if count == 0 {
                    break;
                }
            }
        });
        Ok(Self {
            config,
            child,
            stdin,
            messages: rx,
            next_id: 1,
        })
    }

    fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))?;
        let timeout = Duration::from_millis(self.config.request_timeout_ms.max(1));
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .ok_or_else(|| "mcp_request_timeout".to_string())?;
            let message = self
                .messages
                .recv_timeout(remaining)
                .map_err(|error| match error {
                    mpsc::RecvTimeoutError::Timeout => "mcp_request_timeout".to_string(),
                    mpsc::RecvTimeoutError::Disconnected => "mcp_server_disconnected".to_string(),
                })?;
            if message.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            return json_rpc_result(message);
        }
    }

    fn notify(&mut self, method: &str, params: Value) -> Result<(), String> {
        self.send(&json!({ "jsonrpc": "2.0", "method": method, "params": params }))
    }

    fn send(&mut self, value: &Value) -> Result<(), String> {
        serde_json::to_writer(&mut self.stdin, value)
            .map_err(|err| format!("mcp_stdio_write_failed:{err}"))?;
        self.stdin
            .write_all(b"\n")
            .and_then(|_| self.stdin.flush())
            .map_err(|err| format!("mcp_stdio_write_failed:{err}"))
    }
}

impl Drop for StdioConnection {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct HttpConnection {
    config: McpServerConfig,
    url: String,
    headers: BTreeMap<String, String>,
    session_id: Option<String>,
    next_id: u64,
}

impl HttpConnection {
    fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        let response =
            self.post(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))?;
        json_rpc_result(response)
    }

    fn notify(&mut self, method: &str, params: Value) -> Result<(), String> {
        let _ = self.post(&json!({ "jsonrpc": "2.0", "method": method, "params": params }))?;
        Ok(())
    }

    fn post(&mut self, payload: &Value) -> Result<Value, String> {
        let mut command = Command::new("curl");
        command
            .arg("--silent")
            .arg("--show-error")
            .arg("--fail-with-body")
            .arg("--max-time")
            .arg((self.config.request_timeout_ms.max(1) as f64 / 1000.0).to_string())
            .arg("--dump-header")
            .arg("-")
            .arg("--request")
            .arg("POST")
            .arg("--header")
            .arg("Content-Type: application/json")
            .arg("--header")
            .arg("Accept: application/json, text/event-stream")
            .arg("--header")
            .arg(format!("MCP-Protocol-Version: {MCP_PROTOCOL_VERSION}"));
        if let Some(session_id) = &self.session_id {
            command
                .arg("--header")
                .arg(format!("Mcp-Session-Id: {session_id}"));
        }
        for (name, value) in &self.headers {
            command.arg("--header").arg(format!("{name}: {value}"));
        }
        let mut child = command
            .arg("--data-binary")
            .arg("@-")
            .arg(&self.url)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|err| format!("mcp_http_spawn_failed:{err}"))?;
        if let Some(mut stdin) = child.stdin.take() {
            serde_json::to_writer(&mut stdin, payload)
                .map_err(|err| format!("mcp_http_write_failed:{err}"))?;
        }
        let output = child
            .wait_with_output()
            .map_err(|err| format!("mcp_http_wait_failed:{err}"))?;
        if !output.status.success() {
            return Err(format!(
                "mcp_http_request_failed:{}",
                bounded_text(&String::from_utf8_lossy(&output.stderr), 2000)
            ));
        }
        if output.stdout.len() > MAX_RESPONSE_BYTES {
            return Err("mcp_http_response_too_large".to_string());
        }
        let raw = String::from_utf8_lossy(&output.stdout);
        let (headers, body) = split_curl_headers(&raw)?;
        if let Some(value) = header_value(headers, "mcp-session-id") {
            self.session_id = Some(value.to_string());
        }
        if body.trim().is_empty() {
            Ok(Value::Null)
        } else {
            parse_http_json_body(body)
        }
    }
}

enum SseInbound {
    Endpoint(String),
    Message(Value),
}

struct SseConnection {
    config: McpServerConfig,
    child: Child,
    endpoint: String,
    headers: BTreeMap<String, String>,
    messages: Receiver<SseInbound>,
    next_id: u64,
}

impl SseConnection {
    fn open(
        config: McpServerConfig,
        url: &str,
        headers: &BTreeMap<String, String>,
    ) -> Result<Self, String> {
        let mut command = Command::new("curl");
        command
            .arg("--silent")
            .arg("--show-error")
            .arg("--fail-with-body")
            .arg("--no-buffer")
            .arg("--header")
            .arg("Accept: text/event-stream");
        for (name, value) in headers {
            command.arg("--header").arg(format!("{name}: {value}"));
        }
        let mut child = command
            .arg(url)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|err| format!("mcp_sse_spawn_failed:{err}"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "mcp_sse_stdout_missing".to_string())?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| "mcp_sse_stderr_missing".to_string())?;
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || read_sse_stream(stdout, tx));
        thread::spawn(move || {
            let mut reader = BufReader::new(stderr);
            let mut sink = [0_u8; 4096];
            while let Ok(count) = reader.read(&mut sink) {
                if count == 0 {
                    break;
                }
            }
        });

        let deadline = Instant::now() + Duration::from_millis(config.request_timeout_ms.max(1));
        let endpoint = loop {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .ok_or_else(|| "mcp_sse_endpoint_timeout".to_string())?;
            match rx.recv_timeout(remaining) {
                Ok(SseInbound::Endpoint(endpoint)) => {
                    break resolve_legacy_sse_endpoint(url, &endpoint)?;
                }
                Ok(SseInbound::Message(_)) => continue,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    return Err("mcp_sse_endpoint_timeout".to_string())
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err("mcp_sse_stream_disconnected".to_string())
                }
            }
        };
        Ok(Self {
            config,
            child,
            endpoint,
            headers: headers.clone(),
            messages: rx,
            next_id: 1,
        })
    }

    fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        self.post(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))?;
        let deadline =
            Instant::now() + Duration::from_millis(self.config.request_timeout_ms.max(1));
        loop {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .ok_or_else(|| "mcp_request_timeout".to_string())?;
            match self.messages.recv_timeout(remaining) {
                Ok(SseInbound::Message(message))
                    if message.get("id").and_then(Value::as_u64) == Some(id) =>
                {
                    return json_rpc_result(message)
                }
                Ok(_) => continue,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    return Err("mcp_request_timeout".to_string())
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err("mcp_sse_stream_disconnected".to_string())
                }
            }
        }
    }

    fn notify(&mut self, method: &str, params: Value) -> Result<(), String> {
        self.post(&json!({ "jsonrpc": "2.0", "method": method, "params": params }))
    }

    fn post(&self, payload: &Value) -> Result<(), String> {
        let mut command = Command::new("curl");
        command
            .arg("--silent")
            .arg("--show-error")
            .arg("--fail-with-body")
            .arg("--max-time")
            .arg((self.config.request_timeout_ms.max(1) as f64 / 1000.0).to_string())
            .arg("--request")
            .arg("POST")
            .arg("--header")
            .arg("Content-Type: application/json");
        for (name, value) in &self.headers {
            command.arg("--header").arg(format!("{name}: {value}"));
        }
        let mut child = command
            .arg("--data-binary")
            .arg("@-")
            .arg(&self.endpoint)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|err| format!("mcp_sse_post_spawn_failed:{err}"))?;
        if let Some(mut stdin) = child.stdin.take() {
            serde_json::to_writer(&mut stdin, payload)
                .map_err(|err| format!("mcp_sse_post_write_failed:{err}"))?;
        }
        let output = child
            .wait_with_output()
            .map_err(|err| format!("mcp_sse_post_wait_failed:{err}"))?;
        if output.status.success() {
            Ok(())
        } else {
            Err(format!(
                "mcp_sse_post_failed:{}",
                bounded_text(&String::from_utf8_lossy(&output.stderr), 2000)
            ))
        }
    }
}

impl Drop for SseConnection {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn read_sse_stream(reader: impl Read, sender: mpsc::Sender<SseInbound>) {
    let mut event = String::new();
    let mut data = Vec::new();
    for line in BufReader::new(reader).lines() {
        let Ok(line) = line else { break };
        if line.is_empty() {
            if !data.is_empty() {
                let payload = data.join("\n");
                let inbound = if event == "endpoint" {
                    Some(SseInbound::Endpoint(payload.trim().to_string()))
                } else {
                    serde_json::from_str(&payload).ok().map(SseInbound::Message)
                };
                if inbound.is_some_and(|value| sender.send(value).is_err()) {
                    break;
                }
            }
            event.clear();
            data.clear();
            continue;
        }
        if let Some(value) = line.strip_prefix("event:") {
            event = value.trim().to_string();
        } else if let Some(value) = line.strip_prefix("data:") {
            data.push(value.trim_start().to_string());
        }
    }
}

fn resolve_legacy_sse_endpoint(base: &str, endpoint: &str) -> Result<String, String> {
    if endpoint.starts_with("http://") || endpoint.starts_with("https://") {
        return Ok(endpoint.to_string());
    }
    let (scheme, rest) = base
        .split_once("://")
        .ok_or_else(|| "mcp_sse_base_url_invalid".to_string())?;
    let authority = rest.split('/').next().unwrap_or(rest);
    if endpoint.starts_with('/') {
        return Ok(format!("{scheme}://{authority}{endpoint}"));
    }
    let base_dir = base.rsplit_once('/').map(|(dir, _)| dir).unwrap_or(base);
    Ok(format!("{base_dir}/{endpoint}"))
}

pub fn mcp_action_name(server_id: &str, tool_name: &str) -> String {
    format!(
        "mcp.{}.{}",
        action_component(server_id),
        action_component(tool_name)
    )
}

fn action_component(value: &str) -> String {
    let mut out = String::new();
    for character in value.chars() {
        if character.is_ascii_alphanumeric() || character == '_' || character == '-' {
            out.push(character.to_ascii_lowercase());
        } else {
            out.push('_');
        }
    }
    out.trim_matches('_').to_string()
}

fn validate_server_config(config: &McpServerConfig) -> Result<(), String> {
    if config.id.trim().is_empty() || action_component(&config.id).is_empty() {
        return Err("mcp_server_id_required".to_string());
    }
    if config.id != action_component(&config.id) {
        return Err("mcp_server_id_must_be_canonical".to_string());
    }
    if config.name.trim().is_empty() {
        return Err("mcp_server_name_required".to_string());
    }
    match &config.transport {
        McpTransportConfig::Stdio { command, args, env } => {
            if command.trim().is_empty() {
                return Err("mcp_stdio_command_required".to_string());
            }
            if command.contains('\0') || args.iter().any(|arg| arg.contains('\0')) {
                return Err("mcp_stdio_argument_contains_nul".to_string());
            }
            for (key, value) in env {
                if !valid_env_name(key) {
                    return Err(format!("mcp_stdio_env_name_invalid:{key}"));
                }
                if value.contains('\0') {
                    return Err(format!("mcp_stdio_env_value_contains_nul:{key}"));
                }
            }
            Ok(())
        }
        McpTransportConfig::StreamableHttp { url, headers }
        | McpTransportConfig::Sse { url, headers } => {
            if !(url.starts_with("http://") || url.starts_with("https://")) {
                return Err("mcp_http_url_must_be_http_or_https".to_string());
            }
            if url.chars().any(char::is_control) {
                return Err("mcp_http_url_contains_control_character".to_string());
            }
            for (name, value) in headers {
                if name.is_empty()
                    || !name.bytes().all(|byte| {
                        byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte)
                    })
                {
                    return Err(format!("mcp_http_header_name_invalid:{name}"));
                }
                if value.contains(['\r', '\n', '\0']) {
                    return Err(format!("mcp_http_header_value_invalid:{name}"));
                }
            }
            Ok(())
        }
    }
}

fn valid_env_name(value: &str) -> bool {
    let mut chars = value.chars();
    matches!(chars.next(), Some(first) if first == '_' || first.is_ascii_alphabetic())
        && chars.all(|character| character == '_' || character.is_ascii_alphanumeric())
}

fn default_true() -> bool {
    true
}
fn default_request_timeout_ms() -> u64 {
    DEFAULT_REQUEST_TIMEOUT_MS
}

fn expand_env_map(values: &BTreeMap<String, String>) -> Result<BTreeMap<String, String>, String> {
    values
        .iter()
        .map(|(key, value)| Ok((key.clone(), expand_env_text(value)?)))
        .collect()
}

fn expand_env_text(value: &str) -> Result<String, String> {
    let mut out = String::new();
    let chars = value.chars().collect::<Vec<_>>();
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == '$' && chars.get(index + 1) == Some(&'{') {
            let Some(relative_end) = chars[index + 2..]
                .iter()
                .position(|character| *character == '}')
            else {
                return Err("mcp_env_expansion_unclosed".to_string());
            };
            let end = index + 2 + relative_end;
            let expression = chars[index + 2..end].iter().collect::<String>();
            let (key, fallback) = expression
                .split_once(":-")
                .map(|(key, value)| (key, Some(value)))
                .unwrap_or((&expression, None));
            if !key
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || character == '_')
            {
                return Err(format!("mcp_env_name_invalid:{key}"));
            }
            let expanded = std::env::var(key)
                .ok()
                .filter(|value| !value.is_empty())
                .or_else(|| fallback.map(str::to_string))
                .ok_or_else(|| format!("mcp_env_missing:{key}"))?;
            out.push_str(&expanded);
            index = end + 1;
            continue;
        }
        out.push(chars[index]);
        index += 1;
    }
    Ok(out)
}

fn json_rpc_result(message: Value) -> Result<Value, String> {
    if let Some(error) = message.get("error") {
        let code = error
            .get("code")
            .and_then(Value::as_i64)
            .unwrap_or_default();
        let text = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("MCP server error");
        return Err(format!(
            "mcp_server_error:{code}:{}",
            bounded_text(text, 2000)
        ));
    }
    message
        .get("result")
        .cloned()
        .ok_or_else(|| "mcp_response_missing_result".to_string())
}

fn render_call_result(server: &str, tool: &str, result: &Value) -> String {
    let mut lines = vec![format!("Action result: MCP {server}/{tool}")];
    if result.get("isError").and_then(Value::as_bool) == Some(true) {
        lines.push("status: tool_error".to_string());
    } else {
        lines.push("status: completed".to_string());
    }
    if let Some(content) = result.get("content").and_then(Value::as_array) {
        for item in content {
            match item.get("type").and_then(Value::as_str) {
                Some("text") => {
                    if let Some(text) = item.get("text").and_then(Value::as_str) {
                        lines.push(bounded_text(text, 32_000));
                    }
                }
                Some(kind) => lines.push(format!(
                    "[{kind} content: {}]",
                    bounded_text(&item.to_string(), 4000)
                )),
                None => lines.push(bounded_text(&item.to_string(), 4000)),
            }
        }
    } else if let Some(structured) = result.get("structuredContent") {
        lines.push(bounded_text(&structured.to_string(), 32_000));
    } else {
        lines.push(bounded_text(&result.to_string(), 32_000));
    }
    lines.join("\n")
}

fn split_curl_headers(raw: &str) -> Result<(&str, &str), String> {
    let mut remaining = raw;
    loop {
        let (headers, body) = remaining
            .split_once("\r\n\r\n")
            .or_else(|| remaining.split_once("\n\n"))
            .ok_or_else(|| "mcp_http_response_headers_missing".to_string())?;
        if body.starts_with("HTTP/") {
            remaining = body;
            continue;
        }
        return Ok((headers, body));
    }
}

fn header_value<'a>(headers: &'a str, expected: &str) -> Option<&'a str> {
    headers.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.trim()
            .eq_ignore_ascii_case(expected)
            .then(|| value.trim())
    })
}

fn parse_http_json_body(body: &str) -> Result<Value, String> {
    let trimmed = body.trim();
    if trimmed.starts_with('{') {
        return serde_json::from_str(trimmed).map_err(|err| format!("mcp_http_invalid_json:{err}"));
    }
    let mut last = None;
    for line in trimmed.lines() {
        if let Some(data) = line.strip_prefix("data:") {
            let data = data.trim();
            if data != "[DONE]" {
                last = Some(
                    serde_json::from_str(data)
                        .map_err(|err| format!("mcp_http_invalid_sse_json:{err}"))?,
                );
            }
        }
    }
    last.ok_or_else(|| "mcp_http_response_body_missing".to_string())
}

fn bounded_text(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut result = text.chars().take(max_chars).collect::<String>();
    result.push('…');
    result
}

#[cfg(test)]
#[path = "../tests/unit/mcp_tests.rs"]
mod tests;
