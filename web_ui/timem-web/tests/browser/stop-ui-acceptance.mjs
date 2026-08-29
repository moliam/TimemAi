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
        // Keep turn_cancel durable until authoritative TurnFinished. This
        // reproduces a reload that receives an older working snapshot while
        // cancellation is already accepted by Host.
        if (command.type !== "turn_cancel")
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
  const exists = (selector) => browser.evaluate(`Boolean(document.querySelector(${JSON.stringify(selector)}))`);
  const contains = (selector, text) => browser.evaluate(
    `[...document.querySelectorAll(${JSON.stringify(selector)})].some((node) => node.textContent?.includes(${JSON.stringify(text)}) === true)`,
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
    await waitFor(() => exists('.turn-assistant-frame.working'), "formal working frame missing before the first process event");
    assert(!(await exists('.turn-starting-status')), "obsolete placeholder working rendered before the first process event");
    assert(
      await contains('.turn-assistant-frame.working .working-label', "working"),
      "formal working header missing before the first process event",
    );
    await browser.evaluate(`(() => {
      document.querySelector('.turn-assistant-frame.working').dataset.acceptanceWorkingFrame = 'stable';
    })()`);
    host.send({
      type: "worker_activity", session_id: "session-1", context_id: "context-1",
      worker_id: "worker-1", turn_id: "turn-1",
      event: { kind: "worker_started", phase: "processing" },
    });
    await waitFor(
      () => exists('.turn-assistant-frame.working .turn-work-item'),
      "first process event did not populate the formal working frame",
    );
    assert(
      await exists('.turn-assistant-frame.working[data-acceptance-working-frame="stable"]'),
      "first process event replaced the formal working frame",
    );
    assert(!(await exists('.turn-starting-status')), "obsolete placeholder working returned after the first process event");

    // Irregular action: two immediate clicks must still emit one targeted cancellation.
    await browser.evaluate(`(() => {
      const button = document.querySelector('button[aria-label="Cancel current turn"]');
      button.click(); button.click();
    })()`);
    await waitFor(() => sent("turn_cancel").length === 1, "rapid double Stop did not emit exactly one turn_cancel");
    assert(sent("turn_cancel")[0].target_command_id === "submit-original", "turn_cancel target mismatch");

    // Clicking Stop is transport-only. Until Host returns its authoritative
    // Session, the browser must keep rendering the previous Host state.
    assert(await exists('.session-working-icon[aria-label="Session working"]'), "Stop click changed the spinner before Host state arrived");
    assert(
      !(await contains('.completion-card[aria-label="Turn completion statistics"]', "Cancelled")),
      "Stop click rendered Cancelled before Host state arrived",
    );

    const cancelledSession = makeCancelledSession();
    host.setSession(cancelledSession);
    host.send({
      type: "turn_cancelling", session: cancelledSession,
      target_command_id: "submit-original",
    });

    await waitFor(async () => !(await exists('.session-working-icon[aria-label="Session working"]')), "Host-confirmed Session did not stop the spinner");
    await waitFor(
      () => contains('.completion-card[aria-label="Turn completion statistics"]', "Cancelled"),
      "Host-confirmed Session did not render Cancelled",
    );

    // A new message is a normal direct submit. The runtime may still hold Core
    // execution behind its private terminal barrier, but the browser must not
    // expose that as a waiting queue.
    await enterMessage("First after Stop");
    await waitFor(
      () => sent("turn_submit").some((command) => command.text === "First after Stop"),
      "post-Stop task was not submitted immediately",
    );
    assert(
      !(await contains(".queued-message-list", "First after Stop")),
      "post-Stop task leaked into the visible waiting queue",
    );
    const nextSubmit = sent("turn_submit").find(
      (command) => command.text === "First after Stop",
    );
    host.send({
      type: "turn_updated", session_id: "session-1",
      turn: {
        ...turn("turn-2", "First after Stop"),
        state: "pending",
        user_entries: [{
          kind: "task", text: "First after Stop",
          command_id: nextSubmit.command_id, created_at_ms: Date.now(),
        }],
      },
    });
    await waitFor(
      () => contains(".turn-user-entry", "First after Stop"),
      "accepted post-Stop task was not rendered as an independent Turn",
    );

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

    // Refresh/reconnect derives presentation only from Host snapshot. The
    // browser outbox must not synthesize or override cancellation state.
    host.setSession(cancelledSession);
    await browser.call("Page.reload", { ignoreCache: true });
    await waitFor(
      () => contains('.completion-card[aria-label="Turn completion statistics"]', "Cancelled"),
      "Host snapshot did not restore Cancelled after refresh",
    );
    assert(!(await exists('.session-working-icon[aria-label="Session working"]')), "Host cancelled snapshot revived spinner");

    host.send({
      type: "turn_finished", session_id: "session-1", turn_id: "turn-1",
      outcome: { completion: { stop_reason: "CancelledByUser", elapsed_ms: 100 } },
    });
    await waitFor(() => contains('.completion-card[aria-label="Turn completion statistics"]', "Cancelled"), "chat did not render terminal Cancelled state");

    // Events already in flight after terminal completion must not revive the
    // Session spinner while the chat remains Cancelled.
    host.send({
      type: "turn_started", session_id: "session-1", context_id: "context-1",
      worker_id: "worker-1", turn: turn("turn-1"),
    });
    host.send({
      type: "worker_activity", session_id: "session-1", context_id: "context-1",
      worker_id: "worker-1", turn_id: "turn-1", event: { kind: "model_request", round: 2 },
    });
    await sleep(180);
    assert(await contains('.completion-card[aria-label="Turn completion statistics"]', "Cancelled"), "late event erased terminal Cancelled state");
    assert(!(await exists('.session-working-icon[aria-label="Session working"]')), "late post-finish event revived Session spinner");

    const cancelCommand = sent("turn_cancel")[0];
    host.send({ type: "command_ack", command_id: cancelCommand.command_id, status: "committed" });

    // Duplicate terminal events cannot alter the already terminal presentation
    // or redeliver the accepted next task.
    const submittedCount = sent("turn_submit").filter(
      (command) => command.text === "First after Stop",
    ).length;
    host.send({
      type: "turn_finished", session_id: "session-1", turn_id: "turn-1",
      outcome: { completion: { stop_reason: "CancelledByUser", elapsed_ms: 100 } },
    });
    await new Promise((resolve) => setTimeout(resolve, 180));
    assert(
      sent("turn_submit").filter((command) => command.text === "First after Stop").length === submittedCount,
      "duplicate completion redelivered the post-Stop task",
    );
    assert(
      await contains('.completion-card[aria-label="Turn completion statistics"]', "Cancelled"),
      "duplicate completion changed the Cancelled presentation",
    );

    console.log("PASS real Chrome Stop UI acceptance");
    console.log("- formal working frame is present before the first process event and remains the same DOM node afterward");
    console.log("- obsolete placeholder working never renders");
    console.log("- double Stop -> one targeted turn_cancel");
    console.log("- Stop click does not change business UI before Host state arrives");
    console.log("- Host-confirmed and reconnect Session state render Cancelled");
    console.log("- post-confirmation late Core events cannot revive spinner");
    console.log("- the next task submits directly and never appears in the waiting queue");
    console.log("- refresh and duplicate completion preserve the terminal presentation");
  } finally {
    await browser.close();
    await host.close();
  }
}

await main();
