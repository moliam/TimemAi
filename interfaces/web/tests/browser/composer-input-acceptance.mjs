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
  return `# Archived final answer ${index}\n\n${parts.join("\n\n")}\n\nhttps://example.com/raw/${index}\n`;
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
  return {
    url: `http://127.0.0.1:${port}/?token=ui`,
    commands,
    send: (event) => { for (const peer of peers) peer.send(event); },
    close: () => server.close(),
  };
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
    "--headless=new", "--disable-backgrounding-occluded-windows", "--disable-renderer-backgrounding", "--disable-background-timer-throttling",  "--no-sandbox", "--disable-dev-shm-usage",
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
    }, () => startup.failure(), 30000);
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

    // 0. 导航列几何：上下箭头 + 工作圆钮三个控件，整列必须比输入框矮，且顶边
    //    与输入框顶边对齐（用户反馈：导航缩小后要跟输入框高度协调）。
    const geometry = await browser.evaluate(`(() => {
      const nav = document.querySelector(".user-message-navigation");
      const composer = document.querySelector(".composer");
      if (!nav || !composer) return null;
      const n = nav.getBoundingClientRect();
      const c = composer.getBoundingClientRect();
      return { controls: nav.children.length, navHeight: n.height, navTop: n.top,
        composerHeight: c.height, composerTop: c.top };
    })()`);
    assert(geometry, "navigation column or composer is missing");
    assert(geometry.controls === 3,
      `navigation column must keep the three nav controls, got ${geometry.controls}`);
    assert(geometry.navHeight < geometry.composerHeight,
      `navigation column ${geometry.navHeight}px must stay shorter than the composer ${geometry.composerHeight}px`);
    assert(Math.abs(geometry.navTop - geometry.composerTop) <= 1,
      `navigation column top ${geometry.navTop} must align with the composer top ${geometry.composerTop}`);
    console.log(
      `navigation geometry: controls=${geometry.controls} nav=${geometry.navHeight}px ` +
      `composer=${geometry.composerHeight}px top_delta=${(geometry.navTop - geometry.composerTop).toFixed(2)}px`,
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
    assert(median <= 120, `median keystroke frame latency ${median.toFixed(1)}ms exceeded 120ms budget`);
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

    // 3. Hovering the previous-message arrow for 1s opens a scrollable list of
    //    this Session's user inputs; clicking an entry jumps the viewport to
    //    that bubble. Guards hover intent, the portal placement above the
    //    composer, list scrolling, and index/anchoring consistency.
    const arrow = await browser.evaluate(`(() => {
      const button = document.querySelector(".user-message-navigation button");
      if (!button || button.disabled) return null;
      const rect = button.getBoundingClientRect();
      return { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 };
    })()`);
    assert(arrow, "previous-message arrow is missing or disabled");

    await browser.call("Input.dispatchMouseEvent", { type: "mouseMoved", x: 700, y: 420, button: "none", buttons: 0 });
    await browser.call("Input.dispatchMouseEvent", { type: "mouseMoved", x: arrow.x, y: arrow.y, button: "none", buttons: 0 });
    await sleep(500);
    assert(!(await exists(".user-message-history")),
      "history panel opened before the 1s hover intent elapsed");
    await waitFor(() => exists(".user-message-history"),
      "history panel did not open after hovering the arrow for 1s", 3000);

    const history = await browser.evaluate(`(() => {
      const panel = document.querySelector(".user-message-history");
      const list = panel?.querySelector(".user-message-history-list");
      const items = [...(list?.querySelectorAll(".user-message-history-item") ?? [])];
      const composer = document.querySelector(".composer");
      return {
        count: items.length,
        previews: items.map((item) => item.querySelector(".user-message-history-text")?.textContent ?? ""),
        scrollable: Boolean(list) && list.scrollHeight > list.clientHeight + 1,
        active: items.findIndex((item) => item.dataset.active === "true"),
        panel: panel ? panel.getBoundingClientRect().toJSON() : null,
        composer: composer ? composer.getBoundingClientRect().toJSON() : null,
        role: panel?.getAttribute("role") ?? null,
      };
    })()`);
    assert(history.role === "dialog", `history panel role must be dialog, got ${history.role}`);
    assert(history.count === HEAVY_TURNS,
      `history panel listed ${history.count} entries, expected ${HEAVY_TURNS}`);
    assert(history.previews[0]?.includes("Archived task 1"),
      `history list is not in Turn order: first preview ${JSON.stringify(history.previews[0])}`);
    assert(history.previews[HEAVY_TURNS - 1]?.includes(`Archived task ${HEAVY_TURNS}`),
      `history list misses the newest turn: last preview ${JSON.stringify(history.previews[HEAVY_TURNS - 1])}`);
    assert(history.scrollable, "history list must scroll when entries exceed the panel height");
    assert(history.active >= 0, "history panel should mark the user input being read");
    assert(history.panel && history.composer && history.panel.bottom <= history.composer.top + 1,
      "history panel must open above the composer");
    assert(history.panel.left >= 0 && history.panel.right <= 1440,
      `history panel overflows the viewport horizontally: ${history.panel.left}..${history.panel.right}`);

    // Theme parity: the portal must not fall back to one theme's surface.
    const themed = await browser.evaluate(`(() => {
      const panel = document.querySelector(".user-message-history");
      const item = panel?.querySelector(".user-message-history-item");
      if (!panel || !item) return null;
      const read = () => ({
        surface: getComputedStyle(panel).backgroundColor,
        text: getComputedStyle(item).color,
      });
      document.documentElement.dataset.theme = "light";
      const light = read();
      document.documentElement.dataset.theme = "dark";
      const dark = read();
      return { light, dark };
    })()`);
    assert(themed, "history panel or item disappeared during the theme check");
    assert(themed.light.surface !== themed.dark.surface,
      `history panel keeps one surface across themes: ${themed.light.surface}`);
    assert(themed.light.surface.startsWith("rgba(255, 255, 255"),
      `history panel light surface is not light: ${themed.light.surface}`);
    assert(themed.light.text === "rgb(29, 53, 66)",
      `history entry light text color is ${themed.light.text}`);

    // Scroll the list, then click the first entry with a real pointer press.
    const targetIndex = 2;
    const target = await browser.evaluate(`(() => {
      const list = document.querySelector(".user-message-history-list");
      const item = list?.querySelectorAll(".user-message-history-item")[${targetIndex}];
      if (!list || !item) return null;
      const listRect = list.getBoundingClientRect();
      const itemRect = item.getBoundingClientRect();
      list.scrollTop += itemRect.top - listRect.top - 8;
      const rect = item.getBoundingClientRect();
      return { x: rect.left + Math.min(48, rect.width / 2), y: rect.top + rect.height / 2,
        text: item.querySelector(".user-message-history-text")?.textContent ?? "" };
    })()`);
    assert(target, "history entry to click is missing");
    assert(target.text.includes(`Archived task ${targetIndex + 1}`),
      `history entry ${targetIndex} shows ${JSON.stringify(target.text)}`);

    await browser.call("Input.dispatchMouseEvent", { type: "mouseMoved", x: target.x, y: target.y, button: "none", buttons: 0 });
    await sleep(120);
    await browser.call("Input.dispatchMouseEvent", { type: "mousePressed", x: target.x, y: target.y, button: "left", buttons: 1, clickCount: 1 });
    await browser.call("Input.dispatchMouseEvent", { type: "mouseReleased", x: target.x, y: target.y, button: "left", buttons: 0, clickCount: 1 });
    await waitFor(() => browser.evaluate(`!document.querySelector(".user-message-history")`),
      "history panel did not close after selecting an entry", 2500);

    // The jump is an eased 180ms scroll; wait for it to land instead of
    // sampling the first animation frame.
    const landed = () => browser.evaluate(`(() => {
      const viewport = document.querySelector(".chat-scroll");
      const anchor = [...document.querySelectorAll('[data-session-timeline-active="true"] [data-user-message-anchor]')][${targetIndex}];
      if (!viewport || !anchor) return null;
      const top = anchor.getBoundingClientRect().top - viewport.getBoundingClientRect().top;
      return { top, text: anchor.textContent ?? "" };
    })()`);
    await waitFor(async () => {
      const state = await landed();
      return Boolean(state) && state.top >= -2 && state.top <= 64;
    }, async () => {
      const state = await landed();
      return `history jump did not land: top=${state?.top?.toFixed?.(1) ?? "missing"} ` +
        `text=${JSON.stringify(state?.text?.slice(0, 60) ?? null)}`;
    }, 3000);
    const jumped = await landed();
    assert(jumped, "target user bubble is missing after the jump");
    assert(jumped.text.includes(`Archived task ${targetIndex + 1}`),
      `jump landed on the wrong bubble: ${JSON.stringify(jumped.text?.slice(0, 60))}`);
    console.log(`history hover jump: entries=${history.count} target_top=${jumped.top.toFixed(1)}px`);

    // Re-entry regression: after a selection the pointer may travel from the
    // panel back to the arrow. A pending close must not swallow that hover, and
    // moving back must re-arm the 1s intent instead of staying suppressed.
    await browser.call("Input.dispatchMouseEvent", { type: "mouseMoved", x: 700, y: 420, button: "none", buttons: 0 });
    await sleep(320);
    assert(!(await exists(".user-message-history")), "history panel stayed open after the pointer left everything");
    const arrowAgain = await browser.evaluate(`(() => {
      const button = document.querySelector(".user-message-navigation button");
      if (!button) return null;
      const rect = button.getBoundingClientRect();
      return { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 };
    })()`);
    assert(arrowAgain, "previous-message arrow disappeared");
    await browser.call("Input.dispatchMouseEvent", { type: "mouseMoved", x: arrowAgain.x, y: arrowAgain.y, button: "none", buttons: 0 });
    await waitFor(() => exists(".user-message-history"),
      "history panel did not reopen after hovering the arrow again", 2500);
    const reopened = await browser.evaluate(`document.querySelectorAll(".user-message-history-item").length`);
    assert(reopened === HEAVY_TURNS,
      `reopened history panel listed ${reopened} entries, expected ${HEAVY_TURNS}`);
    await browser.call("Input.dispatchMouseEvent", { type: "mouseMoved", x: 700, y: 420, button: "none", buttons: 0 });
    await waitFor(() => browser.evaluate(`!document.querySelector(".user-message-history")`),
      "history panel did not close after the pointer left the arrows", 2500);

    // 4. 模型回复里的超链接：链接必须一眼可辨（常显下划线 + 专属色），
    //    外链带出站箭头，明暗两套都要在真实回复背景上达到 WCAG AA。
    const readLink = () => browser.evaluate(`(() => {
      const anchor = document.querySelector(".turn-final-delivery .markdown-body a.markdown-external-link");
      if (!anchor) return null;
      const style = getComputedStyle(anchor);
      // The link paints its own tint band, so the page surface starts above it.
      let node = anchor.parentElement;
      let surface = "rgba(0, 0, 0, 0)";
      while (node) {
        const background = getComputedStyle(node).backgroundColor;
        if (background && background !== "rgba(0, 0, 0, 0)" && background !== "transparent") {
          surface = background;
          break;
        }
        node = node.parentElement;
      }
      // The outbound marker is decorative: it must not enter copied text.
      const selection = window.getSelection();
      const range = document.createRange();
      range.selectNodeContents(anchor);
      selection.removeAllRanges();
      selection.addRange(range);
      const copied = selection.toString();
      selection.removeAllRanges();
      return {
        color: style.color,
        wash: style.backgroundColor,
        surface,
        line: style.textDecorationLine,
        thickness: style.textDecorationThickness,
        offset: style.textUnderlineOffset,
        marker: getComputedStyle(anchor, "::after").content,
        copied,
        text: anchor.textContent,
      };
    })()`);
    const LINK_PROBE = ".turn-final-delivery .markdown-body a.markdown-external-link";
    const CHIP_PROBE = ".turn-final-delivery .markdown-body a.markdown-address-link";
    // Anchor colors transition; slow runners starve the animation timeline
    // with background work, so poll the probe color until it settles (two
    // equal consecutive reads, bounded fallback) instead of a fixed sleep
    // that can sample a mid-flight frame on loaded machines.
    const setTheme = async (theme, probe) => {
      await browser.evaluate(`document.documentElement.dataset.theme = ${JSON.stringify(theme)}`);
      await browser.evaluate(`new Promise(resolve => {
        const selector = ${JSON.stringify(probe)};
        let last = null;
        const started = Date.now();
        const tick = () => {
          const element = document.querySelector(selector);
          const value = element ? getComputedStyle(element).color : "";
          if (value && value === last) { resolve(); return; }
          last = value;
          if (Date.now() - started > 4000) { resolve(); return; }
          setTimeout(tick, 80);
        };
        setTimeout(tick, 80);
      })`);
    };
    await setTheme("dark", LINK_PROBE);
    const darkLink = await readLink();
    await setTheme("light", LINK_PROBE);
    const lightLink = await readLink();
    await setTheme("dark", LINK_PROBE);
    const links = { dark: darkLink, light: lightLink };
    assert(links.dark && links.light, "no outbound link rendered inside a model reply");
    const parseColor = (value) => {
      const parts = (value.match(/[\d.]+/g) ?? []).map(Number);
      return { r: parts[0] ?? 0, g: parts[1] ?? 0, b: parts[2] ?? 0, a: parts[3] ?? 1 };
    };
    const over = (top, bottom) => ({
      r: top.r * top.a + bottom.r * (1 - top.a),
      g: top.g * top.a + bottom.g * (1 - top.a),
      b: top.b * top.a + bottom.b * (1 - top.a),
    });
    const luminance = ({ r, g, b }) => {
      const [red, green, blue] = [r, g, b].map((channel) => {
        const scaled = channel / 255;
        return scaled <= 0.04045 ? scaled / 12.92 : ((scaled + 0.055) / 1.055) ** 2.4;
      });
      return 0.2126 * red + 0.7152 * green + 0.0722 * blue;
    };
    const ratio = (foreground, background) => {
      const [lighter, darker] = [luminance(foreground), luminance(background)].sort((a, b) => b - a);
      return (lighter + 0.05) / (darker + 0.05);
    };
    for (const theme of ["dark", "light"]) {
      const link = links[theme];
      assert(link.line === "underline",
        `${theme} reply link is not underlined: ${link.line}`);
      assert(link.offset !== "auto" && parseFloat(link.offset) >= 2,
        `${theme} reply underline offset is ${link.offset}`);
      assert(link.thickness !== "auto" && parseFloat(link.thickness) >= 2,
        `${theme} reply underline thickness is ${link.thickness}`);
      // At rest the link must already carry a tint band; a reader should not
      // have to hover or compare hues to notice it.
      assert(link.wash && link.wash !== "rgba(0, 0, 0, 0)",
        `${theme} reply link has no resting tint: ${link.wash}`);
      assert(link.marker.includes("\u2197"),
        `${theme} outbound link marker is ${JSON.stringify(link.marker)}`);
      const ink = parseColor(link.color);
      const surface = parseColor(link.surface);
      // The link sits on its own band, so the text has to stay legible against
      // the composited band too, not only against the page surface.
      const band = over(parseColor(link.wash), surface);
      const onSurface = ratio(ink, surface);
      const onBand = ratio(ink, band);
      assert(onSurface >= 4.5,
        `${theme} reply link contrast is ${onSurface.toFixed(2)}:1 on ${link.surface}`);
      assert(onBand >= 4.5,
        `${theme} reply link contrast is ${onBand.toFixed(2)}:1 on its tint band`);
      const spread = Math.max(ink.r, ink.g, ink.b) - Math.min(ink.r, ink.g, ink.b);
      assert(spread >= 48,
        `${theme} reply link ink is not a distinct hue: ${link.color}`);
      console.log(`reply link ${theme}: ${link.color} on ${link.surface} (${onSurface.toFixed(2)}:1), on band ${onBand.toFixed(2)}:1`);
    }
    for (const theme of ["dark", "light"]) {
      assert(links[theme].copied.trim() === links[theme].text.trim(),
        `${theme} outbound marker leaked into copied text: ${JSON.stringify(links[theme].copied)}`);
    }

    // The arrow must be a decoration, not a layout change: toggling it off and
    // on must leave the paragraph's height untouched.
    const markerLayout = await browser.evaluate(`(() => {
      const anchor = document.querySelector(".turn-final-delivery .markdown-body a.markdown-external-link");
      const paragraph = anchor.closest("p") ?? anchor.parentElement;
      const withMarker = paragraph.getBoundingClientRect().height;
      anchor.classList.add("markdown-address-link");
      const withoutMarker = paragraph.getBoundingClientRect().height;
      anchor.classList.remove("markdown-address-link");
      const restored = paragraph.getBoundingClientRect().height;
      return { withMarker, withoutMarker, restored };
    })()`);
    assert(Math.abs(markerLayout.withMarker - markerLayout.withoutMarker) <= 1,
      `outbound marker changes the reply line box: ${markerLayout.withoutMarker}px -> ${markerLayout.withMarker}px`);
    assert(Math.abs(markerLayout.restored - markerLayout.withMarker) <= 1,
      "reply paragraph height does not settle after the marker returns");

    // A URL label becomes a chip: padding, a keyline and rounded corners carry
    // the affordance instead of an underline.
    const readChip = () => browser.evaluate(`(() => {
      const anchor = document.querySelector(".turn-final-delivery .markdown-body a.markdown-address-link");
      if (!anchor) return null;
      const style = getComputedStyle(anchor);
      let node = anchor.parentElement;
      let surface = "rgba(0, 0, 0, 0)";
      while (node) {
        const background = getComputedStyle(node).backgroundColor;
        if (background && background !== "rgba(0, 0, 0, 0)" && background !== "transparent") {
          surface = background;
          break;
        }
        node = node.parentElement;
      }
      return {
        color: style.color,
        wash: style.backgroundColor,
        surface,
        line: style.textDecorationLine,
        borderWidth: style.borderTopWidth,
        borderStyle: style.borderTopStyle,
        borderColor: style.borderTopColor,
        radius: style.borderTopLeftRadius,
        paddingTop: style.paddingTop,
        paddingLeft: style.paddingLeft,
        text: anchor.textContent,
        copied: (() => {
          const selection = window.getSelection();
          const range = document.createRange();
          range.selectNodeContents(anchor);
          selection.removeAllRanges();
          selection.addRange(range);
          const value = selection.toString();
          selection.removeAllRanges();
          return value;
        })(),
      };
    })()`);
    await setTheme("dark", CHIP_PROBE);
    const darkChip = await readChip();
    await setTheme("light", CHIP_PROBE);
    const lightChip = await readChip();
    await setTheme("dark", CHIP_PROBE);
    const chips = { dark: darkChip, light: lightChip };
    assert(chips.dark && chips.light, "no address-link chip rendered inside a model reply");
    for (const theme of ["dark", "light"]) {
      const chip = chips[theme];
      assert(chip.line === "none", `${theme} address chip is still underlined: ${chip.line}`);
      assert(parseFloat(chip.paddingLeft) > 0 && parseFloat(chip.paddingTop) > 0,
        `${theme} address chip has no padding: ${chip.paddingTop} ${chip.paddingLeft}`);
      assert(chip.borderStyle === "solid" && parseFloat(chip.borderWidth) >= 1,
        `${theme} address chip has no keyline: ${chip.borderWidth} ${chip.borderStyle}`);
      assert(parseFloat(chip.radius) > 0, `${theme} address chip has no radius: ${chip.radius}`);
      const ink = parseColor(chip.color);
      const band = over(parseColor(chip.wash), parseColor(chip.surface));
      const onBand = ratio(ink, band);
      assert(onBand >= 4.5,
        `${theme} address chip contrast is ${onBand.toFixed(2)}:1 on ${chip.wash}`);
      assert(chip.copied.trim() === chip.text.trim(),
        `${theme} address chip leaks markup into copied text: ${JSON.stringify(chip.copied)}`);
      console.log(`address chip ${theme}: ${chip.color} on ${chip.wash}, contrast ${onBand.toFixed(2)}:1, radius ${chip.radius}`);
    }

    // Hover must add a second, non-color cue: thicker underline + a wash.
    // The reply sits far down the history, so bring the link into the viewport
    // before the synthetic pointer lands on it.
    await browser.evaluate(`(() => {
      const anchor = document.querySelector(".turn-final-delivery .markdown-body a.markdown-external-link");
      anchor.scrollIntoView({ block: "center" });
    })()`);
    await sleep(220);
    const idle = await browser.evaluate(`(() => {
      const anchor = document.querySelector(".turn-final-delivery .markdown-body a.markdown-external-link");
      const style = getComputedStyle(anchor);
      return { color: style.color, background: style.backgroundColor, thickness: style.textDecorationThickness };
    })()`);
    const hoverPoint = await browser.evaluate(`(() => {
      const anchor = document.querySelector(".turn-final-delivery .markdown-body a.markdown-external-link");
      const rect = anchor.getBoundingClientRect();
      return {
        x: rect.left + rect.width / 2,
        y: rect.top + rect.height / 2,
        inViewport: rect.top >= 0 && rect.bottom <= window.innerHeight,
      };
    })()`);
    assert(hoverPoint.inViewport, "reply link is outside the viewport before hovering");
    await browser.call("Input.dispatchMouseEvent",
      { type: "mouseMoved", x: hoverPoint.x, y: hoverPoint.y, button: "none", buttons: 0 });
    await sleep(220);
    const hovered = await browser.evaluate(`(() => {
      const anchor = document.querySelector(".turn-final-delivery .markdown-body a.markdown-external-link");
      const style = getComputedStyle(anchor);
      return { color: style.color, background: style.backgroundColor, thickness: style.textDecorationThickness };
    })()`);
    assert(hovered.color !== idle.color,
      `reply link color does not react to hover: ${idle.color}`);
    assert(hovered.background !== idle.background,
      `reply link has no hover wash: ${idle.background} -> ${hovered.background}`);
    assert(parseFloat(hovered.thickness) >= parseFloat(idle.thickness),
      `reply link underline thins on hover: ${idle.thickness} -> ${hovered.thickness}`);
    console.log(`reply link hover: ${idle.color} -> ${hovered.color}, wash ${idle.background} -> ${hovered.background}`);
    await browser.call("Input.dispatchMouseEvent",
      { type: "mouseMoved", x: 8, y: 8, button: "none", buttons: 0 });
    await sleep(120);

    // 5. Streaming + slow scroll-up. While a Session is working the viewport
    //    pins itself to the bottom; a slow upward wheel must detach immediately,
    //    and the commits that follow must not pull the reader back down (the
    //    old clamp compensation treated every upward step as a layout clamp).
    await browser.evaluate(`(() => {
      const viewport = document.querySelector(".chat-scroll");
      viewport.scrollTop = viewport.scrollHeight;
    })()`);
    await sleep(150);

    const workingSession = {
      ...session(),
      state: "working",
      active_turn_id: `turn-${HEAVY_TURNS}`,
      turns: session().turns.map((item, index) =>
        index === HEAVY_TURNS - 1 ? { ...item, state: "working" } : item),
    };
    // The host advertises an event cursor, so authoritative state must ride the
    // ordered semantic stream instead of a raw top-level event.
    host.send({
      type: "semantic_event",
      event_seq: 1,
      event: { type: "session_restart_cwd_resolved", session: workingSession },
    });
    await waitFor(
      () => browser.evaluate(`!!document.querySelector(".user-message-navigation .thread-working-away.is-working")`),
      "working Session state did not apply",
    );

    const scrollTopNow = () => browser.evaluate(`document.querySelector(".chat-scroll").scrollTop`);
    const startTop = await scrollTopNow();
    let typed = "";
    for (let step = 0; step < 4; step += 1) {
      const before = await scrollTopNow();
      await browser.call("Input.dispatchMouseEvent", {
        type: "mouseWheel", x: 700, y: 380, deltaX: 0, deltaY: -40,
      });
      // Slow runners batch wheel handling behind background work, so poll
      // until the viewport actually moves instead of sleeping a fixed 90ms;
      // the contract (wheel must scroll the timeline) is unchanged.
      let afterWheel = before;
      const deadline = Date.now() + 2500;
      while (Date.now() < deadline && afterWheel > before - 8) {
        await sleep(60);
        afterWheel = await scrollTopNow();
      }
      assert(afterWheel <= before - 8,
        `slow wheel-up did not move the viewport: ${before} -> ${afterWheel}`);
      // Any commit (typing rerenders the thread) used to re-pin the reader.
      typed += "s";
      await browser.evaluate(`(() => {
        const area = document.querySelector(".composer textarea");
        if (!area) return;
        Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")
          .set.call(area, ${JSON.stringify("slow-scroll")} + ${JSON.stringify(typed)});
        area.dispatchEvent(new Event("input", { bubbles: true }));
      })()`);
      await sleep(420);
      const afterCommit = await scrollTopNow();
      assert(afterCommit <= afterWheel + 2,
        `a streaming-era commit pulled the viewport back: ${afterWheel} -> ${afterCommit}`);
    }
    const endTop = await scrollTopNow();
    assert(endTop < startTop - 120,
      `slow scroll-up only moved ${startTop - endTop}px in four steps`);
    assert(await browser.evaluate(
      `!!document.querySelector(".user-message-navigation .thread-working-away.away-from-bottom")`),
      "working-away indicator missing after scrolling up during work");

    // An explicit jump-to-latest re-attaches the follower.
    await browser.evaluate(`document.querySelector(".user-message-navigation .thread-working-away")?.click()`);
    await waitFor(async () => {
      const viewport = await browser.evaluate(`(() => {
        const node = document.querySelector(".chat-scroll");
        return { top: node.scrollTop, max: node.scrollHeight - node.clientHeight };
      })()`);
      return viewport.max - viewport.top <= 2;
    }, "jump-to-latest did not return to the bottom", 3000);
    console.log(`slow scroll-up detached: ${startTop} -> ${endTop}`);

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
