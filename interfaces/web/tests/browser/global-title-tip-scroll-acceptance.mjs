// Real-browser acceptance: the global instant title tooltip (GlobalTitleTip)
// must survive scroll-heavy regions. Autoscrolling areas (streaming thread
// output, anchored lists) emit scroll events every frame; hiding the tip on
// any scroll killed it within one activation frame, and with the pointer at
// rest no pointerover ever revived it, so hover hints silently disappeared in
// exactly those busy regions while click (focusin) still worked. The fix
// re-anchors the tip on scroll/resize: it follows the hovered element while
// the element stays under the pointer, hides when the element scrolls away,
// and keeps the plain pointerout and focusin paths intact.
import { spawn } from "node:child_process";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { createHash } from "node:crypto";
import { createServer } from "node:http";
import { existsSync } from "node:fs";
import { extname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { createStartupDiagnostics } from "./browser-startup.mjs";

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
  throw new Error(`Chrome/Chromium not found; checked: ${chromeCandidates.join(", ")}`);
}
const sleep = (ms) => new Promise((settle) => setTimeout(settle, ms));
const assert = (condition, message) => { if (!condition) throw new Error(message); };
async function waitFor(check, message, timeout = 12000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    try { if (await check()) return; } catch {}
    await sleep(40);
  }
  throw new Error("TIMEOUT: " + message);
}

const SESSION_NAME = "Scroll tip acceptance";

const worker = (state) => ({
  worker_id: "worker-1", context_id: "context-1", display_name: "Primary",
  ordinal: 0, state, parent_worker_id: null,
});
const makeSession = () => ({
  session_id: "session-1", display_name: SESSION_NAME, ordinal: 0,
  state: "working", current_dir: "/work", max_llm_input_tokens: 100000,
  tools: [], mcp_server_ids: [],
  contexts: [{ context_id: "context-1", current_dir: "/work", worker_ids: ["worker-1"] }],
  workers: [worker("working")], active_context_id: "context-1", primary_worker_id: "worker-1",
  attachments: [], roles: [], messages: [], turns: [],
  history_before_cursor: null, history_has_more: false,
  active_turn_id: null, cancelling_turn_id: null, pending_turn_id: null,
  message_queue: { revision: 0, items: [], auto_send_enabled: true,
    continuation: { state: "awaiting_normal_completion" }, dispatching_command_id: null },
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
    mcp_servers: [], model_endpoints: [],
  },
  sessions: [makeSession()], role_library: { roles: [], groups: [] }, session_groups: [],
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
      if (masked) offset += offset === 4 ? 4 : 4;
      if (buffer.length < offset + length) return;
      const payload = buffer.subarray(offset, offset + length);
      buffer = buffer.subarray(offset + length);
      if (mask) for (let i = 0; i < payload.length; i += 1) payload[i] ^= mask[i % 4];
      if (opcode === 0x8) { socket.end(); return; }
      if (opcode === 0x1) onJson(JSON.parse(payload.toString("utf8")));
    }
  });
  return { send: (event) => socket.write(encodeFrame(event)) };
}

async function startHost() {
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
    socket.write("HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n" +
      `Sec-WebSocket-Accept: ${accept}\r\n\r\n`);
    let peer;
    peer = makePeer(socket, (command) => {
      if (!command.command_id) return;
      peer.send({ type: "command_ack", command_id: command.command_id, status: "accepted" });
      peer.send({ type: "command_ack", command_id: command.command_id, status: "committed" });
    });
    peers.add(peer);
    socket.on("error", () => peers.delete(peer));
    socket.on("close", () => peers.delete(peer));
    peer.send({
      type: "hello", snapshot: makeSnapshot(),
      event_cursor: 40, event_replay_floor: 0,
    });
  });
  await new Promise((settle) => server.listen(0, "127.0.0.1", settle));
  return {
    url: `http://127.0.0.1:${server.address().port}/`,
    close() { return new Promise((settle) => server.close(settle)); },
  };
}

const pageErrors = [];

async function waitForProcessExit(child, timeoutMs) {
  if (child.exitCode !== null) return true;
  return Promise.race([
    new Promise((settle) => child.once("exit", () => settle(true))),
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
    const [portLine] = (await readFile(join(profile, "DevToolsActivePort"), "utf8")).trim().split("\n");
    const port = Number(portLine);
    return Number.isInteger(port) && port > 0 ? port : null;
  } catch {
    return null;
  }
}

async function startBrowser(url) {
  const profile = await mkdtemp(join(tmpdir(), "timem-title-tip-"));
  const child = spawn(chrome, [
    "--remote-debugging-port=0", `--user-data-dir=${profile}`,
    "--headless=new", "--no-sandbox", "--disable-dev-shm-usage",
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
    await new Promise((settle, reject) => {
      socket.addEventListener("open", settle, { once: true });
      socket.addEventListener("error", reject, { once: true });
    });
    let sequence = 0;
    const requests = new Map();
    socket.addEventListener("message", ({ data }) => {
      const message = JSON.parse(String(data));
      if (message.method === "Runtime.exceptionThrown") {
        pageErrors.push(message.params?.exceptionDetails?.text ?? "exception");
      }
      if (message.method === "Log.entryAdded" && message.params?.entry?.level === "error") {
        pageErrors.push(String(message.params.entry.text).slice(0, 200));
      }
      if (!message.id || !requests.has(message.id)) return;
      const { resolve: settle, reject } = requests.get(message.id);
      requests.delete(message.id);
      if (message.error) reject(new Error(message.error.message));
      else settle(message.result);
    });
    const call = (method, params = {}) => new Promise((settle, reject) => {
      const id = ++sequence;
      requests.set(id, { resolve: settle, reject });
      socket.send(JSON.stringify({ id, method, params }));
    });
    await call("Runtime.enable"); await call("Page.enable"); await call("Log.enable");
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

const centerOf = (browser, selector) => browser.evaluate(`(() => {
  const e = document.querySelector(${JSON.stringify(selector)});
  if (!e) return null;
  const r = e.getBoundingClientRect();
  return { x: r.x + r.width / 2, y: r.y + r.height / 2 };
})()`);

const tip = (browser) => browser.evaluate(`(() => {
  const el = document.querySelector(".global-title-tip");
  if (!el) return null;
  const r = el.getBoundingClientRect();
  return { text: el.textContent, top: r.top, left: r.left, height: r.height };
})()`);

async function main() {
  const host = await startHost();
  const browser = await startBrowser(host.url);
  try {
    await waitFor(() => browser.evaluate(`Boolean(document.querySelector(".session-name"))`), "session row rendered");

    // 1. A pointer that lands directly on a titled element shows the tip at once.
    const center = await centerOf(browser, ".session-name");
    assert(center, "session-name present");
    await browser.call("Input.dispatchMouseEvent", { type: "mouseMoved", x: 900, y: 950, button: "none", buttons: 0 });
    await sleep(120);
    await browser.call("Input.dispatchMouseEvent", { type: "mouseMoved", x: center.x, y: center.y, button: "none", buttons: 0 });
    await waitFor(async () => (await tip(browser))?.text === SESSION_NAME, "tip appears on hover");

    // 2. The regression: scroll bursts from autoscrolling regions must not kill the tip.
    await browser.evaluate(`(() => {
      for (let i = 0; i < 12; i += 1) setTimeout(() => document.documentElement.dispatchEvent(new Event("scroll")), i * 25);
      return true;
    })()`);
    await sleep(450);
    const afterBurst = await tip(browser);
    assert(afterBurst && afterBurst.text === SESSION_NAME, "tip survives injected scroll bursts");

    // 3. A real in-place scroll shifts the element but keeps it under the pointer:
    //    the tip must re-anchor to the moved rect (gap 8px above the element).
    const follow = await browser.evaluate(`(() => {
      const list = document.querySelector(".session-list");
      if (!list) return null;
      list.style.overflowY = "auto";
      list.style.maxHeight = "320px";
      const filler = document.createElement("div");
      filler.style.flex = "0 0 900px";
      filler.style.height = "900px";
      list.appendChild(filler);
      const before = document.querySelector(".session-name").getBoundingClientRect().top;
      list.scrollTop = 5;
      const after = document.querySelector(".session-name").getBoundingClientRect().top;
      return { scrolled: list.scrollTop, shift: before - after };
    })()`);
    assert(follow && follow.scrolled > 0 && Math.abs(follow.shift - follow.scrolled) < 2,
      `list actually scrolled (scrollTop=${follow?.scrolled}, shift=${follow?.shift})`);
    await sleep(150);
    const followed = await tip(browser);
    assert(followed && followed.text === SESSION_NAME, "tip survives a real in-place scroll");
    const nameTop = await browser.evaluate(`document.querySelector(".session-name").getBoundingClientRect().top`);
    // The tip floats 8px above the element: its bottom edge tracks rect.top.
    assert(Math.abs((followed.top + followed.height + 8) - nameTop) <= 3,
      `tip re-anchored above the moved element (tip.bottom=${followed.top + followed.height}, expected=${nameTop - 8})`);

    // 4. Once the element scrolls away from the resting pointer, the tip hides.
    await browser.evaluate(`(() => { document.querySelector(".session-list").scrollTop += 60; return true; })()`);
    await sleep(250);
    assert(!(await tip(browser)), "tip hides once the element scrolls away from the pointer");

    // 5. Plain pointer path still works: re-entry revives, leaving hides.
    await browser.evaluate(`(() => { document.querySelector(".session-list").scrollTop = 0; return true; })()`);
    await sleep(150);
    const recenter = await centerOf(browser, ".session-name");
    await browser.call("Input.dispatchMouseEvent", { type: "mouseMoved", x: recenter.x, y: recenter.y, button: "none", buttons: 0 });
    await waitFor(async () => (await tip(browser))?.text === SESSION_NAME, "tip revives on re-entry");
    await browser.call("Input.dispatchMouseEvent", { type: "mouseMoved", x: 900, y: 950, button: "none", buttons: 0 });
    await sleep(250);
    assert(!(await tip(browser)), "tip hides on pointerout");

    // 6. The focusin path (what users resorted to) still shows the tip. A real
    //    click on the brand toggle also collapses the sidebar, whose relayout
    //    correctly hides the tip once the button leaves the pointer; programmatic
    //    focus isolates the activation path itself (same event a keyboard user
    //    produces via Tab navigation).
    await browser.evaluate(`document.querySelector(".brand-logo-toggle").focus()`);
    await waitFor(async () => (await tip(browser))?.text === "Hide session navigation", "tip appears via focusin");

    assert(pageErrors.length === 0, `unexpected page errors: ${pageErrors.slice(0, 3).join(" | ")}`);
    console.log("PASS global-title-tip-scroll-acceptance: hover tip appears, survives scroll bursts, re-anchors on real scroll, hides when the element scrolls away or the pointer leaves, and reappears on click focus.");
  } finally {
    await browser.close();
    await host.close();
  }
}

main().catch((error) => {
  console.error("FAIL global-title-tip-scroll-acceptance:", error?.message ?? error);
  process.exit(1);
});
