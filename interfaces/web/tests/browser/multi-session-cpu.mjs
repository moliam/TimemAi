import { createHash } from "node:crypto";
import { existsSync } from "node:fs";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { createServer } from "node:http";
import { spawn } from "node:child_process";
import { tmpdir } from "node:os";
import { extname, join, resolve } from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { fileURLToPath } from "node:url";

const here = fileURLToPath(new URL(".", import.meta.url));
const args = process.argv.slice(2);
const arg = (name, fallback) => {
  const index = args.indexOf(`--${name}`);
  return index >= 0 ? args[index + 1] : fallback;
};
const dist = resolve(arg("dist", resolve(here, "../../dist")));
const label = arg("label", "run");
const sessionCount = Number(arg("sessions", "4"));
const seconds = Number(arg("seconds", "8"));
const intervalMs = Number(arg("interval-ms", "25"));
const turnsPerSession = Number(arg("turns", "60"));
const chromeCandidates = [
  process.env.CHROME_BIN,
  "/usr/bin/google-chrome",
  "/usr/bin/google-chrome-stable",
  "/usr/bin/chromium",
  "/usr/bin/chromium-browser",
].filter(Boolean);
const chrome = chromeCandidates.find((candidate) => existsSync(candidate));
if (!chrome) throw new Error("Chrome/Chromium not found");

const waitFor = async (check, message, timeout = 15000) => {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    try {
      if (await check()) return;
    } catch {}
    await sleep(40);
  }
  throw new Error(typeof message === "function" ? await message() : message);
};

const now = Date.now();
const worker = (index) => ({
  worker_id: `worker-${index}`,
  context_id: `context-${index}`,
  display_name: `Worker ${index}`,
  ordinal: 0,
  state: "working",
  parent_worker_id: null,
});
const markdown = (index, turn) =>
  `## Result ${index}-${turn}\n\n` +
  `Paragraph ${turn} with **bold**, \`inline code\`, and a list:\n\n` +
  Array.from({ length: 8 }, (_, line) => `- item ${line} for session ${index} turn ${turn}`).join("\n") +
  `\n\n\`\`\`bash\necho session-${index}-turn-${turn}\n\`\`\`\n`;
const makeSession = (index) => {
  const turns = Array.from({ length: turnsPerSession }, (_, turnIndex) => {
    const working = turnIndex === turnsPerSession - 1;
    const createdAt = now - (turnsPerSession - turnIndex) * 60_000;
    return {
      turn_id: `turn-${index}-${turnIndex}`,
      state: working ? "working" : "finished",
      created_at_ms: createdAt,
      user_entries: [
        { kind: "task", text: `Task ${turnIndex} for session ${index}`, created_at_ms: createdAt },
      ],
      events: [],
      final_answer: working ? null : markdown(index, turnIndex),
      completion: working ? null : { stop_reason: "Completed" },
    };
  });
  return {
    session_id: `session-${index}`,
    display_name: `Session ${index}`,
    ordinal: index,
    state: "working",
    current_dir: "/work",
    max_llm_input_tokens: 100000,
    tools: [],
    mcp_server_ids: [],
    contexts: [{ context_id: `context-${index}`, current_dir: "/work", worker_ids: [`worker-${index}`] }],
    workers: [worker(index)],
    active_context_id: `context-${index}`,
    primary_worker_id: `worker-${index}`,
    attachments: [],
    roles: [],
    messages: [],
    turns,
    history_before_cursor: null,
    history_has_more: false,
    active_turn_id: `turn-${index}-${turnsPerSession - 1}`,
    cancelling_turn_id: null,
    pending_turn_id: null,
    message_queue: {
      revision: 0,
      items: [],
      auto_send_enabled: true,
      continuation: { state: "awaiting_normal_completion" },
      dispatching_command_id: null,
    },
  };
};
const sessions = Array.from({ length: sessionCount }, (_, index) => makeSession(index));
const snapshot = {
  server: {
    version: "cpu-benchmark",
    protocol_version: 1,
    port: 0,
    bind_host: "127.0.0.1",
    public_access: false,
    debug_mode: false,
    performance_trace: false,
    mem: {
      space: "cpu-benchmark",
      data_dir: "/tmp",
      space_dir: "/tmp/timem-cpu-benchmark",
      memory_dir: "/tmp/timem-cpu-benchmark/memory",
      temporary_retention_days: 5,
      temporary_capacity_bytes: null,
      conversation_capacity_bytes: null,
      mailbox_capacity_bytes: null,
      claude_codex_tool_discovery: false,
      model_tool_result_bytes: 16384,
      context_compress_threshold_percent: 90,
      restore_prompt_context_on_startup: false,
    },
    runtime_options: [],
    session_env_defaults: {},
    workspace_dirs: ["/work"],
    mcp_servers: [],
    model_endpoints: [
      {
        id: "endpoint-1",
        name: "Benchmark endpoint",
        model: "benchmark-model",
        api_protocol: "openai-compatible",
        response_protocol: "xml",
        base_url: "http://127.0.0.1/model",
        max_llm_input_tokens: 100000,
        max_llm_output_tokens: 4096,
        stream: true,
        api_key_configured: true,
        http_headers: {},
      },
    ],
  },
  sessions,
  role_library: { roles: [], groups: [] },
  session_groups: [],
};

function encodeFrame(event) {
  const payload = Buffer.from(JSON.stringify(event));
  if (payload.length < 126) return Buffer.concat([Buffer.from([0x81, payload.length]), payload]);
  const header = Buffer.alloc(4);
  header[0] = 0x81;
  header[1] = 126;
  header.writeUInt16BE(payload.length, 2);
  return Buffer.concat([header, payload]);
}
function makePeer(socket) {
  let buffer = Buffer.alloc(0);
  socket.on("data", (chunk) => {
    buffer = Buffer.concat([buffer, chunk]);
    while (buffer.length >= 2) {
      const opcode = buffer[0] & 0x0f;
      let length = buffer[1] & 0x7f;
      let offset = 2;
      if (length === 126) {
        if (buffer.length < 4) return;
        length = buffer.readUInt16BE(2);
        offset = 4;
      } else if (length === 127) {
        if (buffer.length < 10) return;
        length = Number(buffer.readBigUInt64BE(2));
        offset = 10;
      }
      const masked = Boolean(buffer[1] & 0x80);
      const mask = masked ? buffer.subarray(offset, offset + 4) : null;
      if (masked) offset += 4;
      if (buffer.length < offset + length) return;
      const payload = Buffer.from(buffer.subarray(offset, offset + length));
      buffer = buffer.subarray(offset + length);
      if (mask) for (let i = 0; i < payload.length; i += 1) payload[i] ^= mask[i % 4];
      if (opcode === 0x8) {
        socket.end();
        return;
      }
    }
  });
  return { send: (event) => socket.write(encodeFrame(event)), close: () => socket.destroy() };
}

let peers = new Set();
const server = createServer(async (request, response) => {
  try {
    const pathname = new URL(request.url, "http://localhost").pathname;
    const file = resolve(dist, pathname === "/" ? "index.html" : pathname.slice(1));
    if (!file.startsWith(resolve(dist))) throw new Error("unsafe");
    const body = await readFile(file);
    const mime = {
      ".html": "text/html; charset=utf-8",
      ".js": "text/javascript; charset=utf-8",
      ".css": "text/css; charset=utf-8",
      ".woff": "font/woff",
      ".woff2": "font/woff2",
      ".ttf": "font/ttf",
      ".svg": "image/svg+xml",
      ".png": "image/png",
    };
    response.writeHead(200, { "content-type": mime[extname(file)] ?? "application/octet-stream" });
    response.end(body);
  } catch {
    response.writeHead(404);
    response.end("not found");
  }
});
server.on("upgrade", (request, socket) => {
  const accept = createHash("sha1")
    .update(`${request.headers["sec-websocket-key"]}258EAFA5-E914-47DA-95CA-C5AB0DC85B11`)
    .digest("base64");
  socket.write(
    "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n" +
      `Sec-WebSocket-Accept: ${accept}\r\n\r\n`,
  );
  const peer = makePeer(socket);
  peers.add(peer);
  socket.on("error", () => peers.delete(peer));
  socket.on("close", () => peers.delete(peer));
  peer.send({
    type: "hello",
    snapshot,
    event_cursor: 40,
    event_replay_floor: 0,
  });
});
await new Promise((resolveListen) => server.listen(0, "127.0.0.1", resolveListen));
const url = `http://127.0.0.1:${server.address().port}/`;

async function readDevToolsPort(profile) {
  try {
    const [portLine] = (await readFile(join(profile, "DevToolsActivePort"), "utf8")).trim().split("\n");
    const port = Number(portLine);
    return Number.isInteger(port) && port > 0 ? port : null;
  } catch {
    return null;
  }
}
const profile = await mkdtemp(join(tmpdir(), "timem-cpu-bench-"));
const child = spawn(chrome, [
  "--remote-debugging-port=0",
  `--user-data-dir=${profile}`,
  "--headless=new",
  "--no-sandbox",
  "--disable-dev-shm-usage",
  "--no-first-run",
  "--no-default-browser-check",
  "--disable-background-networking",
  "--disable-component-update",
  "--disable-sync",
  "--disable-background-timer-throttling",
  "--disable-renderer-backgrounding",
  "--disable-backgrounding-occluded-windows",
  "--no-proxy-server",
  "--window-size=1440,1000",
  "about:blank",
], { stdio: ["ignore", "ignore", "pipe"] });
let socket;
let browser;
try {
  let port = null;
  await waitFor(async () => {
    port = await readDevToolsPort(profile);
    return port !== null;
  }, "Chrome DevTools did not start", 30000);
  let target = null;
  await waitFor(async () => {
    const targets = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
    target = targets.find((item) => item.type === "page") ?? null;
    return target !== null;
  }, "benchmark page target did not appear", 20000);
  socket = new WebSocket(target.webSocketDebuggerUrl);
  await new Promise((resolveOpen, rejectOpen) => {
    socket.addEventListener("open", resolveOpen, { once: true });
    socket.addEventListener("error", rejectOpen, { once: true });
  });
  let sequence = 0;
  const requests = new Map();
  const pageErrors = [];
  socket.addEventListener("message", ({ data }) => {
    const message = JSON.parse(String(data));
    if (message.method === "Runtime.exceptionThrown") {
      pageErrors.push(message.params?.exceptionDetails?.text ?? "exception");
    } else if (message.method === "Log.entryAdded" && message.params?.entry?.level === "error") {
      pageErrors.push(message.params.entry.text);
    }
    if (!message.id || !requests.has(message.id)) return;
    const { resolve: resolveRequest, reject: rejectRequest } = requests.get(message.id);
    requests.delete(message.id);
    if (message.error) rejectRequest(new Error(message.error.message));
    else resolveRequest(message.result);
  });
  const call = (method, params = {}) =>
    new Promise((resolveRequest, rejectRequest) => {
      const id = ++sequence;
      requests.set(id, { resolve: resolveRequest, reject: rejectRequest });
      socket.send(JSON.stringify({ id, method, params }));
    });
  const evaluate = async (expression) => {
    const result = await call("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
    if (result.exceptionDetails) throw new Error(result.exceptionDetails.text);
    return result.result.value;
  };
  browser = { call, evaluate, async close() { socket.close(); } };
  await call("Runtime.enable");
  await call("Page.enable");
  await call("Log.enable");
  await call("Performance.enable");
  const navigation = await call("Page.navigate", { url });
  console.error(`benchmark navigation: ${JSON.stringify(navigation)}`);
  await waitFor(
    async () => (await evaluate("location.href")) === url && (await evaluate("document.readyState")) === "complete",
    async () => `benchmark page did not finish loading: ${JSON.stringify({ pageErrors, state: await evaluate(`({ href: location.href, ready: document.readyState, body: document.body?.innerText.slice(0, 600), root: document.querySelector("#root")?.innerHTML.slice(0, 400) })`) })}`,
    20000,
  );
  await waitFor(
    () => evaluate(`document.querySelectorAll(".session-row").length === ${sessionCount}`),
    async () =>
      `session sidebar did not render the benchmark sessions: ${JSON.stringify(
        await evaluate(`({
          rows: document.querySelectorAll(".session-row").length,
          href: location.href,
          ready: document.readyState,
          body: document.body.innerText.slice(0, 1200),
          root: document.querySelector("#root")?.innerHTML.slice(0, 800),
        })`),
      )}`,
    20000,
  );
  await waitFor(() => evaluate(`Boolean(document.querySelector(".chat-scroll"))`), "chat did not render");
  await evaluate(`window.__benchLongTasks = 0; new PerformanceObserver((list) => { window.__benchLongTasks += list.getEntries().length; }).observe({ entryTypes: ["longtask"] }); true`);

  let eventSequence = 40;
  const revisions = new Array(sessionCount).fill(0);
  const texts = new Array(sessionCount).fill("");
  let eventsSent = 0;
  const tick = () => {
    for (let index = 0; index < sessionCount; index += 1) {
      revisions[index] += 1;
      const token = `token-${index}-${revisions[index]} `;
      texts[index] = (texts[index] + token).slice(-4000);
      const revision = revisions[index];
      const event = {
        type: "semantic_event",
        event_seq: ++eventSequence,
        event: {
          type: "core_topic",
          turn_id: `turn-${index}-${turnsPerSession - 1}`,
          turn_event_id: `preview-${index}-${revision}`,
          timeline_seq: revision,
          created_at_ms: Date.now(),
          event: {
            session_id: `session-${index}`,
            topic: { name: "core.model.preview", attributes: {} },
            state: { name: "waiting_model" },
            payload: {
              turn_id: `turn-${index}-${turnsPerSession - 1}`,
              attempt: 1,
              round: 1,
              revision,
              response: { attempt: 1, revision, text: texts[index], status: "streaming" },
              thinking_words: 0,
              thinking_elapsed_ms: 10,
              ttft_ms: 120,
              reasoning_tokens: 0,
              stream_phase: "responding",
            },
          },
        },
      };
      for (const peer of peers) peer.send(event);
      eventsSent += 1;
    }
  };
  const timer = setInterval(tick, intervalMs);
  await sleep(2000);
  const metrics = async () => {
    const result = await call("Performance.getMetrics");
    return Object.fromEntries(result.metrics.map((metric) => [metric.name, metric.value]));
  };
  const before = await metrics();
  const startedAt = Date.now();
  await sleep(seconds * 1000);
  const wallMs = Date.now() - startedAt;
  const after = await metrics();
  clearInterval(timer);
  const longTasks = await evaluate("window.__benchLongTasks");
  const delta = (name) => Number((((after[name] ?? 0) - (before[name] ?? 0)) * 1000).toFixed(1));
  const taskMs = delta("TaskDuration");
  console.log(
    `RESULT ${JSON.stringify({
      label,
      dist,
      sessions: sessionCount,
      turnsPerSession,
      seconds,
      intervalMs,
      eventsSent,
      wallMs,
      taskMs,
      scriptMs: delta("ScriptDuration"),
      layoutMs: delta("LayoutDuration"),
      styleMs: delta("RecalcStyleDuration"),
      longTasks,
      cpuRatio: Number((taskMs / wallMs).toFixed(3)),
    })}`,
  );
} finally {
  try { browser?.close(); } catch {}
  for (const peer of peers) { try { peer.close(); } catch {} }
  peers = new Set();
  if (child.exitCode === null) child.kill("SIGKILL");
  server.closeAllConnections?.();
  await Promise.race([new Promise((resolveClose) => server.close(resolveClose)), sleep(2000)]);
  await rm(profile, { recursive: true, force: true, maxRetries: 3, retryDelay: 100 }).catch(() => {});
}
