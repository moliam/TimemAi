import { createHash } from "node:crypto";
import { createServer } from "node:http";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { spawn } from "node:child_process";
import { tmpdir } from "node:os";
import { extname, join, resolve } from "node:path";

const root = resolve(new URL("../..", import.meta.url).pathname);
const chrome = process.env.CHROME_BIN ?? "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const assert = (condition, message) => { if (!condition) throw new Error(message); };
async function waitFor(check, message, timeout = 10000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    try { if (await check()) return; } catch {}
    await sleep(40);
  }
  throw new Error(message);
}

const worker = (state) => ({
  worker_id: "worker-1", context_id: "context-1", display_name: "Primary",
  ordinal: 0, state, parent_worker_id: null,
});
const turn = (id, text = "Long task") => ({
  turn_id: id, state: "working", created_at_ms: Date.now(),
  user_entries: [{
    kind: "task", text,
    ...(id === "turn-1" ? { command_id: "submit-original" } : {}),
    created_at_ms: Date.now(),
  }],
  events: [], sub_answers: [], final_answer: null, completion: null,
});
const makeSession = (extra = {}) => ({
  session_id: "session-1", display_name: "Stop acceptance", ordinal: 0,
  state: "working", current_dir: "/work", max_llm_input_tokens: 100000,
  tools: [], mcp_server_ids: [],
  contexts: [{ context_id: "context-1", current_dir: "/work", worker_ids: ["worker-1"] }],
  workers: [worker("working")], active_context_id: "context-1", primary_worker_id: "worker-1",
  attachments: [], roles: [], messages: [], turns: [turn("turn-1")],
  history_before_cursor: null, history_has_more: false,
  active_turn_id: "turn-1", cancelling_turn_id: null, pending_turn_id: null,
  ...extra,
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

async function startHost() {
  let authoritativeSession = makeSession();
  let eventSequence = 0;
  const peers = new Set();
  const commands = [];
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
    let peer;
    peer = makePeer(socket, (command) => {
      commands.push(command);
      if (command.command_id) {
        peer.send({ type: "command_ack", command_id: command.command_id, status: "accepted" });
        peer.send({ type: "command_ack", command_id: command.command_id, status: "committed" });
      }
    });
    peers.add(peer);
    socket.on("close", () => peers.delete(peer));
    peer.send({
      type: "hello", snapshot: makeSnapshot(authoritativeSession),
      event_cursor: eventSequence, event_replay_floor: 0,
    });
  });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  return {
    url: `http://127.0.0.1:${server.address().port}/`, commands,
    send(event) {
      eventSequence += 1;
      const envelope = { type: "semantic_event", event_seq: eventSequence, event };
      for (const peer of peers) peer.send(envelope);
    },
    setSession(session) { authoritativeSession = session; },
    close() { return new Promise((resolve) => server.close(resolve)); },
  };
}

async function startBrowser(url) {
  const profile = await mkdtemp(join(tmpdir(), "timem-stop-ui-"));
  const port = 9300 + Math.floor(Math.random() * 500);
  const child = spawn(chrome, [
    `--remote-debugging-port=${port}`, `--user-data-dir=${profile}`,
    "--headless=new", "--no-first-run", "--no-default-browser-check",
    "--disable-background-networking", "--disable-component-update", "--disable-sync",
    "--window-size=1440,1000", "about:blank",
  ], { stdio: ["ignore", "ignore", "pipe"] });
  let chromeError = "";
  child.stderr.on("data", (chunk) => { chromeError += String(chunk); });
  await waitFor(async () => {
    try { return (await fetch(`http://127.0.0.1:${port}/json/version`)).ok; }
    catch { return false; }
  }, `Chrome DevTools did not start: ${chromeError}`, 12000);
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
  socket.addEventListener("message", ({ data }) => {
    const message = JSON.parse(String(data));
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
    call, evaluate,
    async close() {
      socket.close();
      child.kill("SIGTERM");
      await Promise.race([new Promise((resolve) => child.once("exit", resolve)), sleep(2500)]);
      if (child.exitCode === null) child.kill("SIGKILL");
      await rm(profile, { recursive: true, force: true });
    },
  };
}

async function main() {
  const host = await startHost();
  const browser = await startBrowser(host.url);
  const exists = (selector) => browser.evaluate(`Boolean(document.querySelector(${JSON.stringify(selector)}))`);
  const contains = (selector, text) => browser.evaluate(
    `document.querySelector(${JSON.stringify(selector)})?.textContent?.includes(${JSON.stringify(text)}) === true`,
  );
  const sent = (type) => host.commands.filter((command) => command.type === type);
  const enterMessage = async (text) => {
    await browser.evaluate(`(() => {
      const textarea = document.querySelector('textarea[aria-label="Message Timem"]');
      textarea.focus();
      Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value').set.call(textarea, ${JSON.stringify(text)});
      textarea.dispatchEvent(new Event('input', { bubbles: true }));
    })()`);
    await browser.call("Input.dispatchKeyEvent", { type: "keyDown", key: "Enter", code: "Enter", windowsVirtualKeyCode: 13 });
    await browser.call("Input.dispatchKeyEvent", { type: "keyUp", key: "Enter", code: "Enter", windowsVirtualKeyCode: 13 });
  };
  try {
    await waitFor(() => exists('.session-working-icon[aria-label="Session working"]'), "initial working spinner missing");
    await waitFor(() => exists('button[aria-label="Cancel current turn"]'), "Stop button missing");

    // Irregular action: two immediate clicks must still emit one targeted cancellation.
    await browser.evaluate(`(() => {
      const button = document.querySelector('button[aria-label="Cancel current turn"]');
      button.click(); button.click();
    })()`);
    await waitFor(() => sent("turn_cancel").length === 1, "rapid double Stop did not emit exactly one turn_cancel");
    assert(sent("turn_cancel")[0].target_command_id === "submit-original", "turn_cancel target mismatch");
    await waitFor(async () => !(await exists('.session-working-icon[aria-label="Session working"]')), "spinner did not stop immediately");

    host.send({
      type: "turn_cancelling", session_id: "session-1", turn_id: "turn-1",
      target_command_id: "submit-original",
    });

    // Irregular action: two rapid Enter presses during cleanup must be durable FIFO, not direct submits.
    await enterMessage("First after Stop");
    await enterMessage("Second after Stop");
    await waitFor(() => contains(".queued-message-list", "First after Stop"), "first post-Stop input not queued");
    await waitFor(() => contains(".queued-message-list", "Second after Stop"), "second post-Stop input not queued");
    assert(sent("turn_submit").length === 0, "post-Stop input escaped the cancellation barrier");

    // Delayed Core events cannot revive visible work.
    host.send({
      type: "turn_started", session_id: "session-1", context_id: "context-1",
      worker_id: "worker-1", turn: turn("turn-1"),
    });
    host.send({
      type: "worker_activity", session_id: "session-1", context_id: "context-1",
      worker_id: "worker-1", turn_id: "turn-1", event: { kind: "model_request", round: 1 },
    });
    await sleep(180);
    assert(!(await exists('.session-working-icon[aria-label="Session working"]')), "late Core event revived spinner");

    // Refresh/reconnect uses authoritative cancelling snapshot and keeps local durable queue.
    host.setSession(makeSession({
      state: "ready", workers: [worker("ready")], cancelling_turn_id: "turn-1",
    }));
    await browser.call("Page.reload", { ignoreCache: true });
    await waitFor(() => contains(".queued-message-list", "First after Stop"), "durable queue lost after refresh");
    assert(!(await exists('.session-working-icon[aria-label="Session working"]')), "refresh/reconnect revived spinner");

    host.send({
      type: "turn_finished", session_id: "session-1", turn_id: "turn-1",
      outcome: { completion: { stop_reason: "CancelledByUser" } },
    });
    try {
      await waitFor(
        () => sent("turn_submit").some((command) => command.text === "First after Stop"),
        "FIFO head was not released after authoritative cancellation completion",
        3000,
      );
    } catch (error) {
      const diagnostics = await browser.evaluate(`(() => ({
        sessionRows: [...document.querySelectorAll('.session-row')].map((row) => ({ className: row.className, text: row.textContent })),
        stopButton: document.querySelector('.stop-button')?.outerHTML ?? null,
        sendButton: document.querySelector('.send-button')?.outerHTML ?? null,
        queue: document.querySelector('.queued-message-list')?.outerHTML ?? null,
        storage: Object.fromEntries(Object.keys(localStorage).map((key) => [key, localStorage.getItem(key)])),
        alerts: [...document.querySelectorAll('[role="alert"]')].map((node) => node.textContent),
      }))()`);
      console.error("UI_DIAGNOSTICS", JSON.stringify(diagnostics, null, 2));
      console.error("HOST_COMMANDS", JSON.stringify(host.commands, null, 2));
      throw error;
    }
    assert(!sent("turn_submit").some((command) => command.text === "Second after Stop"), "second queued input released too early");

    // Duplicate terminal event cannot grant a second continuation.
    host.send({
      type: "turn_finished", session_id: "session-1", turn_id: "turn-1",
      outcome: { completion: { stop_reason: "CancelledByUser" } },
    });
    await sleep(180);
    assert(!sent("turn_submit").some((command) => command.text === "Second after Stop"), "duplicate completion released an extra input");

    host.send({
      type: "turn_started", session_id: "session-1", context_id: "context-1",
      worker_id: "worker-1", turn: turn("turn-2", "First after Stop"),
    });
    host.send({
      type: "turn_finished", session_id: "session-1", turn_id: "turn-2",
      outcome: { completion: {} },
    });
    try {
      await waitFor(
        () => sent("turn_submit").some((command) => command.text === "Second after Stop"),
        "second FIFO input was not released after the next Turn finished",
        3000,
      );
    } catch (error) {
      const diagnostics = await browser.evaluate(`(() => ({
        sessionRows: [...document.querySelectorAll('.session-row')].map((row) => ({ className: row.className, text: row.textContent })),
        stopButton: document.querySelector('.stop-button')?.outerHTML ?? null,
        sendButton: document.querySelector('.send-button')?.outerHTML ?? null,
        queue: document.querySelector('.queued-message-list')?.outerHTML ?? null,
        storage: Object.fromEntries(Object.keys(localStorage).map((key) => [key, localStorage.getItem(key)])),
        alerts: [...document.querySelectorAll('[role="alert"]')].map((node) => node.textContent),
      }))()`);
      console.error("SECOND_UI_DIAGNOSTICS", JSON.stringify(diagnostics, null, 2));
      console.error("SECOND_HOST_COMMANDS", JSON.stringify(host.commands, null, 2));
      throw error;
    }

    console.log("PASS real Chrome Stop UI acceptance");
    console.log("- double Stop -> one targeted turn_cancel");
    console.log("- spinner stops immediately; late events and refresh cannot revive it");
    console.log("- rapid post-Stop inputs persist and release strictly FIFO");
    console.log("- duplicate completion cannot release an extra queued input");
  } finally {
    await browser.close();
    await host.close();
  }
}

await main();
