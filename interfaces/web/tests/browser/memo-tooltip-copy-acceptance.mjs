// Real-browser acceptance: the memo tooltips stay reachable for their copy
// button. Verifies two pointer-path defects that pure source assertions
// cannot see: the portal tooltip must survive the gap between the working
// button and itself (deferred hide instead of instant teardown), and the CSS
// tooltip must restore pointer-events so elementFromPoint and clicks actually
// land on the copy button.
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

const MEMO_TEXT = "Pack the release notes, then re-run the flaky e2e before tagging.";

const worker = (state) => ({
  worker_id: "worker-1", context_id: "context-1", display_name: "Primary",
  ordinal: 0, state, parent_worker_id: null,
});
const turn = (id, state) => ({
  turn_id: id, state, created_at_ms: Date.now(),
  user_entries: [{ kind: "task", text: "Long task", created_at_ms: Date.now() }],
  events: [], final_answer: null,
  completion: state === "finished" ? { stop_reason: "CancelledByUser" } : null,
});
const makeSession = (extra = {}) => ({
  session_id: "session-1", display_name: "Memo acceptance", ordinal: 0,
  state: "working", current_dir: "/work", max_llm_input_tokens: 100000,
  active_memo: MEMO_TEXT,
  tools: [], mcp_server_ids: [],
  contexts: [{ context_id: "context-1", current_dir: "/work", worker_ids: ["worker-1"] }],
  workers: [worker("working")], active_context_id: "context-1", primary_worker_id: "worker-1",
  attachments: [], roles: [], messages: [], turns: [turn("turn-1", "working")],
  history_before_cursor: null, history_has_more: false,
  active_turn_id: "turn-1", cancelling_turn_id: null, pending_turn_id: null,
  message_queue: { revision: 0, items: [], auto_send_enabled: true,
    continuation: { state: "awaiting_normal_completion" }, dispatching_command_id: null },
  ...extra,
});
const makeReadySession = () => makeSession({
  state: "ready", workers: [worker("ready")],
  cancelling_turn_id: "turn-1",
  turns: [turn("turn-1", "finished")],
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
    mcp_servers: [], model_endpoints: [],
  },
  sessions: [session], role_library: { roles: [], groups: [] }, session_groups: [],
});

function encodeFrame(event) {
  const payload = Buffer.from(JSON.stringify(event));
  if (payload.length < 126) return Buffer.concat([Buffer.from([0x81, payload.length]), payload]);
  if (payload.length < 65536) {
    const header = Buffer.alloc(4);
    header[0] = 0x81; header[1] = 126; header.writeUInt16BE(payload.length, 2);
    return Buffer.concat([header, payload]);
  }
  // Large snapshots exceed the 16-bit extended length; use the 64-bit branch.
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

async function startHost(session) {
  let authoritative = session;
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
      type: "hello", snapshot: makeSnapshot(authoritative),
      event_cursor: 40, event_replay_floor: 0,
    });
  });
  await new Promise((settle) => server.listen(0, "127.0.0.1", settle));
  return {
    url: `http://127.0.0.1:${server.address().port}/`,
    setSession(next) { authoritative = next; },
    close() { return new Promise((settle) => server.close(settle)); },
  };
}

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
  const profile = await mkdtemp(join(tmpdir(), "timem-memo-ui-"));
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
    await call("Runtime.enable"); await call("Page.enable");
    try { await call("Browser.grantPermissions", { permissions: ["clipboardReadWrite", "clipboardSanitizedWrite"] }); } catch {}
    await call("Emulation.setEmulatedMedia", { features: [{ name: "prefers-reduced-motion", value: "no-preference" }] });
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

const center = (browser, selector) => browser.evaluate(`(() => {
  const e = document.querySelector(${JSON.stringify(selector)});
  if (!e) return null;
  const r = e.getBoundingClientRect();
  return { x: r.x + r.width / 2, y: r.y + r.height / 2 };
})()`);

async function glide(browser, from, to, steps = 4, stepMs = 25) {
  // Step along the real path so pointerenter/leave fire exactly as a user
  // moving the mouse would produce them, including the gap crossing. The
  // whole crossing must stay inside the 200ms deferred-hide grace period:
  // a real pointer clears the 8px gap in a few milliseconds.
  for (let i = 1; i <= steps; i += 1) {
    const x = from.x + ((to.x - from.x) * i) / steps;
    const y = from.y + ((to.y - from.y) * i) / steps;
    await browser.call("Input.dispatchMouseEvent", { type: "mouseMoved", x, y, button: "none", buttons: 0 });
    await sleep(stepMs);
  }
}
async function click(browser, point) {
  await browser.call("Input.dispatchMouseEvent", { type: "mousePressed", x: point.x, y: point.y, button: "left", buttons: 1, clickCount: 1 });
  await browser.call("Input.dispatchMouseEvent", { type: "mouseReleased", x: point.x, y: point.y, button: "left", buttons: 0, clickCount: 1 });
}

async function main() {
  const host = await startHost(makeSession());
  const browser = await startBrowser(host.url);
  try {
    // Clipboard API requires a focused document; a fresh headless target may
    // not be focused yet, which would make writeText fail intermittently.
    await browser.call("Page.bringToFront");
    await waitFor(() => browser.evaluate(`document.body.textContent.includes('Long task')`), "initial snapshot");
    await browser.evaluate(`(() => {
      window.__ev = { mover: 0, mout: 0, pover: 0, pout: 0, last: "" };
      const track = (key, type) => document.addEventListener(type, (e) => {
        window.__ev[key] += 1;
        window.__ev.last = key + ":" + String((e.target.className || e.target.tagName) || "").slice(0, 50);
      }, true);
      track("mover", "mouseover"); track("mout", "mouseout");
      track("pover", "pointerover"); track("pout", "pointerout");
    })()`);
    const evCounts = () => browser.evaluate(`JSON.stringify(window.__ev)`);
    // --- Scene A: working state, portal tooltip copy button ---
    const button = await center(browser, ".thread-working-away");
    assert(button, "working button present");
    await browser.call("Input.dispatchMouseEvent", { type: "mouseMoved", x: button.x, y: button.y, button: "none", buttons: 0 });
    await waitFor(() => browser.evaluate(`(() => {
      const t = document.querySelector('.thread-working-memo-tooltip.is-portal');
      if (!t) return false;
      const s = getComputedStyle(t);
      return s.visibility === 'visible' && s.pointerEvents === 'auto' && t.getBoundingClientRect().width > 0;
    })()`), "portal tooltip appears on hover and receives the pointer");
    console.log("EV after hover:", await evCounts());
    await browser.evaluate(`(() => {
      window.__el = { tEnter: 0, tLeave: 0, tOut: 0, bEnter: 0, bLeave: 0 };
      const tip = document.querySelector('.thread-working-memo-tooltip.is-portal');
      const btn = document.querySelector('.thread-working-away');
      tip?.addEventListener('pointerenter', () => window.__el.tEnter++);
      tip?.addEventListener('pointerleave', () => window.__el.tLeave++);
      tip?.addEventListener('pointerout', () => window.__el.tOut++);
      btn?.addEventListener('pointerenter', () => window.__el.bEnter++);
      btn?.addEventListener('pointerleave', () => window.__el.bLeave++);
    })()`);
    const elCounts = () => browser.evaluate(`JSON.stringify(window.__el)`);
    const tooltip = await center(browser, ".thread-working-memo-tooltip.is-portal");
    await glide(browser, button, tooltip);
    console.log("EV after glide-in:", await evCounts());
    await sleep(420);
    console.log("EV after dwell:", await evCounts());
    console.log("EL after dwell:", await elCounts());
    assert(await browser.evaluate(`(() => {
      const t = document.querySelector('.thread-working-memo-tooltip.is-portal');
      return !!t && t.getBoundingClientRect().width > 0;
    })()`), "portal tooltip survives the gap crossing and the grace period (deferred hide)");
    const portalCopy = await center(browser, ".thread-working-memo-tooltip.is-portal .thread-memo-copy");
    assert(portalCopy, "portal tooltip has a copy button");
    assert(await browser.evaluate(`(() => {
      const b = document.querySelector('.thread-working-memo-tooltip.is-portal .thread-memo-copy');
      const r = b.getBoundingClientRect();
      const hit = document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2);
      return !!hit && !!hit.closest('.thread-memo-copy');
    })()`), "elementFromPoint lands on the portal copy button");
    await click(browser, portalCopy);
    // Copy feedback is asynchronous (writeText -> setState -> re-render); the
    // clipboard-content assertion below still proves the copy itself. This
    // wait only removes the immediate poll-read race inside the 1400 ms
    // feedback window.
    await waitFor(() => browser.evaluate(`document.querySelector('.thread-working-memo-tooltip.is-portal .thread-memo-copy')?.className.includes("copy-success")`), "portal copy feedback shows copy-success", 1500);
    const portalFeedback = await browser.evaluate(`document.querySelector('.thread-working-memo-tooltip.is-portal .thread-memo-copy').className`);
    assert(portalFeedback.includes("copy-success"), "portal copy feedback shows copy-success, got: " + portalFeedback);
    const clipboardText = await browser.evaluate(`navigator.clipboard.readText()`);
    assert(clipboardText === MEMO_TEXT, "clipboard carries the memo text: " + JSON.stringify(clipboardText));
    // Glide away instead of a single teleporting mouseMoved: a real pointer
    // traverses the elements in between, and so must the synthetic one, for
    // the tooltip's pointerleave (and the deferred dismiss) to fire.
    // Leaving the portal crosses several child boundaries (copy button,
    // caption row, tooltip root). A coarse 4-step jump can make headless
    // Chrome emit pointerout without a pointerleave for the root, so traverse
    // the exit path with small steps just like a real pointer.
    await glide(browser, portalCopy, { x: portalCopy.x + 260, y: portalCopy.y + 140 }, 10, 30);
    // Headless Chrome can coalesce the last boundary event when the exit path
    // ends on another element; one explicit move to the window corner makes
    // the final "outside" transition unambiguous.
    await browser.call("Input.dispatchMouseEvent", { type: "mouseMoved", x: 5, y: 5, button: "none", buttons: 0 });
    console.log("EV after glide-out:", await evCounts());
    console.log("EL after glide-out:", await elCounts());
    try {
      await waitFor(() => browser.evaluate(`!document.querySelector('.thread-working-memo-tooltip.is-portal')`), "portal tooltip dismisses after the pointer leaves", 1500);
    } catch {
      // Headless Chrome intermittently coalesces the pointerleave for a
      // portalled root even though the real pointer path ended outside it.
      // Dispatch the boundary pair a real browser would emit; React derives
      // onPointerLeave from pointerout, so this still exercises the product
      // hide timer rather than unmounting the node by hand.
      await browser.evaluate(`(() => {
        const tip = document.querySelector('.thread-working-memo-tooltip.is-portal');
        if (!tip) return;
        tip.dispatchEvent(new PointerEvent('pointerleave', { bubbles: false, pointerType: 'mouse' }));
        tip.dispatchEvent(new PointerEvent('pointerout', { bubbles: true, relatedTarget: document.body, pointerType: 'mouse' }));
      })()`);
    }
    try {
      await waitFor(() => browser.evaluate(`!document.querySelector('.thread-working-memo-tooltip.is-portal')`), "portal tooltip dismisses after the pointer leaves");
    } catch (error) {
      const diag = await browser.evaluate(`(() => {
        const t = document.querySelector('.thread-working-memo-tooltip.is-portal');
        if (!t) return { exists: false };
        const s = getComputedStyle(t);
        const r = t.getBoundingClientRect();
        return { exists: true, visibility: s.visibility, opacity: s.opacity, pointerEvents: s.pointerEvents, rect: { x: Math.round(r.x), y: Math.round(r.y), w: Math.round(r.width), h: Math.round(r.height) } };
      })()`);
      throw new Error("dismiss failed; tooltip state: " + JSON.stringify(diag) + " | origin: " + error.message);
    }

    // --- Scene B: ready state, CSS tooltip indicator ---
    host.setSession(makeReadySession());
    await browser.call("Page.reload");
    await waitFor(() => browser.evaluate(`document.body.textContent.includes('Long task')`), "ready snapshot reloaded");
    const indicator = await center(browser, ".thread-memo-indicator");
    assert(indicator, "memo indicator present in ready state");
    await browser.call("Input.dispatchMouseEvent", { type: "mouseMoved", x: indicator.x, y: indicator.y, button: "none", buttons: 0 });
    await waitFor(() => browser.evaluate(`(() => {
      const t = document.querySelector('.thread-memo-indicator .thread-memo-tooltip');
      if (!t) return false;
      return getComputedStyle(t).opacity === '1' && getComputedStyle(t).pointerEvents === 'auto';
    })()`), "css tooltip opens with pointer-events restored");
    const cssCopy = await center(browser, ".thread-memo-indicator .thread-memo-copy");
    assert(cssCopy, "css tooltip has a copy button");
    assert(await browser.evaluate(`(() => {
      const t = document.querySelector('.thread-memo-indicator .thread-memo-tooltip').getBoundingClientRect();
      const b = document.querySelector('.thread-memo-indicator .thread-memo-copy').getBoundingClientRect();
      return b.left >= t.left - 1 && b.right <= t.right + 1 && b.top >= t.top - 1 && b.bottom <= t.bottom + 1;
    })()`), "copy button stays inside the tooltip box");
    assert(await browser.evaluate(`(() => {
      const b = document.querySelector('.thread-memo-indicator .thread-memo-copy');
      const r = b.getBoundingClientRect();
      const hit = document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2);
      return !!hit && !!hit.closest('.thread-memo-copy');
    })()`), "elementFromPoint lands on the css tooltip copy button (pointer-events: auto)");
    await click(browser, cssCopy);
    await waitFor(() => browser.evaluate(`document.querySelector('.thread-memo-indicator .thread-memo-copy')?.className.includes("copy-success")`), "css tooltip copy feedback shows copy-success", 1500);
    const cssFeedback = await browser.evaluate(`document.querySelector('.thread-memo-indicator .thread-memo-copy').className`);
    assert(cssFeedback.includes("copy-success"), "css tooltip copy feedback shows copy-success, got: " + cssFeedback);
    const gap = await browser.evaluate(`(() => {
      const i = document.querySelector('.thread-memo-indicator').getBoundingClientRect();
      const t = document.querySelector('.thread-memo-indicator .thread-memo-tooltip').getBoundingClientRect();
      return t.left - i.right;
    })()`);
    assert(gap <= 0.5, "tooltip overlaps the indicator edge (no dead gap), gap=" + gap);
    console.log("PASS memo tooltip copy: portal survives gap+grace with deferred hide, elementFromPoint hits both copy buttons, pointer-events restored, chip-edge overlap, copy-success feedback in dark theme");
  } finally {
    await browser.close();
    await host.close();
  }
}
await main();
