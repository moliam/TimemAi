// Real-browser acceptance for Session Insights: compact header chip, click to
// open the floating panel, drag the panel, reorder rows, pin/collapse, and
// local preference persistence. Pure source assertions cannot see pointer or
// layout defects, so this drives a real Chromium target against a mock Host.
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
  "/usr/bin/google-chrome",
  "/usr/bin/google-chrome-stable",
  "/usr/bin/chromium",
  "/usr/bin/chromium-browser",
].filter(Boolean);
const chrome = chromeCandidates.find((candidate) => existsSync(candidate));
if (!chrome) throw new Error(`Chrome/Chromium not found; checked: ${chromeCandidates.join(", ")}`);
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

const worker = () => ({
  worker_id: "worker-1", context_id: "context-1", display_name: "Primary",
  ordinal: 0, state: "working", parent_worker_id: null,
});
const modelEvents = () => {
  const events = [];
  for (let round = 1; round <= 16; round += 1) {
    events.push({
      event_id: `response-${round}`, source: "worker_activity", created_at_ms: 1000 + round,
      payload: { kind: "model_response", round, usage: { completion_tokens: 40 + round * 10, prompt_tokens: 1000 + round, cached_tokens: 600, reasoning_tokens: 20, llm_calls: 1, tool_calls: 2 } },
    });
    events.push({
      event_id: `completed-${round}`, source: "worker_activity", created_at_ms: 1001 + round,
      payload: { kind: "model_request_completed", round, latency_ms: 500 + round * 10, ttft_ms: 100 + round * 20 },
    });
  }
  return events;
};
const makeSession = () => ({
  session_id: "session-insights", display_name: "Insights acceptance", ordinal: 0,
  state: "working", current_dir: "/work", max_llm_input_tokens: 16000,
  tools: [], mcp_server_ids: [],
  contexts: [{ context_id: "context-1", current_dir: "/work", worker_ids: ["worker-1"] }],
  workers: [worker()], active_context_id: "context-1", primary_worker_id: "worker-1",
  attachments: [], roles: [], messages: [],
  turns: [{
    turn_id: "turn-1", state: "working", created_at_ms: 900,
    user_entries: [{ kind: "task", text: "Insights acceptance turn", created_at_ms: 900 }],
    events: modelEvents(), final_answer: null, completion: null,
  }],
  history_before_cursor: null, history_has_more: false,
  active_turn_id: "turn-1", cancelling_turn_id: null, pending_turn_id: null,
  message_queue: { revision: 0, items: [], auto_send_enabled: true,
    continuation: { state: "awaiting_normal_completion" }, dispatching_command_id: null },
});
const fullResourceCaps = {
  cpu: { enabled: true, percent: 85, kernel: "applied", limit: "2380000 100000", reason: null, escalation_required: false },
  mem: { enabled: true, percent: 85, kernel: "applied", limit: "28310351872", reason: null, escalation_required: false },
  scope: "/sys/fs/cgroup/user.slice/ui.scope",
};
const makeSnapshot = (session, resourceCaps = fullResourceCaps) => ({
  server: {
    version: "ui-acceptance", protocol_version: 1, port: 0,
    bind_host: "127.0.0.1", public_access: false, debug_mode: false,
    performance_trace: false,
    mem: {
      space: "ui-test", data_dir: "/tmp", space_dir: "/tmp/timem-ui-test",
      memory_dir: "/tmp/timem-ui-test/memory", temporary_retention_days: 5,
      temporary_capacity_bytes: null, conversation_capacity_bytes: null,
      mailbox_capacity_bytes: null, mailbox_files: 0, favorite_bytes: 0,
      conversation_bytes: 0, mailbox_bytes: 0, temporary_bytes: 0,
      claude_codex_tool_discovery: false, model_tool_result_bytes: 16384,
      context_compress_threshold_percent: 90, restore_prompt_context_on_startup: false,
      system_cpu_cap_percent: 85, system_mem_cap_percent: 85,
      system_cpu_cap_enabled: true, system_mem_cap_enabled: true,
      resource_caps: resourceCaps,
    },
    runtime_options: [], session_env_defaults: {}, workspace_dirs: ["/work"],
    mcp_servers: [], model_endpoints: [],
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
  let authoritativeCaps = fullResourceCaps;
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
      if (command.command_id) {
        peer.send({ type: "command_ack", command_id: command.command_id, status: "accepted" });
        peer.send({ type: "command_ack", command_id: command.command_id, status: "committed" });
      }
      if (command.type === "session_resource_usage_query") {
        // Two samples 2s apart: the second poll answers with a delta so the
        // CPU curve can derive cores exactly like a live Host would.
        for (const sample of [
          { at: 10000, usec: 1_000_000, mem: 1_200_000_000, busy: 5_000, total: 10_000, memTotal: 34_359_738_368, memAvailable: 21_474_836_480 },
          { at: 12000, usec: 5_600_000, mem: 1_300_000_000, busy: 7_000, total: 14_000, memTotal: 34_359_738_368, memAvailable: 20_401_094_656 },
          { at: 14000, usec: 10_200_000, mem: 1_450_000_000, busy: 9_000, total: 18_000, memTotal: 34_359_738_368, memAvailable: 19_327_352_832 },
          { at: 16000, usec: 14_800_000, mem: 1_600_000_000, busy: 11_000, total: 22_000, memTotal: 34_359_738_368, memAvailable: 17_179_869_184 },
        ]) {
          peer.send({
            type: "session_resource_usage",
            session_id: command.session_id,
            sampled_at_ms: sample.at,
            usage: {
              cpu_usage_usec: sample.usec,
              cpu_quota_cores: 2.8,
              memory_current_bytes: sample.mem,
              memory_peak_bytes: sample.mem,
              swap_current_bytes: 120_000_000,
              memory_high_bytes: 1_800_000_000,
              memory_max_bytes: 2_000_000_000,
              oom_kill_count: 0,
            },
            system: {
              cpu_busy_ticks: sample.busy,
              cpu_total_ticks: sample.total,
              memory_total_bytes: sample.memTotal,
              memory_available_bytes: sample.memAvailable,
            },
          });
        }
      }
    });
    peers.add(peer);
    socket.on("error", () => peers.delete(peer));
    socket.on("close", () => peers.delete(peer));
    peer.send({
      type: "hello", snapshot: makeSnapshot(authoritative),
      event_cursor: 40, event_replay_floor: 0,
    });
  });
  const broadcastHello = () => {
    for (const peer of peers) {
      peer.send({
        type: "hello",
        snapshot: makeSnapshot(authoritative, authoritativeCaps),
        event_cursor: 40, event_replay_floor: 0,
      });
    }
  };
  await new Promise((settle) => server.listen(0, "127.0.0.1", settle));
  return {
    url: `http://127.0.0.1:${server.address().port}/`,
    setSession(next) { authoritative = next; },
    setResourceCaps(next) { authoritativeCaps = next; broadcastHello(); },
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
  const profile = await mkdtemp(join(tmpdir(), "timem-insights-ui-"));
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
async function click(browser, point) {
  await browser.call("Input.dispatchMouseEvent", { type: "mousePressed", x: point.x, y: point.y, button: "left", buttons: 1, clickCount: 1 });
  await browser.call("Input.dispatchMouseEvent", { type: "mouseReleased", x: point.x, y: point.y, button: "left", buttons: 0, clickCount: 1 });
}

async function main() {
  const host = await startHost(makeSession());
  const resourceCaps = (cpuKernel, memKernel) => ({
    cpu: { enabled: cpuKernel === "applied", percent: 85, kernel: cpuKernel, limit: null, reason: null, escalation_required: false },
    mem: { enabled: memKernel === "applied", percent: 85, kernel: memKernel, limit: null, reason: null, escalation_required: false },
    scope: "/sys/fs/cgroup/user.slice/ui.scope",
  });
  const browser = await startBrowser(host.url);
  try {
    await browser.call("Page.bringToFront");
    await waitFor(() => browser.evaluate(`document.body.textContent.includes('Insights acceptance turn')`), "initial snapshot");
    await browser.evaluate(`localStorage.setItem('timem-web-locale-v1','en'); localStorage.removeItem('timem.sessionInsights.order.v1'); localStorage.removeItem('timem.sessionInsights.position.v1'); localStorage.removeItem('timem.sessionInsights.pinned.v1')`);
    await browser.call("Page.reload");
    await waitFor(() => browser.evaluate(`document.body.textContent.includes('Insights acceptance turn')`), "snapshot after preference reset");

    // Collapsed header: only the top item + chevron, no title bar.
    assert(await browser.evaluate(`!!document.querySelector('.session-insights-chip')`), "compact chip exists in the header");
    assert(await browser.evaluate(`!document.querySelector('.session-insights-panel')`), "panel starts collapsed");
    assert(await browser.evaluate(`!!document.querySelector('.session-insights-shield.full')`), "collapsed header shows the full-protection shield");
    assert(await browser.evaluate(`document.querySelector('.session-insights-shield').title.includes('Full protection')`), "shield exposes the current protection status on hover");
    host.setResourceCaps(resourceCaps("applied", "disabled"));
    await waitFor(() => browser.evaluate(`!!document.querySelector('.session-insights-shield.partial')`), "partial-protection shield");
    assert(await browser.evaluate(`document.querySelector('.session-insights-shield').title.includes('Partial protection: CPU capped at 85% only')`), "partial shield hover text");
    host.setResourceCaps(resourceCaps("disabled", "unavailable"));
    await waitFor(() => browser.evaluate(`!!document.querySelector('.session-insights-shield.none')`), "no-protection shield");
    assert(await browser.evaluate(`document.querySelector('.session-insights-shield').title.includes('No resource protection')`), "no-protection shield hover text");
    host.setResourceCaps(fullResourceCaps);
    await waitFor(() => browser.evaluate(`!!document.querySelector('.session-insights-shield.full')`), "full-protection shield restored");
    const shield = await center(browser, ".session-insights-shield");
    await click(browser, shield);
    await waitFor(() => browser.evaluate(`!!document.querySelector('#settings-center .system-resource-caps')`), "shield opens system resource settings");
    assert(await browser.evaluate(`document.querySelector('.settings-center-nav button.active')?.textContent.includes('System')`), "system settings section is active");
    const closeSettings = await center(browser, ".settings-center-header .icon-button");
    await click(browser, closeSettings);
    await waitFor(() => browser.evaluate(`!document.querySelector('#settings-center')`), "settings closed after shield jump");
    assert(await browser.evaluate(`document.querySelector('.session-insights-chip-label').textContent === 'tokens/s'`), "default top item is the token speed label");
    assert(await browser.evaluate(`!document.querySelector('.chat-header').textContent.includes('Session Insights')`), "collapsed header does not render the Session Insights title");

    const chip = await center(browser, ".session-insights-chip");
    await click(browser, chip);
    await waitFor(() => browser.evaluate(`!!document.querySelector('.session-insights-panel')`), "panel opens on chip click");
    assert(await browser.evaluate(`document.querySelectorAll('.session-insight-row').length === 10`), "panel lists all ten items");
    assert(await browser.evaluate(`!document.querySelector('.session-insights-shield')`), "shield leaves the header while the panel is open");
    // Typography: compact and refined; values keep the bold tabular scale,
    // labels stay sentence-case at a small size, especially for CJK text.
    await waitFor(() => browser.evaluate(`document.fonts.check('700 11.5px Rajdhani')
      && document.fonts.check('600 9.5px "Space Grotesk"')
      && document.fonts.check('400 9.5px "IBM Plex Mono"')`), "custom Insight fonts load");
    const typography = await browser.evaluate(`(() => {
      const value = getComputedStyle(document.querySelector('.session-insight-value'));
      const label = getComputedStyle(document.querySelector('.session-insight-label'));
      const panel = getComputedStyle(document.querySelector('.session-insights-panel'));
      const title = getComputedStyle(document.querySelector('.session-insights-title'));
      const detail = getComputedStyle(document.querySelector('.session-insight-detail'));
      const flowNumber = getComputedStyle(document.querySelector('.session-insight-flow-number'));
      return {
        valueSize: value.fontSize,
        valueWeight: value.fontWeight,
        valueFamily: value.fontFamily,
        valueFeatures: value.fontFeatureSettings,
        labelSize: label.fontSize,
        labelWeight: label.fontWeight,
        labelTransform: label.textTransform,
        labelFamily: label.fontFamily,
        panelSize: panel.fontSize,
        panelFamily: panel.fontFamily,
        titleFamily: title.fontFamily,
        titleWeight: title.fontWeight,
        detailFamily: detail.fontFamily,
        flowNumberFamily: flowNumber.fontFamily,
      };
    })()`);
    assert(typography.panelSize === '10.5px', `panel base font: ${JSON.stringify(typography)}`);
    assert(typography.valueSize === '11.5px' && typography.valueWeight === '700', `value typography: ${JSON.stringify(typography)}`);
    assert(typography.valueFamily.includes('Rajdhani'), `value font family: ${JSON.stringify(typography)}`);
    assert(typography.labelSize === '9.5px' && typography.labelWeight === '500' && typography.labelTransform === 'none', `label typography: ${JSON.stringify(typography)}`);
    assert(typography.panelFamily.includes('Source Han Sans SC') && typography.labelFamily.includes('Noto Sans CJK SC'), `Chinese stack: ${JSON.stringify(typography)}`);
    assert(typography.titleFamily.includes('Space Grotesk') && typography.titleWeight === '600', `title font: ${JSON.stringify(typography)}`);
    assert(typography.detailFamily.includes('IBM Plex Mono') && typography.flowNumberFamily.includes('IBM Plex Mono'), `mono values: ${JSON.stringify(typography)}`);
    assert(typography.valueFeatures.includes('tnum'), `tabular numerals enabled: ${JSON.stringify(typography)}`);
    assert(await browser.evaluate(`document.querySelector('.session-insights-panel').getBoundingClientRect().width > 300`), "panel renders as a floating card");
    assert(await browser.evaluate(`[...document.querySelectorAll('.session-insight-flow-number')].map(e => e.textContent).every(t => t && t !== '0')`), "token flow shows numeric values");
    assert(await browser.evaluate(`(() => {
      const row = [...document.querySelectorAll('.session-insight-row')].find(r => r.textContent.includes('LLM'));
      return !!row && !row.textContent.includes('0 0');
    })()`), "LLM/tool counters render real totals");

    // Speed now states average + latest; TTFT states average + spread +
    // latest. The spread is the O(1) Welford population deviation, not an
    // all-history exact percentile.
    assert(await browser.evaluate(`(() => {
      const rows = [...document.querySelectorAll('.session-insight-row')];
      const speed = rows.find(r => r.textContent.includes('tokens/s'));
      const ttft = rows.find(r => r.textContent.includes('TTFT'));
      const cache = rows.find(r => r.textContent.includes('Cache hit'));
      return !!speed && /avg .* now /.test(speed.textContent)
        && !!ttft && /avg .* std dev .* current /.test(ttft.textContent)
        && !!cache && /avg .* now /.test(cache.textContent)
        && cache.textContent.includes('59.5%') && cache.textContent.includes('59.1%');
    })()`), "speed/TTFT/cache show average/latest; cache curve covers the last requests");
    // The mock has 16 requests: TTFT 120..420 ms. Whole-history mean is 0.27s
    // and population sigma is 0.09s; a 15-sample window would show 0.28s.
    assert(await browser.evaluate(`(() => {
      const row = [...document.querySelectorAll('.session-insight-row')].find(r => r.textContent.includes('TTFT'));
      return !!row && row.textContent.includes('avg 0.27s')
        && row.textContent.includes('std dev ±0.09s')
        && row.textContent.includes('current 0.42s')
        && row.title.includes('all historical requests');
    })()`), "TTFT average/stddev cover all history while current is the latest request");
    assert(await browser.evaluate(`(() => {
      const rows = [...document.querySelectorAll('.session-insight-row')];
      const speed = rows.find(r => r.textContent.includes('tokens/s'));
      const ttft = rows.find(r => r.textContent.includes('TTFT'));
      return !!speed && speed.title.includes('Generation speed') && !speed.title.includes('Drag')
        && !!ttft && ttft.title.includes('time to first token') && !ttft.title.includes('Drag');
    })()`), "row hover text describes each metric instead of reordering");
    assert(await browser.evaluate(`document.querySelector('.session-insight-grip').title.includes('Drag a row to reorder')`), "drag hint remains on the grip");
    assert(await browser.evaluate(`document.querySelector('.session-insights-chip-value').textContent.includes('/')`), "collapsed chip shows average/latest for the top item");

    // Sparklines: speed/TTFT have 15 samples, CPU/MEM have a 20s window.
    await waitFor(() => browser.evaluate(`document.querySelectorAll('.session-insight-sparkline').length === 7`), "sparklines render for speed/TTFT/cache/CPU/systemCPU/MEM/systemMEM");
    assert(await browser.evaluate(`(() => {
      const rows = [...document.querySelectorAll('.session-insight-row')];
      const speed = rows.find(r => r.textContent.includes('tokens/s'));
      const ttft = rows.find(r => r.textContent.includes('TTFT'));
      const cache = rows.find(r => r.textContent.includes('Cache hit'));
      return !!speed?.querySelector('polyline') && !!ttft?.querySelector('polyline')
        && !!cache?.querySelector('polyline');
    })()`), "token speed, TTFT, and cache rows carry sparklines");
    await waitFor(() => browser.evaluate(`(() => {
      const rows = [...document.querySelectorAll('.session-insight-row')];
      const cpu = rows.find(r => r.textContent.includes('CPU usage'));
      const mem = rows.find(r => r.textContent.includes('Memory usage'));
      return !!cpu?.querySelector('polyline') && !!mem?.querySelector('polyline');
    })()`), "CPU/MEM rows carry resource sparklines");
    // Each row states its enforced cap in the label and the current usage as
    // the value: CPU quota 2.8 cores / 2.30 used; memory.max 2e9 bytes (1.86 G)
    // / 1.49G used. Limits come from the Host sample, never from a UI guess.
    assert(await browser.evaluate(`(() => {
      const rows = [...document.querySelectorAll('.session-insight-row')];
      const cpu = rows.find(r => r.textContent.includes('CPU usage'));
      const mem = rows.find(r => r.textContent.includes('Memory usage'));
      return !!cpu && /cap 2\\.80 cores/.test(cpu.textContent) && /2\\.30 cores/.test(cpu.textContent)
        && !!mem && /cap 1\\.86 G/.test(mem.textContent) && /1\\.49G/.test(mem.textContent);
    })()`), "CPU/MEM rows show the active cap and current usage");
    assert(await browser.evaluate(`(() => {
      const rows = [...document.querySelectorAll('.session-insight-row')];
      const systemCpu = rows.find(r => r.textContent.includes('System CPU'));
      const systemMem = rows.find(r => r.textContent.includes('System memory'));
      return !!systemCpu && systemCpu.textContent.includes('50%') && !!systemCpu.querySelector('polyline')
        && !!systemMem && systemMem.textContent.includes('50%') && systemMem.textContent.includes('16G/32G')
        && !!systemMem.querySelector('polyline');
    })()`), "System CPU/memory rows show whole-machine usage with curves");

    // Drag the panel by its header.
    const before = await browser.evaluate(`(() => { const r = document.querySelector('.session-insights-panel').getBoundingClientRect(); return { x: r.x, y: r.y }; })()`);
    const head = await center(browser, ".session-insights-head");
    await browser.call("Input.dispatchMouseEvent", { type: "mousePressed", x: head.x, y: head.y, button: "left", buttons: 1, clickCount: 1 });
    for (let step = 1; step <= 6; step += 1) {
      await browser.call("Input.dispatchMouseEvent", { type: "mouseMoved", x: head.x + step * 18, y: head.y + step * 12, button: "left", buttons: 1 });
      await sleep(16);
    }
    await browser.call("Input.dispatchMouseEvent", { type: "mouseReleased", x: head.x + 108, y: head.y + 72, button: "left", buttons: 0, clickCount: 1 });
    const after = await browser.evaluate(`(() => { const r = document.querySelector('.session-insights-panel').getBoundingClientRect(); return { x: r.x, y: r.y }; })()`);
    assert(after.x > before.x + 40 && after.y > before.y + 30, `panel drags with the pointer: ${JSON.stringify(before)} -> ${JSON.stringify(after)}`);

    // Reorder rows: keyboard path (Alt+ArrowDown) and the chip follows order.
    await browser.evaluate(`document.querySelectorAll('.session-insight-row')[0].focus()`);
    await browser.call("Input.dispatchKeyEvent", { type: "keyDown", key: "ArrowDown", code: "ArrowDown", modifiers: 1 });
    await browser.call("Input.dispatchKeyEvent", { type: "keyUp", key: "ArrowDown", code: "ArrowDown", modifiers: 1 });
    await waitFor(() => browser.evaluate(`document.querySelector('.session-insights-chip-label').textContent === 'TTFT'`), "reordered top item drives the collapsed chip");
    assert(await browser.evaluate(`document.querySelectorAll('.session-insight-row')[0].textContent.includes('TTFT')`), "first row is now TTFT");
    assert(await browser.evaluate(`JSON.parse(localStorage.getItem('timem.sessionInsights.order.v1'))[0] === 'ttft'`), "order is persisted locally");

    // Pin keeps the panel open on outside clicks; collapse returns it to the chip.
    const pin = await center(browser, `.session-insights-actions button[aria-pressed]`);
    await click(browser, pin);
    await browser.call("Input.dispatchMouseEvent", { type: "mousePressed", x: 20, y: 500, button: "left", buttons: 1, clickCount: 1 });
    await browser.call("Input.dispatchMouseEvent", { type: "mouseReleased", x: 20, y: 500, button: "left", buttons: 0, clickCount: 1 });
    assert(await browser.evaluate(`!!document.querySelector('.session-insights-panel')`), "pinned panel survives an outside click");
    // The first action button is pin; the last one is collapse.
    const actionCount = await browser.evaluate(`document.querySelectorAll('.session-insights-actions button').length`);
    const collapsePoint = await browser.evaluate(`(() => { const b = [...document.querySelectorAll('.session-insights-actions button')][${actionCount - 1}]; const r = b.getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2 }; })()`);
    await click(browser, collapsePoint);
    await waitFor(() => browser.evaluate(`!document.querySelector('.session-insights-panel')`), "collapse hides the floating panel");
    assert(await browser.evaluate(`document.querySelector('.session-insights-chip').getAttribute('aria-expanded') === 'false'`), "chip is collapsed after the action");
    assert(await browser.evaluate(`!document.querySelector('.chat-header').textContent.includes('Session Insights')`), "collapsed header still omits the title bar");

    // Reload: order and panel position persist; panel is collapsed by default.
    await browser.call("Page.reload");
    await waitFor(() => browser.evaluate(`document.body.textContent.includes('Insights acceptance turn')`), "snapshot after reload");
    assert(await browser.evaluate(`document.querySelector('.session-insights-chip-label').textContent === 'TTFT'`), "custom order survives reload");
    assert(await browser.evaluate(`!document.querySelector('.session-insights-panel')`), "panel starts collapsed after reload");
    const chipAfter = await center(browser, ".session-insights-chip");
    await click(browser, chipAfter);
    await waitFor(() => browser.evaluate(`!!document.querySelector('.session-insights-panel')`), "panel reopens after reload");
    const reopened = await browser.evaluate(`(() => { const r = document.querySelector('.session-insights-panel').getBoundingClientRect(); return { x: r.x, y: r.y }; })()`);
    assert(Math.abs(reopened.x - after.x) <= 2 && Math.abs(reopened.y - after.y) <= 2, `panel position persists: ${JSON.stringify(after)} vs ${JSON.stringify(reopened)}`);

    // The shipped Chinese copy must render the requested label shape:
    // CPU 消耗（限额 2.80 核）/ 2.30 核 and 内存消耗（限额 1.86 G）/ 1.49G.
    await browser.evaluate(`localStorage.setItem('timem-web-locale-v1','zh')`);
    await browser.call("Page.reload");
    await waitFor(() => browser.evaluate(`document.body.textContent.includes('Insights acceptance turn')`), "snapshot after Chinese locale");
    const chipZh = await center(browser, ".session-insights-chip");
    await click(browser, chipZh);
    await waitFor(() => browser.evaluate(`!!document.querySelector('.session-insights-panel')`), "panel opens in Chinese");
    assert(await browser.evaluate(`(() => {
      const rows = [...document.querySelectorAll('.session-insight-row')];
      const cpu = rows.find(r => r.textContent.includes('CPU 消耗'));
      const mem = rows.find(r => r.textContent.includes('内存消耗'));
      return !!cpu && cpu.textContent.includes('CPU 消耗（限额 2.80 核）') && cpu.textContent.includes('2.30 核')
        && !!mem && mem.textContent.includes('内存消耗（限额 1.86 G）') && mem.textContent.includes('1.49G');
    })()`), "Chinese CPU/MEM rows state the cap in the label and usage as the value");
    assert(await browser.evaluate(`(() => {
      const rows = [...document.querySelectorAll('.session-insight-row')];
      const systemCpu = rows.find(r => r.textContent.includes('系统 CPU'));
      const systemMem = rows.find(r => r.textContent.includes('系统内存'));
      return !!systemCpu && systemCpu.textContent.includes('50%') && !!systemCpu.querySelector('polyline')
        && !!systemMem && systemMem.textContent.includes('50%') && systemMem.textContent.includes('16G/32G')
        && systemCpu.title.includes('整机') && systemMem.title.includes('整机');
    })()`), "Chinese system rows show whole-machine usage with curves");
    assert(await browser.evaluate(`(() => {
      const rows = [...document.querySelectorAll('.session-insight-row')];
      const speed = rows.find(r => r.textContent.includes('tokens/s'));
      const ttft = rows.find(r => r.textContent.includes('TTFT'));
      const cache = rows.find(r => r.textContent.includes('Cache hit'));
      return !!speed && speed.textContent.includes('平均') && speed.textContent.includes('本次')
        && !!ttft && ttft.textContent.includes('平均 0.27s') && ttft.textContent.includes('标准差 ±0.09s') && ttft.textContent.includes('当前 0.42s')
        && !!cache && cache.textContent.includes('平均 59.5%') && cache.textContent.includes('本次 59.1%')
        && speed.title.includes('生成速度') && speed.title.includes('全部历史')
        && ttft.title.includes('首 token 延迟') && ttft.title.includes('全部历史请求')
        && cache.title.includes('缓存命中率') && cache.title.includes('最近 15 次');
    })()`), "Chinese rows show whole-history average/stddev and hover copy states it");

    console.log("PASS Session Insights: compact chip, draggable panel, persisted order/position, pin and collapse all verified in a real browser.");
  } finally {
    await browser.close();
    await host.close();
  }
}

main().catch((error) => { console.error("FAIL Session Insights:", error); process.exitCode = 1; });
