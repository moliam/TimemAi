// Composer typing acceptance: drives real Chrome against the built dist with
// a scripted WebSocket host and a deliberately heavy history snapshot
// (20 archived turns x 8KB markdown final answers). Guards the write-through
// architecture: keystrokes stay local to the composer field (bounded rAF
// latency per keystroke) and submitting inside the throttle window still
// sends the complete live text.
import { createStartupDiagnostics } from "./browser-startup.mjs";
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
if (!chrome) throw new Error(`Chrome/Chromium not found; checked: ${chromeCandidates.join(", ")}`);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const assert = (condition, message) => { if (!condition) throw new Error(message); };
async function waitFor(check, message, timeout = 10000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    try { if (await check()) return; } catch {}
    await sleep(40);
  }
  throw new Error(typeof message === "function" ? await message() : message);
}

const now = Date.now();
// 15 chars x 8 = exactly 120 keystrokes, mixed CJK and ASCII.
const TYPED = "输入性能验收0123456789".repeat(8);
const HEAVY_TURNS = 20;
const heavyAnswer = (index) => {
  const line = `Turn ${index} archived paragraph with **bold**, \`inline code\`, and a [link](https://example.com/a/${index}).`;
  const parts = [];
  let size = 0;
  while (size < 8 * 1024) { parts.push(line); size += line.length; }
  return `# Archived final answer ${index}\n\n${parts.join("\n\n")}\n`;
};
const turn = (index) => ({
  turn_id: `turn-${index}`, state: "finished", created_at_ms: now - (HEAVY_TURNS - index) * 1000,
  user_entries: [{
    kind: "task", text: `Archived task ${index}`,
    created_at_ms: now - (HEAVY_TURNS - index) * 1000,
  }],
  events: [], final_answer: heavyAnswer(index), completion: null,
});
const worker = () => ({
  worker_id: "worker-1", context_id: "context-1", display_name: "Primary",
  ordinal: 0, state: "ready", parent_worker_id: null,
});
const session = () => ({
  session_id: "session-1", display_name: "Composer input acceptance", ordinal: 0,
  state: "ready", current_dir: "/work", max_llm_input_tokens: 100000,
  tools: [], mcp_server_ids: [], timers: [],
  contexts: [{ context_id: "context-1", current_dir: "/work", worker_ids: ["worker-1"] }],
  workers: [worker()], active_context_id: "context-1", primary_worker_id: "worker-1",
  attachments: [], roles: [], messages: [],
  turns: Array.from({ length: HEAVY_TURNS }, (_, index) => turn(index + 1)),
  history_before_cursor: null, history_has_more: false,
  active_turn_id: null, cancelling_turn_id: null, pending_turn_id: null,
  message_queue: {
    revision: 0, items: [], auto_send_enabled: true,
    continuation: { state: "awaiting_normal_completion" },
    dispatching_command_id: null,
  },
});
const makeSnapshot = () => ({
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
  sessions: [session()], role_library: { roles: [], groups: [] }, session_groups: [],
});

function encodeFrame(event) {
  const payload = Buffer.from(JSON.stringify(event));
  if (payload.length < 126) return Buffer.concat([Buffer.from([0x81, payload.length]), payload]);
  if (payload.length < 65536) {
    const header = Buffer.alloc(4);
    header[0] = 0x81; header[1] = 126; header.writeUInt16BE(payload.length, 2);
    return Buffer.concat([header, payload]);
  }
  // Heavy-history snapshots exceed the 16-bit extended length; use 64-bit.
  const header = Buffer.alloc(10);
  header[0] = 0x81; header[1] = 127; header.writeBigUInt64BE(BigInt(payload.length), 2);
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
  const commands = [];
  const peers = new Set();
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
    const peer = makePeer(socket, (command) => {
      commands.push(command);
      if (!command.command_id) return;
      peer.send({ type: "command_ack", command_id: command.command_id, status: "accepted" });
    });
    peers.add(peer);
    socket.on("close", () => peers.delete(peer));
    peer.send({ type: "hello", snapshot: makeSnapshot(), event_cursor: 0, event_replay_floor: 0 });
  });
  await new Promise((resolveListen) => server.listen(0, "127.0.0.1", resolveListen));
  const port = server.address().port;
  return { url: `http://127.0.0.1:${port}/?token=ui`, commands, close: () => server.close() };
}

async function readDevToolsPort(profile) {
  try {
    const text = await readFile(join(profile, "DevToolsActivePort"), "utf8");
    const port = Number(text.trim().split("\n")[0]);
    return Number.isInteger(port) && port > 0 ? port : null;
  } catch { return null; }
}

async function stopBrowserProcess(child, profile) {
  if (child.exitCode === null) child.kill("SIGTERM");
  for (let i = 0; i < 50 && child.exitCode === null; i += 1) await sleep(20);
  if (child.exitCode === null) child.kill("SIGKILL");
  await rm(profile, { recursive: true, force: true }).catch(() => {});
}

async function startBrowser(url) {
  const profile = await mkdtemp(join(tmpdir(), "timem-composer-ui-"));
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
    await new Promise((resolveOpen, rejectOpen) => {
      socket.addEventListener("open", resolveOpen, { once: true });
      socket.addEventListener("error", rejectOpen, { once: true });
    });
    let sequence = 0;
    const requests = new Map();
    socket.addEventListener("message", ({ data }) => {
      const message = JSON.parse(String(data));
      if (!message.id || !requests.has(message.id)) return;
      const { resolve: resolveRequest, reject: rejectRequest } = requests.get(message.id);
      requests.delete(message.id);
      if (message.error) rejectRequest(new Error(message.error.message));
      else resolveRequest(message.result);
    });
    const call = (method, params = {}) => new Promise((resolveRequest, rejectRequest) => {
      const id = ++sequence;
      requests.set(id, { resolve: resolveRequest, reject: rejectRequest });
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
      async close() { socket.close(); await stopBrowserProcess(child, profile); },
    };
  } catch (error) {
    await stopBrowserProcess(child, profile);
    throw error;
  }
}

const percentile = (sorted, fraction) => sorted[Math.min(sorted.length - 1, Math.floor(fraction * sorted.length))];

async function main() {
  const host = await startHost();
  const browser = await startBrowser(host.url);
  const exists = (selector) => browser.evaluate(`Boolean(document.querySelector(${JSON.stringify(selector)}))`);
  const composerValue = () => browser.evaluate(
    `document.querySelector(".composer textarea")?.value ?? null`,
  );
  try {
    await waitFor(() => exists(".composer textarea"), "composer textarea missing");
    // Wait until the hello snapshot selected the session and archived answers
    // rendered: a disabled composer never receives React change events, and
    // the heavy history must be on screen before typing is measured.
    await waitFor(
      async () =>
        (await browser.evaluate(
          `document.querySelector(".composer textarea")?.disabled === false`,
        )) === true,
      "composer textarea stays disabled before the hello snapshot",
    );
    await waitFor(
      () => browser.evaluate(
        `document.body.innerText.includes("Archived final answer ${HEAVY_TURNS}")`,
      ),
      "heavy history final answers not rendered",
    );

    // 1. Type 120 keystrokes one by one; per keystroke, measure how long the
    // next animation frame takes to arrive after the input event. Keystrokes
    // must stay local to the composer field: only the throttled write-through
    // commits may rerender the heavy thread, so the typical frame stays fast.
    const stats = await browser.evaluate(`(() => new Promise((resolve) => {
      const area = document.querySelector(".composer textarea");
      const text = ${JSON.stringify(TYPED)};
      const latencies = [];
      const start = performance.now();
      const step = (i) => {
        if (i >= text.length) {
          resolve({ latencies, elapsedMs: performance.now() - start, value: area.value });
          return;
        }
        Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")
          .set.call(area, text.slice(0, i + 1));
        const t0 = performance.now();
        area.dispatchEvent(new Event("input", { bubbles: true }));
        requestAnimationFrame(() => {
          latencies.push(performance.now() - t0);
          step(i + 1);
        });
      };
      step(0);
    }))()`);
    const sorted = [...stats.latencies].sort((a, b) => a - b);
    const median = percentile(sorted, 0.5);
    const p90 = percentile(sorted, 0.9);
    const max = sorted[sorted.length - 1];
    const heavyFrames = stats.latencies.filter((ms) => ms >= 150).length;
    console.log(
      `composer typing: keystrokes=${stats.latencies.length} median_ms=${median.toFixed(1)} ` +
      `p90_ms=${p90.toFixed(1)} max_ms=${max.toFixed(1)} heavy_frames(>=150ms)=${heavyFrames} ` +
      `total_ms=${stats.elapsedMs.toFixed(0)}`,
    );
    assert(stats.value === TYPED, `composer value drifted from live text: ${JSON.stringify(stats.value?.slice(-20))}`);
    assert(median <= 40, `median keystroke frame latency ${median.toFixed(1)}ms exceeded 40ms budget`);
    assert(heavyFrames / stats.latencies.length <= 0.2,
      `${heavyFrames}/${stats.latencies.length} keystroke frames were heavy (>=150ms); typing is rerendering the thread`);
    assert(stats.elapsedMs <= 12000, `120 keystrokes took ${stats.elapsedMs.toFixed(0)}ms total`);

    // 2. Submit immediately: inside the 300ms throttle window the committed
    // draft state may lag, but the submit path must read the live text and
    // send the complete typed message.
    await browser.evaluate(
      `document.querySelector(".composer .send-button")?.click()`,
    );
    await waitFor(
      async () => (await composerValue()) === "",
      async () => `composer not cleared after submit: value=${JSON.stringify(await composerValue())}`,
    );
    const submit = [...host.commands].reverse().find((c) => c.type === "turn_submit");
    assert(submit?.text === TYPED,
      `turn_submit inside throttle window missing live text: ${JSON.stringify(submit?.text ?? null)?.slice(0, 80)}`);

    console.log("composer input acceptance: all checks passed");
  } finally {
    await browser.close();
    host.close();
  }
}

main().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
