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
      "--lang=zh-CN", "--accept-lang=zh-CN",
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
    if (process.env.STREAM_CPU_BENCH === "1") {
      await browser.evaluate(`localStorage.setItem("timem-web-stream-ui-mode-v1", "true")`);
      await browser.call("Page.reload", {ignoreCache:true});
      await waitFor(() => contains("body", "Long task"), "benchmark reload missing");
      await browser.call("Performance.enable");
      const metrics = async () => Object.fromEntries((await browser.call("Performance.getMetrics")).metrics.map(m => [m.name,m.value]));
      const before = await metrics();
      let text = "";
      for (let i = 1; i <= 120; i++) {
        text += `Streaming paragraph ${i}: **stable text** and inline code. `;
        publish(i, text);
        await sleep(40);
      }
      await sleep(1500);
      const after = await metrics();
      const delta = Object.fromEntries(["TaskDuration","ScriptDuration","LayoutDuration","RecalcStyleDuration","LayoutCount","RecalcStyleCount"].map(k => [k,after[k]-before[k]]));
      console.log("CPU_BENCH", JSON.stringify(delta));
      if (process.env.TIMEM_PERF_GUARD === "1") {
        assert(delta.TaskDuration < 4, `stream main-thread budget exceeded: ${delta.TaskDuration}s`);
        assert(delta.LayoutCount < 500, `stream layout budget exceeded: ${delta.LayoutCount}`);
      }
      return;
    }
    publish(1, "early response", [{ index: 0, task: "Interim", answer: "early chat" }]);
    await sleep(150);
    assert(!(await contains(".response-preview", "early response")), "default off leaked preview");
    await browser.evaluate(`localStorage.setItem("timem-web-tool-result-status-v1", "true"); window.dispatchEvent(new StorageEvent("storage", {key:"timem-web-tool-result-status-v1"}));`);
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
        state: working ? "working" : "ready", events, sub_answers: [], final_answer: null, completion: null,
        user_entries: [...t.user_entries.filter(entry => entry.kind !== "supplement"),
          { kind: "supplement", text: "Chronological supplement", created_at_ms: 2.5 }],
        preview: { attempt: 2, revision: 20, chat: [], response: { attempt: 2, revision: 20, text, status: "intermediate" } },
      })) });
      await browser.call("Page.reload", { ignoreCache: true });
      await waitFor(() => contains("body", "Long task"), "round snapshot missing");
    };
    // Lifecycle projections update the same DOM row, not a newly entering command.
    const lifecycle = (id, phase, status, time) => {
      const event = toolEvent(id, time);
      Object.assign(event.payload.payload, { action: "run_bash", action_id: "stable-action", event: phase, status });
      return event;
    };
    const actionEvents = [thoughtEvent("stable-thought", "Stable thought", 1), lifecycle("start", "start", "running", 2)];
    await setRound(actionEvents, "Stable thought");
    await waitFor(() => contains(".stream-tool-row", "Bash"), "initial action missing");
    await browser.evaluate(`window.actionRow = document.querySelector('.stream-tool-row'); window.actionCommand = document.querySelector('.stream-tool-command'); window.actionHead = document.querySelector('.stream-tool-head'); true;`);
    assert(await browser.evaluate(`!document.querySelector('.stream-tool-fold.expanded') && document.querySelector('.stream-tool-command-preview').textContent === 'echo hello' && document.querySelector('.stream-tool-toggle').getAttribute('aria-expanded') === 'false'`), "running tool must default to one-line closed summary");
    await browser.evaluate(`document.querySelector('.stream-tool-toggle').click()`);
    await waitFor(() => browser.evaluate(`!!document.querySelector('.stream-tool-fold.expanded')`), "running command cannot expand");
    await sleep(450);
    await browser.evaluate(`window.toolHeight = document.querySelector('.stream-tool-row').getBoundingClientRect().height; true`);
    for (const [phase, status, time] of [["execution_start", "running", 3], ["finish", "background_running", 4], ["finish", "completed", 5]]) {
      actionEvents.push(lifecycle(`update-${time}`, phase, status, time));
      const base = host.getSession();
      const updated = { ...base, turns: base.turns.map(t => ({ ...t, events: [...actionEvents] })) };
      host.setSession(updated);
      host.send({ type: "hello", snapshot: makeSnapshot(updated) });
      if (status === "running") {
        await waitFor(() => browser.evaluate(`!!document.querySelector('.stream-tool-dot') && !document.querySelector('.stream-tool-status')?.textContent`), "running must use dot without redundant text");
        assert(await browser.evaluate(`getComputedStyle(document.querySelector('.stream-tool-row.running .stream-tool-dot')).animationName.includes('stream-tool-glow')`), "running dot must carry the glow pulse");
        assert(await browser.evaluate(`getComputedStyle(document.querySelector('.stream-tool-row.running .stream-tool-log')).maxHeight === '120px'`), "running live log must clamp to 120px");
      } else {
        await waitFor(() => contains(status === "background_running" ? ".stream-tool-background" : ".stream-tool-status", status === "background_running" ? "(bg)" : status === "completed" ? "✓" : status), `${status}: status not delivered`);
      }
      assert(await browser.evaluate(`window.actionRow === document.querySelector('.stream-tool-row') && window.actionCommand === document.querySelector('.stream-tool-command') && window.actionHead === document.querySelector('.stream-tool-head')`), `${status}: action DOM remounted`);
      assert(await browser.evaluate(`document.querySelectorAll('.stream-tool-row').length === 1`), "status update duplicated action");
      assert(await browser.evaluate(`!!document.querySelector('.stream-tool-dot') === ${status === "running" || status === "background_running"}`), `${status}: tool dot visibility incorrect`);
      assert(await browser.evaluate(`(() => {
        const slot = document.querySelector('.stream-tool-status-slot');
        const marker = slot.querySelector('.stream-tool-dot') || slot.querySelector('.stream-tool-status');
        const name = slot.nextElementSibling;
        const a = slot.getBoundingClientRect(), b = marker.getBoundingClientRect();
        return name.tagName === 'B' && a.right <= name.getBoundingClientRect().left && Math.abs((a.left + a.right - b.left - b.right) / 2) < 1;
      })()`), `${status}: status must stay centered before the tool name`);

      if (status === "completed") {
        await waitFor(() => browser.evaluate(`!!document.querySelector('.stream-tool-status[role="status"] .action-status-changed')`), "status-only highlight missing");
      }
    }
    await waitFor(() => contains(".stream-tool-status", "✓"), "terminal status missing");
    await sleep(700);
    assert(await browser.evaluate(`!!document.querySelector('.stream-tool-fold.expanded') && Math.abs(document.querySelector('.stream-tool-row').getBoundingClientRect().height - window.toolHeight) < 1 && !document.querySelector('.stream-tool-merged-item.merged')`), "completion changed user expansion or geometry before AI reply");
    // Same-round serial execution advances logical order without any AI text.
    const serialStart = lifecycle("serial-start", "execution_start", "running", 6);
    serialStart.payload.payload.action_id = "serial-b";
    const serial = { ...host.getSession(), turns: host.getSession().turns.map(t => ({ ...t, events: [...actionEvents, serialStart] })) };
    host.setSession(serial); host.send({ type: "hello", snapshot: makeSnapshot(serial) });
    await waitFor(() => browser.evaluate(`document.querySelectorAll('.stream-tool-merged-item.merged').length === 1 && document.querySelectorAll('.stream-tool-row.running').length === 1`), "serial B start must fold A and retain B");
    assert(await browser.evaluate(`window.actionRow === document.querySelector('.stream-tool-row')`), "serial handoff remounted A");
    assert(await browser.evaluate(`getComputedStyle(document.querySelector('.stream-tool-merged-item')).transitionDuration === '0s'`), "automatic tool handoff must not animate page layout");
    const settledPositions = await browser.evaluate(`new Promise(resolve => {
      const positions = []; let remaining = 18;
      const sample = () => {
        const viewport = document.querySelector('.chat-scroll');
        const row = document.querySelector('.stream-tool-row.running');
        positions.push({scroll: viewport.scrollTop, top: row.getBoundingClientRect().top});
        if (--remaining) requestAnimationFrame(sample); else resolve(positions);
      }; requestAnimationFrame(sample);
    })`);
    for (const field of ['scroll', 'top']) {
      const values = settledPositions.map(position => position[field]);
      assert(Math.max(...values) - Math.min(...values) < 1, `serial handoff keeps ${field} stable after commit: ${JSON.stringify(values)}`);
    }
    for (const width of [1440, 390]) {
      await browser.call("Emulation.setDeviceMetricsOverride", {width, height:1000, deviceScaleFactor:1, mobile:false});
      assert(await browser.evaluate(`(() => {
        const summary = document.querySelector('.stream-tool-run-toggle > svg');
        const live = document.querySelector('.stream-tool-row.running .stream-tool-toggle > svg');
        return !!summary && !!live && Math.abs(summary.getBoundingClientRect().left - live.getBoundingClientRect().left) < 1;
      })()`), `collapsed summary and live tool must be peers at ${width}px`);
    }
    await browser.call("Emulation.clearDeviceMetricsOverride");

    const serialFinish = lifecycle("serial-finish", "finish", "failed", 7);
    serialFinish.payload.payload.action_id = "serial-b";
    const serialDone = { ...serial, turns: serial.turns.map(t => ({ ...t, events: [...actionEvents, serialStart, serialFinish] })) };
    host.setSession(serialDone); host.send({ type: "hello", snapshot: makeSnapshot(serialDone) });
    await waitFor(() => contains(".stream-tool-status", "✗"), "serial B finish missing");
    assert(await browser.evaluate(`document.querySelectorAll('.stream-tool-merged-item.merged').length === 1`), "B completion must not fold B");
    await setRound(actionEvents, "Stable thought");
    await browser.evaluate(`document.querySelector('.stream-tool-toggle').click()`);
    const afterTool = host.getSession();
    const reply = {...afterTool, turns:afterTool.turns.map(t => ({...t, events:[...actionEvents, thoughtEvent("after-tool", "Next AI reply", 6)]}))};
    host.setSession(reply); host.send({type:"hello", snapshot:makeSnapshot(reply)});
    await waitFor(() => browser.evaluate(`!!document.querySelector('.stream-tool-merged-item.merged')`), "next AI reply did not retire completed single tool");
    await waitFor(() => browser.evaluate(`!!document.querySelector('.stream-tool-count.incremented')`), "deferred tool absorption must animate even when completion count did not change");
    await browser.evaluate(`document.querySelector('.stream-tool-run-toggle').click()`);
    await waitFor(() => browser.evaluate(`!document.querySelector('.stream-tool-merged-item.merged') && !!document.querySelector('.stream-tool-fold.expanded')`), "reopening run lost manual output expansion");

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
    assert(await browser.evaluate(`(() => {
      const command = document.querySelector('.stream-tool-command');
      const style = getComputedStyle(command);
      return style.animationName === 'stream-tool-enter' && style.animationDuration === '0.16s';
    })()`), "command entrance transition missing");
    assert(await browser.evaluate(`(() => {
      const sessionIcon = document.querySelector('.session-working-icon');
      const sessionAnim = sessionIcon ? getComputedStyle(sessionIcon) : null;
      const sessionDot = sessionIcon ? sessionIcon.getBoundingClientRect() : null;
      const workerStatic = [...document.querySelectorAll('.worker-working-icon')].every(node => getComputedStyle(node).animationName === 'none');
      const pulse = document.querySelector('.turn-assistant-frame.working .working-chip .pulse, .stream-working-dot');
      const pulseAnim = pulse ? getComputedStyle(pulse) : null;
      return !!sessionAnim && sessionAnim.animationName === 'stream-working-grow' &&
        parseFloat(sessionAnim.animationDuration) === 1.2 &&
        !!sessionDot &&
        workerStatic && !!pulseAnim && pulseAnim.animationName === sessionAnim.animationName &&
        pulseAnim.animationDuration === sessionAnim.animationDuration;
    })()`), "sidebar session cue must breathe in sync with the chat pulse while workers stay static");
    assert(await browser.evaluate(`['.user-message-navigation button', '.session-group-heading', '.final-answer-outline-toggle'].every(selector => [...document.querySelectorAll(selector)].every(node => getComputedStyle(node).backdropFilter === 'none'))`), "scroll overlays must not sample blurred backdrops");
    await browser.call("Emulation.setEmulatedMedia", { features: [{ name: "prefers-reduced-motion", value: "reduce" }] });
    assert(await browser.evaluate(`['.stream-tool-head', '.stream-tool-command', '.stream-tool-dot'].every(selector => { const node = document.querySelector(selector); return !node || getComputedStyle(node).animationName === 'none'; })`), "reduced motion must disable tool entrance");
    await browser.call("Emulation.setEmulatedMedia", { features: [] });
    await waitFor(() => contains(".turn-stream-tools .user-supplement", "Chronological supplement"), "live supplement missing");
    assert(await browser.evaluate(`(() => {
      const region = document.querySelector('.turn-stream-tools');
      const text = region.textContent;
      return text.indexOf('Unique thought N') < text.indexOf('tool-old') &&
        text.indexOf('tool-old') < text.indexOf('Chronological supplement');
    })()`), "live supplement order incorrect");
    events.push(thoughtEvent("response-next", "", 3), toolEvent("tool-next", 4));
    await setRound(events, "Unique thought N");
    await waitFor(() => contains(".stream-tool-row", "tool-next"), "thoughtless round tool missing");
    assert(await contains(".stream-tool-row", "tool-old"), "previous tool disappeared during work");
    assert(await contains(".stream-thought-text", "Unique thought N"), "thoughtless round lost prior thought");
    assert(await contains(".turn-stream-tools .user-supplement", "Chronological supplement"), "supplement disappeared during work");
    for (const theme of ["dark", "light"]) {
      assert(await browser.evaluate(`(() => {
        document.documentElement.dataset.theme = ${JSON.stringify(theme)};
        const bubble = getComputedStyle(document.querySelector('.turn-stream-tools .user-supplement'));
        const user = getComputedStyle(document.querySelector('.turn-user-content'));
        return bubble.backgroundColor === user.backgroundColor && bubble.color === user.color &&
          bubble.backgroundColor !== 'rgba(0, 0, 0, 0)' && parseFloat(bubble.borderTopLeftRadius) >= 12 && parseFloat(bubble.paddingLeft) > 0;
      })()`), `${theme}: supplement must use the user's colored bubble`);
    }

    assert(await browser.evaluate(`!document.querySelector('.turn-assistant-frame')`), "working process was prematurely archived");
    events.push(thoughtEvent("thought-new", "Unique thought NEW", 5), toolEvent("tool-new", 6));
    await setRound(events, "Unique thought NEW");
    await waitFor(() => contains(".stream-thought-text", "Unique thought NEW"), "new thought missing");
    assert(await browser.evaluate(`document.querySelectorAll(".stream-thought-text").length === 2`), "earlier thought disappeared");
    assert(await contains(".stream-tool-row", "tool-next"), "older round tool disappeared");
    await setRound(events, "Unique thought NEW", false);
    assert(await browser.evaluate(`!document.querySelector('.stream-working-trailer')`), "terminal turn retained working trailer");
    const workExpanded = () => browser.evaluate(`document.querySelector('button.work-title-chip')?.getAttribute('aria-expanded')`);
    for (const streamMode of [false, true]) {
      await browser.evaluate(`localStorage.setItem("timem-web-stream-ui-mode-v1", ${JSON.stringify(String(streamMode))});`);
      for (const terminal of ["finished", "interrupted"]) {
        const base = host.getSession();
        const workingSession = { ...base, state: "working", active_turn_id: "turn-1",
          turns: base.turns.map(t => ({ ...t, state: "working", final_answer: null, completion: null })) };
        host.setSession(workingSession);
        await browser.call("Page.reload", { ignoreCache: true });
        if (streamMode) assert(await browser.evaluate(`!document.querySelector('.turn-assistant-frame')`), "working frame should not appear");
        else await waitFor(async () => (await workExpanded()) === "true", "working panel default changed");
        const completedSession = { ...workingSession, state: "ready", active_turn_id: null,
          workers: [worker("ready")], turns: workingSession.turns.map(t => ({ ...t,
            state: terminal, final_answer: terminal === "finished" ? "Completed answer" : null,
          })) };
        host.setSession(completedSession);
        host.send({ type: "hello", snapshot: makeSnapshot(completedSession) });
        await waitFor(async () => (await workExpanded()) === "false", `${streamMode}/${terminal}: did not auto-collapse`);
        await browser.call("Page.reload", { ignoreCache: true });
        await waitFor(async () => (await workExpanded()) === "false", `${streamMode}/${terminal}: reload expanded historical work`);
        await waitFor(() => contains(".work-tool-count", "(+3 tools)"), "historical collapsed count must include all calls");
        await browser.evaluate(`document.querySelector('button.work-title-chip').click()`);
        await waitFor(async () => (await workExpanded()) === "true", "historical manual expansion failed");
        for (const theme of ["dark", "light"]) {
          assert(await browser.evaluate(`(() => {
            document.documentElement.dataset.theme = ${JSON.stringify(theme)};
            const groups = [...document.querySelectorAll('.tool-activity-group-body')];
            const summaries = [...document.querySelectorAll('.tool-activity summary, .tool-activity-group > summary')];
            if (!groups.length || !summaries.length) return false;
            summaries[0].focus();
            return groups.every(node => getComputedStyle(node).borderLeftWidth === '0px') &&
              summaries.every(node => !getComputedStyle(node).boxShadow.includes('inset'));
          })()`), `${theme}: archived commands must not have a left rail or thick focus stripe`);
        }

      }
    }
    await browser.evaluate(`localStorage.setItem("timem-web-stream-ui-mode-v1", "true")`);
    const answer = (id, ordinal, time) => ({ sub_answer_id: id, ordinal, task: "Hidden task metadata", answer: `Interim body ${ordinal}`, created_at_ms: time, preview_attempt: 3, preview_index: ordinal - 1 });
    const interimSession = { ...host.getSession(), state: "working", active_turn_id: "turn-1", turns: [{
      ...turn("turn-1"), events: [thoughtEvent("prior", "Prior thought archived", 1),
        { ...toolEvent("delivery", 2), payload: { ...toolEvent("delivery", 2).payload,
          payload: { action: "sub_answer", status: "completed", input: { answer: "Interim body 2", task: "Hidden task metadata" } } } }],
      sub_answers: [answer("a1", 1, 3), answer("a2", 2, 4)],
      preview: { attempt: 3, revision: 1, chat: [{ index: 1, task: "Hidden task metadata", answer: "Interim body 2" }], response: null },
    }] };
    host.setSession(interimSession);
    await browser.call("Page.reload", { ignoreCache: true });
    await waitFor(() => contains(".live-interim-answer", "Interim body 2"), "latest answer not live");
    assert(!(await contains(".turn-stream-tools", "sub_answer")), "delivery tool leaked into stream");
    assert(await contains(".turn-stream-tools", "Prior thought archived"), "prior thought disappeared");
    assert(await browser.evaluate(`document.querySelectorAll('.live-interim-answer').length === 1`), "only latest answer should remain live");
    for (const status of ["completed", "failed", "timeout", "cancelled", "cancelled_by_user", "running", "background_running"]) {
      for (const name of ["run_bash", "readfile"]) {
        const event = toolEvent(name, 6);
        event.payload.payload.status = status;
        host.setSession({ ...interimSession, turns: [{ ...turn("turn-1"), events: [event] }] });
        await browser.call("Page.reload", { ignoreCache: true });
        await waitFor(() => browser.evaluate(`!!document.querySelector('.stream-tool-row')`), "action row missing");
        assert(await browser.evaluate(`!!document.querySelector('.stream-tool-dot') === ${status === "running" || status === "background_running"}`), `${name}/${status}: execution dot must match running state`);
        if (status === "running" || status === "background_running") {
          assert(await browser.evaluate(`(() => {
            const dot = document.querySelector('.stream-tool-dot');
            const animation = dot.getAnimations()[0];
            if (!animation) return false;
            animation.pause(); animation.currentTime = 0;
            const small = dot.getBoundingClientRect().width;
            const slot = dot.parentElement.getBoundingClientRect().width;
            animation.currentTime = 600;
            const large = dot.getBoundingClientRect().width;
            const stable = Math.abs(dot.parentElement.getBoundingClientRect().width - slot) < .1;
            animation.play();
            return large > small + 2 && stable;
          })()`), `${name}/${status}: dot must breathe without resizing its slot`);
          await browser.call("Emulation.setEmulatedMedia", {features:[{name:"prefers-reduced-motion",value:"reduce"}]});
          assert(await browser.evaluate(`getComputedStyle(document.querySelector('.stream-tool-dot')).animationName === 'none'`), "reduced motion must disable breathing");
          await browser.call("Emulation.setEmulatedMedia", {features:[]});
        }
        if (status === "running" || status === "background_running") {
          assert(await browser.evaluate(`![...document.querySelectorAll('.stream-tool-status')].some(node => /running/i.test(node.textContent)) && !!document.querySelector('.stream-tool-status-slot[aria-label]')`), "running text must be omitted visually but retained accessibly");
        }
      }
    }
    const next = { ...interimSession, turns: interimSession.turns.map(t => ({ ...t, preview: null,
      events: [...t.events, thoughtEvent("after", "Next thought focus", 5), toolEvent("run_bash", 6)] })) };
    host.setSession(next);
    await browser.call("Page.reload", { ignoreCache: true });
    await waitFor(() => contains(".stream-thought-text", "Next thought focus"), "next thought not focused");
    assert(await browser.evaluate(`document.querySelectorAll('.live-interim-answer').length === 0`), "next reply must collapse prior answers into Chat");
    assert(await browser.evaluate(`document.querySelectorAll('.turn-stream-tools .chat-title-chip[aria-expanded="false"]').length === 2`), "reload lost collapsed Chat entries");
    await browser.evaluate(`document.querySelector('.turn-stream-tools .chat-title-chip').click()`);
    await waitFor(() => contains(".live-interim-answer", "Interim body 1"), "Chat could not reopen delivered answer");
    await browser.evaluate(`document.querySelector('.turn-stream-tools .chat-title-chip').click()`);
    assert(await browser.evaluate(`document.querySelectorAll('.live-interim-answer').length === 0`), "Chat could not collapse again");
    const finishedInterim = { ...next, state: "ready", active_turn_id: null, turns: next.turns.map(t => ({ ...t, state: "finished", final_answer: "Final after interim" })) };
    host.setSession(finishedInterim);
    await browser.call("Page.reload", { ignoreCache: true });
    await waitFor(() => contains(".turn-chat-delivery", "Chat (+2)"), "finished history lost Chat disclosure");
    await browser.evaluate(`document.querySelector('.chat-title-chip').click()`);
    await waitFor(() => contains(".turn-chat-panel", "Interim body 1"), "finished Chat lost first answer");
    assert(await contains(".turn-chat-panel", "Interim body 2"), "finished Chat lost second answer");
    host.setSession(next);
    await browser.call("Page.reload", { ignoreCache: true });
    await waitFor(() => contains(".stream-thought-text", "Next thought focus"), "active fixture not restored");
    assert(await browser.evaluate(`!getComputedStyle(document.querySelector('.stream-tool-command')).fontFamily.match(/monospace|Consolas|SFMono/i)`), "command still uses console font");
    for (const size of ["12px", "16px"]) {
      assert(await browser.evaluate(`(() => {
        document.documentElement.style.setProperty('--content-size', ${JSON.stringify(size)});
        const reference = document.createElement('section');
        reference.className = 'turn-final-delivery';
        reference.innerHTML = '<div class="message-content"><div class="markdown-body"><p>Reference</p></div></div>';
        document.querySelector('.turn-answer-delivery').append(reference);
        const actual = getComputedStyle(document.querySelector('.stream-thought-text .markdown-body p'));
        const expected = getComputedStyle(reference.querySelector('p'));
        const equal = ['fontFamily', 'fontSize', 'fontWeight', 'lineHeight'].every(key => actual[key] === expected[key]);
        reference.remove();
        return equal;
      })()`), `stream/final typography differs at ${size}`);
    }
    assert(await browser.evaluate(`(() => {
      const text = document.querySelector('.turn-stream-tools > .stream-thought-text');
      const row = document.querySelector('.stream-tool-row');
      return text && row && getComputedStyle(text).marginBottom === '0px' && getComputedStyle(row).paddingTop === '2px' && getComputedStyle(row).paddingBottom === '2px';
    })()`), "live text/tool spacing stacks paragraph margin or oversized row padding");
    const longThought = Array.from({length: 60}, (_, i) => `Paragraph ${i} remains stable.\n\n`).join("");
    const longEvents = [thoughtEvent("long", longThought, 1), toolEvent("adjacent-a", 3), toolEvent("adjacent-b", 4), thoughtEvent("after-adjacent", "Following AI reply", 100)];
    await setRound(longEvents, longThought);
    await waitFor(() => contains(".stream-tool-run-toggle", "2 ✓"), "adjacent completed tools not merged");
    await waitFor(() => browser.evaluate(`document.querySelectorAll('.stream-tool-merged-item.merged').length === 2`), "completed group not folded");
    await browser.evaluate(`document.querySelector('.stream-tool-run-toggle').click()`);
    await waitFor(() => browser.evaluate(`!document.querySelector('.stream-tool-merged-item.merged')`), "merged calls cannot expand");
    await browser.evaluate(`document.querySelector('.stream-tool-toggle').click()`);
    await waitFor(() => browser.evaluate(`!!document.querySelector('.stream-tool-fold.expanded')`), "completed output cannot expand");
    await browser.evaluate(`window.stableThought = document.querySelector('.stream-thought-text'); window.stableTop = window.stableThought.getBoundingClientRect().top; true;`);
    const baseLong = host.getSession();
    const nextLong = {...baseLong, turns: baseLong.turns.map(t => ({...t, events: [...longEvents, thoughtEvent("long-next", "Next round stays below", 5)]}))};
    host.setSession(nextLong); host.send({type:"hello", snapshot:makeSnapshot(nextLong)});
    await waitFor(() => contains(".stream-thought-text", "Next round stays below"), "next round missing");
    assert(await browser.evaluate(`window.stableThought === document.querySelector('.stream-thought-text')`), "next round remounted old text");
    for (const terminal of ["finished", "interrupted"]) {
      await setRound(longEvents, longThought);
      const current = host.getSession();
      const done = {...current, state:"ready", active_turn_id:null, turns:current.turns.map(t => ({...t, state:terminal, final_answer:terminal === "finished" ? "Final settled answer" : null}))};
      host.setSession(done); host.send({type:"hello", snapshot:makeSnapshot(done)});
      await waitFor(() => browser.evaluate(`!!document.querySelector('.stream-continuous-process.archiving')`), "terminal transition did not start");
      assert(await browser.evaluate(`document.querySelector('.stream-continuous-process').getAnimations().length > 0`), "terminal height animation missing");
      await waitFor(() => browser.evaluate(`!document.querySelector('.stream-continuous-process') && !!document.querySelector('.collapsed-work')`), "process not archived after transition");
      assert(await browser.evaluate(`!document.querySelector('.stream-working-trailer')`), "terminal trailer still visible");
    }
    // A tall process must not leave stale scroll space after final-answer handoff.
    for (const reducedMotion of [false, true]) {
    await browser.call("Emulation.setEmulatedMedia", { features: [{ name: "prefers-reduced-motion", value: reducedMotion ? "reduce" : "no-preference" }] });
    for (const streamMode of [true, false]) {
      await browser.evaluate(`localStorage.setItem("timem-web-stream-ui-mode-v1", ${JSON.stringify(String(streamMode))})`);
      const manyTools = Array.from({length: 80}, (_, i) => toolEvent(`handoff-tool-${i}`, 3 + i));
      await setRound([thoughtEvent("handoff-thought", longThought, 1), ...manyTools], longThought);
      await browser.evaluate(`document.querySelector('.chat-scroll').scrollTop = document.querySelector('.chat-scroll').scrollHeight`);
      const current = host.getSession();
      const finalText = Array.from({length: 8}, (_, i) => `## Final section ${i}\n\n${"Final answer reading text. ".repeat(20)}\n\n`).join("");
      const done = {...current, state:"ready", active_turn_id:null, turns:current.turns.map(t => ({...t, state:"finished", final_answer:finalText}))};
      host.setSession(done); host.send({type:"hello", snapshot:makeSnapshot(done)});
      await waitFor(() => browser.evaluate(`!!document.querySelector('.turn-final-delivery') && !document.querySelector('.stream-continuous-process')`), "large process did not archive");
      await sleep(300);
      const geometry = await browser.evaluate(`(() => {
        const viewport = document.querySelector('.chat-scroll');
        const answer = document.querySelector('.turn-final-delivery').getBoundingClientRect();
        const view = viewport.getBoundingClientRect();
        return { visible: answer.bottom > view.top && answer.top < view.bottom, trailing: viewport.scrollHeight - (answer.bottom - view.top + viewport.scrollTop), top: viewport.scrollTop };
      })()`);
      assert(geometry.visible && geometry.trailing < 150, `${streamMode}: archive left blank viewport/stale scroll space: ${JSON.stringify(geometry)}`);
    }
    }
    await browser.call("Emulation.setEmulatedMedia", { features: [] });
    console.log("PASS Chrome large-tool handoff: final answer visible without stale scroll space in both UI and motion modes");
    await browser.evaluate(`localStorage.setItem("timem-web-stream-ui-mode-v1", "true")`);
    // Visual interaction contracts, beyond node identity.
    await setRound(longEvents, longThought);
    await waitFor(() => browser.evaluate(`document.querySelectorAll('.stream-tool-merged-item.merged').length === 2`), "initial merged run missing");
    assert(await browser.evaluate(`document.querySelector('.stream-tool-count').textContent === '2 ✓' && !document.querySelector('.stream-tool-count.incremented')`), "initial snapshot must hide zero failures without animating");
    await browser.evaluate(`window.countToggle = document.querySelector('.stream-tool-run-toggle'); window.countRows = [...document.querySelectorAll('.stream-tool-row')]; window.countNode = document.querySelector('.stream-tool-count'); window.countAnimations = 0; document.addEventListener('animationstart', e => { if(e.animationName === 'stream-tool-count-increment') window.countAnimations++; });`);
    // CSS zoom exercises layout scaling, not native browser chrome zoom. DPR is
    // varied independently so physical pixel density cannot drive CSS spacing.
    const responsiveOriginal = await browser.evaluate(`({root:document.documentElement.style.fontSize, content:document.documentElement.style.getPropertyValue('--content-size'), zoom:document.body.style.zoom})`);
    let responsiveCases = 0;
    for (const width of [390, 768, 1440]) {
      for (const font of [12, 16, 24, 40]) {
        for (const zoom of [1, 1.5, 2]) {
          const dpr = width === 390 ? 3 : width === 768 ? 2 : 1;
          await browser.call("Emulation.setDeviceMetricsOverride", {width, height:1000, deviceScaleFactor:dpr, mobile:false});
          const result = await browser.evaluate(`(() => {
            document.documentElement.style.fontSize = '16px';
            document.documentElement.style.setProperty('--content-size', '${font}px');
            document.body.style.zoom = '${zoom}';
            const tools = document.querySelector('.turn-stream-tools');
            const style = getComputedStyle(tools);
            const toggle = tools.querySelector('.stream-tool-run-toggle');
            const rect = toggle.getBoundingClientRect();
            return {gap:parseFloat(style.rowGap), margin:parseFloat(style.marginBottom), width:document.documentElement.scrollWidth, viewport:innerWidth, toggleWidth:rect.width, toggleHeight:rect.height, label:tools.querySelector('.stream-tool-count').textContent};
          })()`);
          const expected = Math.min(4, Math.max(2, font * .125));
          assert(Math.abs(result.gap - expected) < .1 && Math.abs(result.margin - expected) < .1, `relative spacing ${width}/${font}/${zoom}: ${JSON.stringify(result)}`);
          assert(result.width <= result.viewport + 1 && result.toggleWidth > 0 && result.toggleHeight > 0 && result.label === '2 ✓', `responsive overflow/control ${width}/${font}/${zoom}: ${JSON.stringify(result)}`);
          responsiveCases++;
        }
      }
    }
    // Base-font scaling and lower clamp bound, independently of reading size.
    assert(await browser.evaluate(`(() => {
      document.documentElement.style.fontSize = '20px';
      document.documentElement.style.setProperty('--content-size', '8px');
      return parseFloat(getComputedStyle(document.querySelector('.turn-stream-tools')).rowGap) === 2.5;
    })()`), "spacing lower bound must follow root font");
    assert(await browser.evaluate(`(() => {
      document.documentElement.style.setProperty('--content-size', '100px');
      const style = getComputedStyle(document.querySelector('.turn-stream-tools'));
      return parseFloat(style.rowGap) === 5 && parseFloat(style.marginBottom) === 5;
    })()`), "spacing upper bound must follow root font");
    await browser.evaluate(`document.documentElement.style.fontSize = ${JSON.stringify(responsiveOriginal.root)}; document.documentElement.style.setProperty('--content-size', ${JSON.stringify(responsiveOriginal.content)}); document.body.style.zoom = ${JSON.stringify(responsiveOriginal.zoom)}; true`);
    await browser.call("Emulation.clearDeviceMetricsOverride");
    console.log(`PASS Chrome responsive count layout: ${responsiveCases} viewport/font/CSS-zoom combinations, DPR 1/2/3 and root-font clamp`);
    const visualBase = host.getSession();
    const moreCalls = {...visualBase, turns: visualBase.turns.map(t => ({...t, events:[...longEvents, toolEvent("adjacent-c", 5)]}))};
    host.setSession(moreCalls); host.send({type:"hello", snapshot:makeSnapshot(moreCalls)});
    await waitFor(() => contains(".stream-tool-run-toggle", "3 ✓"), "new completion missing");
    assert(await browser.evaluate(`document.querySelectorAll('.stream-tool-merged-item.merged').length === 3`), "new completion reopened merged history");
    await waitFor(() => browser.evaluate(`window.countAnimations === 1`), "increment animation did not start");
    assert(await browser.evaluate(`window.countToggle === document.querySelector('.stream-tool-run-toggle') && window.countNode !== document.querySelector('.stream-tool-count') && window.countRows.every((row, i) => row === document.querySelectorAll('.stream-tool-row')[i])`), "count increment remounted toggle or tool rows");
    assert(await browser.evaluate(`document.querySelector('.stream-tool-count').textContent === '3 ✓' && getComputedStyle(document.querySelector('.stream-tool-count')).animationDuration === '0.36s'`), "count label or animation duration incorrect");
    await sleep(450);
    await browser.evaluate(`window.countNode = document.querySelector('.stream-tool-count'); true`);
    host.send({type:"hello", snapshot:makeSnapshot(moreCalls)});
    await sleep(450);
    assert(await browser.evaluate(`window.countAnimations === 1 && window.countNode === document.querySelector('.stream-tool-count') && document.querySelector('.stream-tool-count').getAnimations().length === 0`), "duplicate snapshot replayed animation or animation never settled");

    // Real browser hot path: repeated increments, bounded layout/CPU, stable DOM.
    await browser.call("Performance.enable");
    const countMetrics = async () => Object.fromEntries((await browser.call("Performance.getMetrics")).metrics.map(m => [m.name, m.value]));
    const countBefore = await countMetrics();
    const growingEvents = [...longEvents, toolEvent("adjacent-c", 5)];
    for (let i = 4; i <= 23; i++) {
      growingEvents.push(toolEvent(`increment-${i}`, i + 2));
      const next = {...visualBase, turns: visualBase.turns.map(t => ({...t, events:[...growingEvents]}))};
      host.setSession(next); host.send({type:"hello", snapshot:makeSnapshot(next)});
      await waitFor(() => browser.evaluate(`document.querySelector('.stream-tool-count')?.textContent === '${i} ✓' && window.countAnimations === ${i - 2}`), `increment ${i} did not animate exactly once`);
    }
    await sleep(450);
    const countAfter = await countMetrics();
    const countCost = Object.fromEntries(["TaskDuration", "LayoutCount", "RecalcStyleCount"].map(k => [k, countAfter[k] - countBefore[k]]));
    assert(countCost.TaskDuration < 4, `count main-thread budget exceeded: ${JSON.stringify(countCost)}`);
    assert(countCost.LayoutCount < 500, `count layout budget exceeded: ${JSON.stringify(countCost)}`);
    assert(await browser.evaluate(`window.countToggle === document.querySelector('.stream-tool-run-toggle') && window.countRows.every((row, i) => row === document.querySelectorAll('.stream-tool-row')[i]) && document.querySelectorAll('.stream-tool-count').length === 1 && document.querySelectorAll('.stream-tool-merged-item.merged').length === 23 && document.querySelector('.stream-tool-count').getAnimations().length === 0`), "burst leaked count nodes, remounted rows, or reopened archive");
    console.log("PASS Chrome count increment performance", JSON.stringify(countCost));
    await browser.call("Emulation.setEmulatedMedia", {features:[{name:"prefers-reduced-motion", value:"reduce"}]});
    growingEvents.push(toolEvent("reduced-count", 30));
    const reducedCalls = {...visualBase, turns: visualBase.turns.map(t => ({...t, events:[...growingEvents]}))};
    host.setSession(reducedCalls); host.send({type:"hello", snapshot:makeSnapshot(reducedCalls)});
    await waitFor(() => browser.evaluate(`document.querySelector('.stream-tool-count')?.textContent === '24 ✓'`), "reduced motion lost count update");
    assert(await browser.evaluate(`getComputedStyle(document.querySelector('.stream-tool-count')).animationName === 'none' && window.countAnimations === 21`), "reduced motion animated count");
    await browser.call("Emulation.setEmulatedMedia", {features:[]});
    // Restoring motion may start the existing CSS animation; isolate that media
    // transition from the subsequent Host-update animation under test.
    await sleep(450);
    const beforeBatchAnimations = await browser.evaluate(`window.countAnimations`);
    // One Host snapshot may finish several calls; animate once, including failures.
    const batchFailure = toolEvent("batch-failure", 31);
    batchFailure.payload.payload.status = "failed";
    growingEvents.push(batchFailure, toolEvent("batch-success", 32));
    const batchCalls = {...visualBase, turns: visualBase.turns.map(t => ({...t, events:[...growingEvents]}))};
    host.setSession(batchCalls); host.send({type:"hello", snapshot:makeSnapshot(batchCalls)});
    await waitFor(() => browser.evaluate(`document.querySelector('.stream-tool-count')?.textContent === '25 ✓ | 1 ✗' && window.countAnimations === ${beforeBatchAnimations + 1}`), "batched completion must animate once with exact mixed counts");
    await sleep(450);
    host.send({type:"hello", snapshot:makeSnapshot(batchCalls)});
    await sleep(450);
    assert(await browser.evaluate(`window.countAnimations === ${beforeBatchAnimations + 1} && window.countToggle === document.querySelector('.stream-tool-run-toggle') && document.querySelectorAll('.stream-tool-merged-item.merged').length === 26 && document.querySelector('.stream-tool-count').getAnimations().length === 0`), "mixed batch replay animated, remounted toggle, or reopened history");
    console.log("PASS Chrome count feedback: exact labels, repeat increments, duplicate suppression, stable nodes, reduced motion");
    await browser.evaluate(`document.querySelector('.stream-tool-run-toggle').click(); document.querySelector('.stream-tool-toggle').click();`);
    await waitFor(() => browser.evaluate(`!!document.querySelector('.stream-tool-fold.expanded')`), "output did not open");
    await browser.evaluate(`(() => { const range = document.createRange(); range.selectNodeContents(document.querySelector('.stream-tool-command')); const selection = getSelection(); selection.removeAllRanges(); selection.addRange(range); document.dispatchEvent(new Event('selectionchange')); })()`);
    await browser.evaluate(`document.querySelector('.stream-tool-run-toggle').click()`);
    assert(await browser.evaluate(`!document.querySelector('.stream-tool-merged-item.merged') && !getSelection().isCollapsed`), "merging disrupted text selection");
    await browser.evaluate(`getSelection().removeAllRanges(); document.dispatchEvent(new Event('selectionchange'));`);
    const failed = toolEvent("failed-call", 3); failed.payload.payload.status = "failed";
    await setRound([thoughtEvent("error-thought", "Failure details", 1), failed], "Failure details");
    assert(await browser.evaluate(`!document.querySelector('.stream-tool-fold.expanded')`), "failure must preserve closed default");
    await browser.evaluate(`document.querySelector('.stream-tool-toggle').click()`);
    await waitFor(() => browser.evaluate(`!!document.querySelector('.stream-tool-fold.expanded')`), "failure details cannot expand");
    await browser.evaluate(`document.querySelector('.stream-tool-toggle').click()`);
    await waitFor(() => browser.evaluate(`!document.querySelector('.stream-tool-fold.expanded')`), "failed tool collapse control ineffective");
    await setRound([thoughtEvent("mixed-thought", "Mixed results", 1), toolEvent("success-call", 2.75), failed, thoughtEvent("mixed-reply", "Failure explanation", 4)], "Mixed results");
    await waitFor(() => contains(".stream-tool-run-toggle", "工具 1 ✓ | 1 ✗"), "mixed result counts missing");
    assert(await browser.evaluate(`(() => {
      const bar = document.querySelector('.stream-tool-merged-item.merged .stream-tool-row');
      const rail = getComputedStyle(bar, '::before');
      const height = bar && bar.getBoundingClientRect().height;
      return !!bar && height >= 28 && height <= 40 && rail.width === '1px' && rail.position === 'absolute';
    })()`), "retired steps must render as ~32px summary bars on the 1px step timeline");
    await waitFor(() => browser.evaluate(`document.querySelectorAll('.stream-tool-merged-item.merged').length === 2`), "failed call not merged with adjacent success");
    assert(await browser.evaluate(`document.querySelector('.stream-tool-run-toggle > span')?.textContent === '工具' && [...document.querySelector('.stream-tool-run-toggle').querySelectorAll('span')].every(n => getComputedStyle(n).fontWeight === '400')`), "tools label and counts must use normal weight");
    await browser.evaluate(`document.querySelector('.stream-tool-run-toggle').click()`);
    await waitFor(() => browser.evaluate(`!document.querySelector('.stream-tool-merged-item.merged')`), "merged failure rows cannot reopen");
    for (const width of [390, 768]) {
      await browser.call("Emulation.setDeviceMetricsOverride", {width, height:844, deviceScaleFactor:1, mobile:false});
      assert(await browser.evaluate(`document.documentElement.scrollWidth <= window.innerWidth + 1`), `horizontal overflow at ${width}px`);
    }
    await browser.call("Emulation.clearDeviceMetricsOverride");
    assert(await browser.evaluate(`document.querySelector('.stream-tool-run-toggle').getAttribute('aria-expanded') === 'true' && !!document.querySelector('.stream-tool-run-toggle > svg.lucide-minus')`), "expanded tools must display minus");
    await browser.evaluate(`document.querySelector('.stream-tool-run-toggle').click()`);
    assert(await browser.evaluate(`document.querySelector('.stream-tool-run-toggle').getAttribute('aria-expanded') === 'false' && !!document.querySelector('.stream-tool-run-toggle > svg.lucide-plus')`), "collapsed tools must display plus");
    await browser.evaluate(`localStorage.setItem("timem-web-tool-result-status-v1", "false"); window.dispatchEvent(new StorageEvent("storage", {key:"timem-web-tool-result-status-v1"}));`);
    await waitFor(() => contains(".stream-tool-run-toggle", "2 已完成"), "neutral folded count missing");
    assert(await browser.evaluate(`[...document.querySelectorAll('.stream-tool-status')].every(n => n.textContent === '已完成' && n.getAttribute('aria-label') === '已完成')`), "neutral rows leaked success/failure visually or accessibly");
    await browser.evaluate(`localStorage.setItem("timem-web-tool-result-status-v1", "true"); window.dispatchEvent(new StorageEvent("storage", {key:"timem-web-tool-result-status-v1"}));`);
    await waitFor(() => contains(".stream-tool-run-toggle", "1 ✓ | 1 ✗"), "result preference did not update mounted rows");
    console.log("PASS Chrome visual interaction: stable completed groups, selection protection, failure toggle, 390/768px overflow");
    console.log("PASS Chrome continuous stream: multi-round DOM stability, completed adjacency merge/reopen, terminal animation and interruption archive");
    console.log("PASS Chrome interim continuity: deduplicated deliveries, earlier thought/answers retained, reload and typography");
    console.log("PASS Chrome work collapse: both modes, completion/interruption, reload, manual expansion");
    console.log("PASS Chrome round continuity: earlier thoughts/tools retained, no working archive, terminal trailer removed");
    console.log("PASS Chrome provisional UI: default off, midstream enable, response/chat updates, network interruption, reload snapshot, retraction");
  } finally { await browser.close(); await host.close(); }
}
await main();
