import { createStartupDiagnostics } from "./browser-startup.mjs";
import { createHash } from "node:crypto";
import { createServer } from "node:http";
import { existsSync } from "node:fs";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { spawn } from "node:child_process";
import { tmpdir } from "node:os";
import { extname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

// Regression guard for the whole-page blank crash: assistant-ui message
// repositories treat duplicate message ids as a fatal React error. A Host
// snapshot (or a legacy restore) that repeats one id must never blank the UI.
const root = resolve(fileURLToPath(new URL("../..", import.meta.url)));
const chromeCandidates = [
  process.env.CHROME_BIN,
  "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
  "/usr/bin/google-chrome",
  "/usr/bin/google-chrome-stable",
  "/usr/bin/chromium",
  "/usr/bin/chromium-browser",
].filter(Boolean);
const chrome = chromeCandidates.find((candidate) => existsSync(candidate));
if (!chrome) {
  throw new Error(
    `Chrome/Chromium not found; checked: ${chromeCandidates.join(", ")}`,
  );
}
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const assert = (condition, message) => { if (!condition) throw new Error(message); };
async function waitFor(check, message, timeout = 15000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    try { if (await check()) return; } catch {}
    await sleep(40);
  }
  throw new Error(typeof message === "function" ? await message() : message);
}

const worker = () => ({
  worker_id: "worker-1", context_id: "context-1", display_name: "Primary",
  ordinal: 0, state: "ready", parent_worker_id: null,
});
// Real restored turns materialize every history user record (task plus
// same-millisecond supplements) as user entries; the snapshot messages array
// mirrors them for the client message repository.
const finishedTurn = (entryTexts) => ({
  turn_id: "turn-history",
  state: "finished",
  created_at_ms: 1,
  user_entries: entryTexts.map((text, index) => ({
    kind: index === 0 ? "task" : "supplement",
    text,
    created_at_ms: 1,
  })),
  events: [],
  final_answer: null,
  completion: { stop_reason: "TurnComplete" },
});
const makeSession = (messages, entryTexts) => ({
  session_id: "session-1", display_name: "Duplicate history", ordinal: 0,
  state: "ready", current_dir: "/work", max_llm_input_tokens: 100000,
  tools: [], mcp_server_ids: [],
  contexts: [{ context_id: "context-1", current_dir: "/work", worker_ids: ["worker-1"] }],
  workers: [worker()], active_context_id: "context-1", primary_worker_id: "worker-1",
  attachments: [], roles: [], messages, turns: [finishedTurn(entryTexts)],
  history_before_cursor: null, history_has_more: false,
  active_turn_id: null, cancelling_turn_id: null, pending_turn_id: null,
  message_queue: {
    revision: 0, items: [], auto_send_enabled: true,
    continuation: { state: "awaiting_normal_completion" },
    dispatching_command_id: null,
  },
});
const makeSnapshot = (session) => ({
  server: {
    version: "ui-acceptance", protocol_version: 1, port: 0,
    bind_host: "127.0.0.1", public_access: false, debug_mode: false,
    performance_trace: false,
    mem: {
      space: "ui-test", data_dir: "/tmp", space_dir: "/tmp/timem-ui-test",
      memory_dir: "/tmp/timem-ui-test/memory", temporary_retention_days: 5,
      temporary_capacity_bytes: null, conversation_capacity_bytes: null,
    },
    runtime_options: [], session_env_defaults: {}, workspace_dirs: ["/work"],
    mcp_servers: [],
    model_endpoints: [{
      id: "endpoint-1", name: "Acceptance endpoint", model: "test-model",
      api_protocol: "openai-compatible", response_protocol: "xml",
      base_url: "http://127.0.0.1/model", max_llm_input_tokens: 100000,
      max_llm_output_tokens: 4096, stream: true, api_key_configured: true,
      http_headers: {},
    }],
  },
  sessions: [session], role_library: { roles: [], groups: [] }, session_groups: [],
});

function encodeFrame(event) {
  const payload = Buffer.from(JSON.stringify(event));
  if (payload.length < 126) return Buffer.concat([Buffer.from([0x81, payload.length]), payload]);
  const header = Buffer.alloc(4);
  header[0] = 0x81; header[1] = 126; header.writeUInt16BE(payload.length, 2);
  return Buffer.concat([header, payload]);
}
function makePeer(socket, onJson) {
  let buffer = Buffer.alloc(0);
  socket.on("data", (chunk) => {
    buffer = Buffer.concat([buffer, chunk]);
    while (buffer.length >= 2) {
      const opcode = buffer[0] & 0x0f;
      let length = buffer[1] & 0x7f;
      let offset = 2;
      if (length === 126) {
        if (buffer.length < 4) return;
        length = buffer.readUInt16BE(2); offset = 4;
      } else if (length === 127) {
        if (buffer.length < 10) return;
        length = Number(buffer.readBigUInt64BE(2)); offset = 10;
      }
      const masked = Boolean(buffer[1] & 0x80);
      const mask = masked ? buffer.subarray(offset, offset + 4) : null;
      if (masked) offset += 4;
      if (buffer.length < offset + length) return;
      const payload = Buffer.from(buffer.subarray(offset, offset + length));
      buffer = buffer.subarray(offset + length);
      if (mask) for (let i = 0; i < payload.length; i += 1) payload[i] ^= mask[i % 4];
      if (opcode === 0x8) { socket.end(); return; }
      if (opcode === 0x1) onJson(JSON.parse(payload.toString("utf8")));
    }
  });
  return { send: (event) => socket.write(encodeFrame(event)) };
}

async function startHost(messages, entryTexts) {
  const mime = {
    ".html": "text/html; charset=utf-8", ".js": "text/javascript; charset=utf-8",
    ".css": "text/css; charset=utf-8", ".woff": "font/woff", ".woff2": "font/woff2",
    ".ttf": "font/ttf", ".svg": "image/svg+xml",
  };
  const server = createServer(async (request, response) => {
    try {
      const pathname = new URL(request.url, "http://localhost").pathname;
      const file = resolve(root, "dist", pathname === "/" ? "index.html" : pathname.slice(1));
      assert(file.startsWith(resolve(root, "dist")), "unsafe asset path");
      const body = await readFile(file);
      response.writeHead(200, { "content-type": mime[extname(file)] ?? "application/octet-stream" });
      response.end(body);
    } catch {
      response.writeHead(404); response.end("not found");
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
    const peer = makePeer(socket, () => {});
    socket.on("error", () => {});
    peer.send({ type: "hello", snapshot: makeSnapshot(makeSession(messages, entryTexts)), event_cursor: 0, event_replay_floor: 0 });
  });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  return {
    url: `http://127.0.0.1:${server.address().port}/`,
    close: () => new Promise((resolve) => server.close(resolve)),
  };
}

async function waitForProcessExit(child, timeoutMs) {
  if (child.exitCode !== null) return true;
  return Promise.race([
    new Promise((resolve) => child.once("exit", () => resolve(true))),
    sleep(timeoutMs).then(() => false),
  ]);
}
async function removeBrowserProfile(profile) {
  const deadline = Date.now() + 5000;
  while (true) {
    try {
      await rm(profile, { recursive: true, force: true, maxRetries: 3, retryDelay: 100 });
      return;
    } catch (error) {
      if (Date.now() >= deadline || !["EBUSY", "ENOTEMPTY", "EPERM"].includes(error?.code)) throw error;
    }
    await sleep(100);
  }
}
async function stopBrowserProcess(child, profile) {
  if (child.exitCode === null) child.kill("SIGTERM");
  if (!(await waitForProcessExit(child, 2500)) && child.exitCode === null) {
    child.kill("SIGKILL");
    await waitForProcessExit(child, 2500);
  }
  await removeBrowserProfile(profile);
}
async function readDevToolsPort(profile) {
  try {
    const [portLine] = (await readFile(join(profile, "DevToolsActivePort"), "utf8")).trim().split("\n");
    const port = Number(portLine);
    return Number.isInteger(port) && port > 0 ? port : null;
  } catch { return null; }
}

async function startBrowser(url) {
  const profile = await mkdtemp(join(tmpdir(), "timem-dup-history-"));
  const child = spawn(chrome, [
    "--remote-debugging-port=0", `--user-data-dir=${profile}`,
    "--headless=new", "--no-sandbox", "--disable-dev-shm-usage",
    "--lang=zh-CN", "--accept-lang=zh-CN",
    "--no-first-run", "--no-default-browser-check",
    "--disable-background-networking", "--disable-component-update", "--disable-sync",
    "--window-size=1440,1000", "about:blank",
  ], { stdio: ["ignore", "ignore", "pipe"] });
  const startup = createStartupDiagnostics(child, chrome);

  try {
    let port = null;
    await waitFor(async () => {
      if (startup.stopped()) return false;
      port = await readDevToolsPort(profile);
      startup.port(port);
      if (port === null) return false;
      return startup.probe(`http://127.0.0.1:${port}/json/version`);
    }, () => startup.failure(), 12000);
    const target = await (await fetch(
      `http://127.0.0.1:${port}/json/new?${encodeURIComponent(url)}`,
      { method: "PUT" },
    )).json();
    const socket = new WebSocket(target.webSocketDebuggerUrl);
    await new Promise((resolve, reject) => {
      socket.addEventListener("open", resolve, { once: true });
      socket.addEventListener("error", reject, { once: true });
    });
    let sequence = 0;
    const requests = new Map();
    const exceptions = [];
    socket.addEventListener("message", ({ data }) => {
      const message = JSON.parse(String(data));
      if (message.method === "Runtime.exceptionThrown") {
        const detail = message.params.exceptionDetails;
        exceptions.push(detail.text ?? detail.exception?.description ?? "unknown exception");
        return;
      }
      if (!message.id || !requests.has(message.id)) return;
      const { resolve, reject } = requests.get(message.id);
      requests.delete(message.id);
      if (message.error) reject(new Error(message.error.message));
      else resolve(message.result);
    });
    const call = (method, params = {}) => new Promise((resolve, reject) => {
      const id = ++sequence;
      requests.set(id, { resolve, reject });
      socket.send(JSON.stringify({ id, method, params }));
    });
    await call("Runtime.enable"); await call("Page.enable");
    const evaluate = async (expression) => {
      const result = await call("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
      if (result.exceptionDetails) throw new Error(result.exceptionDetails.text);
      return result.result.value;
    };
    return {
      evaluate, exceptions,
      async close() {
        socket.close();
        await stopBrowserProcess(child, profile);
      },
    };
  } catch (error) {
    await stopBrowserProcess(child, profile);
    throw error;
  }
}

async function runScenario(label, messages, entryTexts, expects) {
  const host = await startHost(messages, entryTexts);
  const browser = await startBrowser(host.url);
  try {
    const bodyText = () => browser.evaluate("document.body.innerText");
    for (const text of expects.rendered) {
      await waitFor(async () => (await bodyText()).includes(text), `${label}: expected text missing: ${text}`);
    }
    for (const text of expects.absent ?? []) {
      await sleep(300);
      assert(!(await bodyText()).includes(text), `${label}: unexpected text rendered: ${text}`);
    }
    const root = await browser.evaluate("Boolean(document.querySelector('#root') && document.querySelector('#root').innerHTML.trim().length > 0)");
    assert(root, `${label}: page rendered blank`);
    assert(browser.exceptions.length === 0, `${label}: page threw: ${browser.exceptions.join(" | ")}`);
    console.log(`${label}: passed`);
  } finally {
    await browser.close();
    await host.close();
  }
}

async function main() {
  // Scenario A: the fixed deterministic identity separates same-millisecond
  // task + supplement entries (the exact data that used to blank the page).
  await runScenario(
    "unique-history-ids",
    [
      { id: "history_msg_turn-history_1791125099126_user_task_fe687892", role: "user", text: "first task", created_at_ms: 1791125099126, kind: "task", completion: null },
      { id: "history_msg_turn-history_1791125099126_user_supplement_8e15f2ec", role: "user", text: "supplement while working", created_at_ms: 1791125099126, kind: "supplement", completion: null },
    ],
    ["first task", "supplement while working"],
    { rendered: ["first task", "supplement while working"] },
  );
  // Scenario B: defense in depth. Even a snapshot carrying literally
  // duplicated ids (legacy Host, corrupted data) must render instead of
  // blanking: the client feed deduplicates before assistant-ui.
  await runScenario(
    "duplicate-history-ids",
    [
      { id: "history_msg_turn-history_100_user", role: "user", text: "legacy task", created_at_ms: 100, kind: "task", completion: null },
      { id: "history_msg_turn-history_100_user", role: "user", text: "legacy duplicate", created_at_ms: 100, kind: "task", completion: null },
    ],
    ["legacy task"],
    { rendered: ["legacy task"], absent: ["legacy duplicate"] },
  );
}

main().catch((error) => {
  console.error(error);
  process.exit(1);
});
