import { createHash } from "node:crypto";
import { createServer } from "node:http";
import { existsSync } from "node:fs";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { spawn } from "node:child_process";
import { tmpdir } from "node:os";
import { extname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

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
  message_queue: {
    revision: 0, items: [], auto_send_enabled: true,
    continuation: { state: "awaiting_normal_completion" },
    dispatching_command_id: null,
  },
  ...extra,
});
const makeCancelledSession = (base = makeSession()) => ({
  ...base,
  state: "ready",
  workers: [worker("ready")],
  cancelling_turn_id: "turn-1",
  turns: base.turns.map((item) => item.turn_id === "turn-1"
    ? { ...item, state: "finished", completion: { stop_reason: "CancelledByUser" } }
    : item),
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
  // A real Host normally has emitted events before a browser connects. A
  // reconnect baseline must therefore work from a non-zero sequence.
  let eventSequence = 40;
  let connectionCount = 0;
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
    connectionCount += 1;
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
      if (!command.command_id) return;
      peer.send({ type: "command_ack", command_id: command.command_id, status: "accepted" });
      if (command.type === "turn_submit" && authoritativeSession.state === "working") {
        authoritativeSession = {
          ...authoritativeSession,
          message_queue: {
            ...authoritativeSession.message_queue,
            revision: authoritativeSession.message_queue.revision + 1,
            items: [
              ...authoritativeSession.message_queue.items,
              {
                command_id: command.command_id,
                enqueue_seq: authoritativeSession.message_queue.items.length,
                payload: {
                  turn_id: `queued-${command.command_id}`,
                  text: command.text,
                  created_at_ms: Date.now(),
                  attachments: [],
                  worker_roles: [],
                },
              },
            ],
          },
        };
        eventSequence += 1;
        peer.send({
          type: "semantic_event",
          event_seq: eventSequence,
          event: {
            type: "message_queue_updated",
            session_id: authoritativeSession.session_id,
            message_queue: authoritativeSession.message_queue,
          },
        });
        // Transport acceptance is not business completion. The authoritative
        // Session queue projection nevertheless makes this future task visible
        // immediately, without browser-owned persistence or replay.
        return;
      }
      // Keep turn_cancel durable until authoritative TurnFinished. This
      // reproduces a reload that receives an older working snapshot while
      // cancellation is already accepted by Host.
      if (command.type !== "turn_cancel")
        peer.send({ type: "command_ack", command_id: command.command_id, status: "committed" });
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
    getSession() { return authoritativeSession; },
    getConnectionCount() { return connectionCount; },
    setSession(session) { authoritativeSession = session; },
    close() { return new Promise((resolve) => server.close(resolve)); },
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
      if (
        Date.now() >= deadline ||
        !["EBUSY", "ENOTEMPTY", "EPERM"].includes(error?.code)
      ) {
        throw error;
      }
      await sleep(100);
    }
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
    const [portLine] = (await readFile(join(profile, "DevToolsActivePort"), "utf8"))
      .trim()
      .split("\n");
    const port = Number(portLine);
    return Number.isInteger(port) && port > 0 ? port : null;
  } catch {
    return null;
  }
}

async function startBrowser(url) {
  const profile = await mkdtemp(join(tmpdir(), "timem-stop-ui-"));
  const child = spawn(chrome, [
    "--remote-debugging-port=0", `--user-data-dir=${profile}`,
    "--headless=new", "--no-sandbox", "--disable-dev-shm-usage",
    "--no-first-run", "--no-default-browser-check",
    "--disable-background-networking", "--disable-component-update", "--disable-sync",
    "--window-size=1440,1000", "about:blank",
  ], { stdio: ["ignore", "ignore", "pipe"] });
  let chromeError = "";
  child.stderr.on("data", (chunk) => { chromeError += String(chunk); });

  try {
    let port = null;
    await waitFor(async () => {
      if (child.exitCode !== null) return false;
      port = await readDevToolsPort(profile);
      if (port === null) return false;
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
        await stopBrowserProcess(child, profile);
      },
    };
  } catch (error) {
    await stopBrowserProcess(child, profile);
    throw error;
  }
}

async function main() {
  const host = await startHost();
  const browser = await startBrowser(host.url);
  const contains = (selector, text) => browser.evaluate(`[...document.querySelectorAll(${JSON.stringify(selector)})].some(n => n.textContent.includes(${JSON.stringify(text)}))`);
  const publish = (revision, response, chat = [], interruption = null) => {
    const preview = { attempt: 1, revision, response: response ? { attempt: 1, revision, text: response, status: "streaming" } : null, chat, interruption };
    const session = host.getSession();
    host.setSession({ ...session, turns: session.turns.map(t => ({ ...t, preview })) });
    host.send({ type: "core_topic", turn_id: "turn-1", turn_event_id: null, event: {
      session_id: "session-1", context_id: "context-1", worker_id: "worker-1",
      topic: { name: "core.model.preview", attributes: {} }, state: "waiting_model",
      payload: { turn_id: "turn-1", ...preview },
    }});
  };
  try {
    await waitFor(() => contains("body", "Long task"), "initial snapshot missing");
    publish(1, "early response", [{ index: 0, task: "Interim", answer: "early chat" }]);
    await sleep(150);
    assert(!(await contains(".response-preview", "early response")), "default off leaked preview");
    await browser.evaluate(`localStorage.setItem("timem-web-stream-ui-mode-v1", "true"); window.dispatchEvent(new StorageEvent("storage", {key:"timem-web-stream-ui-mode-v1"}));`);
    await waitFor(() => contains(".response-preview", "early response"), "midstream enable missing response");
    await waitFor(() => contains(".provisional-chat", "early chat"), "chat did not stream");
    publish(2, "early response continued", [{index:0,task:"Interim",answer:"early chat continued"}]);
    await waitFor(() => contains(".provisional-chat", "continued"), "chat update missing");
    publish(3, "early response continued", [{index:0,task:"Interim",answer:"early chat continued"}], "network_error");
    await waitFor(() => contains(".response-preview-interruption", "Network error"), "network interruption missing");
    await browser.call("Page.reload", {ignoreCache:true});
    await waitFor(() => contains(".provisional-chat", "early chat continued"), "reconnect lost partial chat");
    await waitFor(() => contains(".response-preview-interruption", "Network error"), "reconnect lost interruption");
    publish(4, null, []);
    await waitFor(async () => !(await contains(".provisional-chat", "early chat")), "invalid chat not retracted");
    assert(!(await contains(".response-preview", "early response")), "invalid response not retracted");
    const thoughtEvent = (id, text, time) => ({
      event_id: id, source: "core_topic", created_at_ms: time,
      payload: { session_id: "session-1", state: { name: "running" }, topic: { name: "core.model.response", attributes: {} }, payload: { free_talk: text } },
    });
    const toolEvent = (id, time) => ({
      event_id: id, source: "core_topic", created_at_ms: time,
      payload: { session_id: "session-1", state: { name: "running" }, topic: { name: "core.action", attributes: {} }, payload: { action: id, status: "completed", input: { cmd: "echo hello" } } },
    });
    const setRound = async (events, text, working = true) => {
      const base = host.getSession();
      host.setSession({ ...base, turns: base.turns.map(t => ({ ...t,
        state: working ? "working" : "ready", events,
        preview: { attempt: 2, revision: 20, chat: [], response: { attempt: 2, revision: 20, text, status: "intermediate" } },
      })) });
      await browser.call("Page.reload", { ignoreCache: true });
      await waitFor(() => contains("body", "Long task"), "round snapshot missing");
    };
    const events = [thoughtEvent("thought-n", "Unique thought N", 1), toolEvent("tool-old", 2)];
    await setRound(events, "Unique thought N");
    await waitFor(() => contains(".stream-thought-text", "Unique thought N"), "retained thought missing");
    assert(!(await contains(".response-preview", "Unique thought N")), "intermediate thought duplicated in preview");
    assert(await contains(".stream-tool-row", "tool-old"), "current tool missing");
    assert(await browser.evaluate(`!!document.querySelector('.stream-working-trailer')`), "working trailer missing");
    assert(await browser.evaluate(`!document.querySelector('.stream-thought-card, .stream-reading-hold')`), "extra thought frame exists");
    for (const theme of ["dark", "light"]) {
      assert(await browser.evaluate(`(() => {
        document.documentElement.dataset.theme = ${JSON.stringify(theme)};
        const row = getComputedStyle(document.querySelector('.stream-tool-row'));
        const command = getComputedStyle(document.querySelector('.stream-tool-command'));
        return row.borderTopWidth === '0px' && row.backgroundColor === 'rgba(0, 0, 0, 0)' &&
          command.borderTopWidth === '0px' && command.boxShadow !== 'none';
      })()`), `${theme} tool styles must be unboxed with borderless command shadow`);
    }
    events.push(thoughtEvent("response-next", "", 3), toolEvent("tool-next", 4));
    await setRound(events, "Unique thought N");
    await waitFor(() => contains(".stream-tool-row", "tool-next"), "thoughtless round tool missing");
    assert(!(await contains(".stream-tool-row", "tool-old")), "previous tool remained live");
    assert(await contains(".stream-thought-text", "Unique thought N"), "thoughtless round lost prior thought");
    events.push(thoughtEvent("thought-new", "Unique thought NEW", 5), toolEvent("tool-new", 6));
    await setRound(events, "Unique thought NEW");
    await waitFor(() => contains(".stream-thought-text", "Unique thought NEW"), "new thought missing");
    assert(await browser.evaluate(`document.querySelectorAll(".stream-thought-text").length === 1`), "thought duplicated");
    assert(!(await contains(".stream-tool-row", "tool-next")), "older round tool remained live");
    await setRound(events, "Unique thought NEW", false);
    assert(await browser.evaluate(`!document.querySelector('.stream-working-trailer')`), "terminal turn retained working trailer");
    console.log("PASS Chrome round handoff: single unboxed thought, tool-only round, next thought, terminal trailer");
    console.log("PASS Chrome provisional UI: default off, midstream enable, response/chat updates, network interruption, reload snapshot, retraction");
  } finally { await browser.close(); await host.close(); }
}
await main();
