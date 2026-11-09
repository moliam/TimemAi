// Real-browser acceptance for the System resource-caps save action. Source
// assertions cannot catch a button falling back to native browser chrome, so
// this renders the real Settings pane against a mock Host and checks the
// computed surface, alignment, disabled/enabled states, light theme, and
// parity with another Settings save button.
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
  session_id: "session-settings-ui", display_name: "Resource caps acceptance", ordinal: 0,
  state: "working", current_dir: "/work", max_llm_input_tokens: 16000,
  tools: [], mcp_server_ids: [],
  contexts: [{ context_id: "context-1", current_dir: "/work", worker_ids: ["worker-1"] }],
  workers: [worker()], active_context_id: "context-1", primary_worker_id: "worker-1",
  attachments: [], roles: [], messages: [],
  turns: [{
    turn_id: "turn-1", state: "working", created_at_ms: 900,
    user_entries: [{ kind: "task", text: "Resource caps acceptance turn", created_at_ms: 900 }],
    events: modelEvents(), final_answer: null, completion: null,
  }],
  history_before_cursor: null, history_has_more: false,
  active_turn_id: "turn-1", cancelling_turn_id: null, pending_turn_id: null,
  message_queue: { revision: 0, items: [], auto_send_enabled: true,
    continuation: { state: "awaiting_normal_completion" }, dispatching_command_id: null },
});
const DEFAULT_CAPS = {
  cpu: { enabled: true, percent: 85, kernel: "applied", limit: "2380000 100000", reason: null, escalation_required: false },
  mem: { enabled: true, percent: 85, kernel: "applied", limit: "28310351872", reason: null, escalation_required: false },
  scope: "/sys/fs/cgroup/user.slice/ui.scope",
};
const makeSnapshot = (session, resourceCaps = DEFAULT_CAPS) => ({
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

async function startHost(session, resourceCaps = DEFAULT_CAPS) {
  let authoritative = session;
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
    let peer;
    peer = makePeer(socket, (command) => {
      commands.push(command);
      if (command.command_id) {
        peer.send({ type: "command_ack", command_id: command.command_id, status: "accepted" });
        peer.send({ type: "command_ack", command_id: command.command_id, status: "committed" });
      }
      if (command.type === "session_resource_usage_query") {
        // Two samples 2s apart: the second poll answers with a delta so the
        // CPU curve can derive cores exactly like a live Host would.
        for (const sample of [
          { at: 10000, usec: 1_000_000, mem: 1_200_000_000 },
          { at: 12000, usec: 5_600_000, mem: 1_300_000_000 },
          { at: 14000, usec: 10_200_000, mem: 1_450_000_000 },
          { at: 16000, usec: 14_800_000, mem: 1_600_000_000 },
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
          });
        }
      }
    });
    peers.add(peer);
    socket.on("error", () => peers.delete(peer));
    socket.on("close", () => peers.delete(peer));
    peer.send({
      type: "hello", snapshot: makeSnapshot(authoritative, resourceCaps),
      event_cursor: 40, event_replay_floor: 0,
    });
  });
  await new Promise((settle) => server.listen(0, "127.0.0.1", settle));
  return {
    url: `http://127.0.0.1:${server.address().port}/`,
    setSession(next) { authoritative = next; },
    commands,
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
    "--headless=new", "--disable-backgrounding-occluded-windows", "--disable-renderer-backgrounding", "--disable-background-timer-throttling",  "--no-sandbox", "--disable-dev-shm-usage",
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

async function openSystemPane(browser) {
  if (!(await browser.evaluate(`!!document.querySelector('.settings-center')`))) {
    await browser.evaluate(`document.querySelector('[aria-controls="settings-center"]').click()`);
    await waitFor(() => browser.evaluate(`!!document.querySelector('.settings-center')`), "settings center opens");
  }
  await browser.evaluate(`([...document.querySelectorAll('.settings-center-nav button')].find(b => b.textContent.trim() === '系统')).click()`);
  await waitFor(() => browser.evaluate(`!!document.querySelector('.system-resource-caps .resource-caps-actions button')`), "system resource caps pane");
}

const capsButtonStyle = (browser) => browser.evaluate(`(() => {
  const b = document.querySelector('.system-resource-caps .resource-caps-actions button');
  if (!b) return null;
  const s = getComputedStyle(b);
  const r = b.getBoundingClientRect();
  const row = b.closest('.resource-caps-actions').getBoundingClientRect();
  return {
    label: b.textContent.trim(), bg: s.backgroundColor, color: s.color,
    radius: s.borderRadius, border: s.borderTopWidth, fontSize: s.fontSize,
    fontWeight: s.fontWeight, height: r.height, shadow: s.boxShadow,
    opacity: s.opacity, disabled: b.disabled,
    rightGap: Math.round(row.right - r.right),
    centerGap: Math.round(Math.abs((row.top + row.height / 2) - (r.top + r.height / 2))),
  };
})()`);

async function saveActionFlow() {
  const host = await startHost(makeSession());
  const browser = await startBrowser(host.url);
  try {
    await browser.call("Page.bringToFront");
    await waitFor(() => browser.evaluate(`document.body.textContent.includes('Resource caps acceptance turn')`), "initial snapshot");
    // Force the user-facing Chinese copy and the dark theme this pane ships with.
    await browser.evaluate(`localStorage.setItem('timem-web-locale-v1','zh'); localStorage.setItem('timem-web-appearance-v1', JSON.stringify({theme:'dark'}))`);
    await browser.call("Page.reload");
    await waitFor(() => browser.evaluate(`document.body.textContent.includes('Resource caps acceptance turn')`), "snapshot after locale/theme");

    await openSystemPane(browser);
    const clean = await capsButtonStyle(browser);
    assert(clean.label === "保存", `save action uses the short 保存 label, got ${JSON.stringify(clean.label)}`);
    assert(clean.disabled, "clean caps state disables save");
    // The shared Settings primary+compact surface: dark green fill, 8px radius,
    // no native border, compact 34px height, settings font size and shadow.
    assert(clean.bg === "rgb(40, 95, 82)", `dark fill is the settings primary surface, got ${clean.bg}`);
    assert(clean.color === "rgb(241, 250, 247)", `dark label color matches settings primary, got ${clean.color}`);
    assert(clean.radius === "8px", `rounded settings corners, got ${clean.radius}`);
    assert(clean.border === "0px", `no native button border, got ${clean.border}`);
    assert(clean.height >= 33 && clean.height <= 36, `compact 34px control height, got ${clean.height}`);
    assert(clean.fontSize === "10.5px", `settings button font size, got ${clean.fontSize}`);
    assert(clean.fontWeight === "680", `settings button weight, got ${clean.fontWeight}`);
    assert(clean.shadow === "none", "disabled save action drops the shadow like the other Settings actions");

    // Dirty a preset so the action becomes enabled, then re-check the surface.
    await browser.evaluate(`(() => {
      const select = document.querySelector('.system-resource-caps select');
      const setValue = Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, 'value').set;
      setValue.call(select, '70');
      select.dispatchEvent(new Event('change', { bubbles: true }));
    })()`);
    await waitFor(() => browser.evaluate(`!document.querySelector('.system-resource-caps .resource-caps-actions button').disabled`), "dirty caps enable save");
    const enabled = await capsButtonStyle(browser);
    assert(enabled.label === "保存", "enabled save action keeps the short label");
    assert(enabled.opacity === "1", `enabled save action is fully opaque, got ${enabled.opacity}`);
    assert(enabled.bg === clean.bg && enabled.radius === clean.radius && enabled.fontSize === clean.fontSize, "enabled surface matches the disabled surface");
    assert(enabled.shadow !== "none", "enabled save action carries the settings shadow");
    assert(enabled.rightGap <= 1 && enabled.centerGap <= 1, `save action aligns to the actions row, gaps ${enabled.rightGap}/${enabled.centerGap}`);

    // The row carries the status slot on the left and the action on the right.
    assert(await browser.evaluate(`!!document.querySelector('.resource-caps-actions .resource-caps-action-status')`), "actions row keeps a dedicated status slot");

    // Parity with an existing Settings save action proves visual coordination.
    await browser.evaluate(`([...document.querySelectorAll('.settings-center-nav button')].find(b => b.textContent.trim() === '记忆')).click()`);
    await waitFor(() => browser.evaluate(`!!document.querySelector('.settings-group-actions button.primary.compact')`), "memory save action");
    const reference = await browser.evaluate(`(() => {
      const b = document.querySelector('.settings-group-actions button.primary.compact');
      const s = getComputedStyle(b);
      return { bg: s.backgroundColor, color: s.color, radius: s.borderRadius, border: s.borderTopWidth, fontSize: s.fontSize };
    })()`);
    assert(reference.bg === enabled.bg, `caps fill matches other Settings save actions: ${reference.bg} vs ${enabled.bg}`);
    assert(reference.color === enabled.color, `caps label color matches other Settings save actions: ${reference.color} vs ${enabled.color}`);
    assert(reference.radius === enabled.radius && reference.border === enabled.border && reference.fontSize === enabled.fontSize, "caps geometry matches other Settings save actions");

    // Light theme keeps the same coordinated surface language.
    await browser.evaluate(`([...document.querySelectorAll('.settings-center-nav button')].find(b => b.textContent.trim() === '外观')).click()`);
    await waitFor(() => browser.evaluate(`!!document.querySelector('.segmented-control')`), "appearance pane");
    await browser.evaluate(`([...document.querySelectorAll('.segmented-control button')].find(b => b.textContent.trim() === '浅色')).click()`);
    await waitFor(() => browser.evaluate(`document.documentElement.dataset.theme === 'light'`), "light theme applied");
    await browser.evaluate(`([...document.querySelectorAll('.settings-center-nav button')].find(b => b.textContent.trim() === '系统')).click()`);
    await waitFor(() => browser.evaluate(`!!document.querySelector('.system-resource-caps .resource-caps-actions button')`), "system pane after theme switch");
    const light = await capsButtonStyle(browser);
    assert(light.label === "保存", "light theme keeps the short label");
    assert(light.bg === "rgb(47, 118, 101)", `light fill is the settings primary surface, got ${light.bg}`);
    assert(light.color === "rgb(255, 255, 255)", `light label color matches settings primary, got ${light.color}`);
    assert(light.radius === "8px" && light.border === "0px", "light theme keeps the settings button geometry");

    console.log("PASS Resource caps save action: 保存 label + settings-native button surface verified in a real browser (dark/light, disabled/enabled).");
  } finally {
    await browser.close();
    await host.close();
  }
}

// A dimension without any usable kernel facility must render as an OFF,
// greyed, inoperable switch with only a hoverable warning icon — regardless of
// the stored enabled flag — and never surface technical status copy.
async function unsupportedFlow() {
  const caps = {
    cpu: DEFAULT_CAPS.cpu,
    mem: { enabled: true, percent: 85, kernel: "unsupported", limit: null, reason: "cgroup v2 controllers unavailable", escalation_required: false },
    scope: DEFAULT_CAPS.scope,
  };
  const host = await startHost(makeSession(), caps);
  const browser = await startBrowser(host.url);
  try {
    await browser.call("Page.bringToFront");
    await waitFor(() => browser.evaluate(`document.body.textContent.includes('Resource caps acceptance turn')`), "initial snapshot");
    await browser.evaluate(`localStorage.setItem('timem-web-locale-v1','zh'); localStorage.setItem('timem-web-appearance-v1', JSON.stringify({theme:'dark'}))`);
    await browser.call("Page.reload");
    await waitFor(() => browser.evaluate(`document.body.textContent.includes('Resource caps acceptance turn')`), "snapshot after locale/theme");

    await openSystemPane(browser);
    const mem = await browser.evaluate(`(() => {
      const row = document.querySelector('.resource-cap-row[data-dimension="mem"]');
      if (!row) return null;
      const sw = row.querySelector('input[type="checkbox"]');
      const warn = row.querySelector('.resource-cap-warning');
      return {
        capability: row.dataset.capability,
        checked: sw.checked,
        disabled: sw.disabled,
        selectDisabled: row.querySelector('select').disabled,
        nameColor: getComputedStyle(row.querySelector('.resource-cap-name')).color,
        nameOpacity: getComputedStyle(row.querySelector('.resource-cap-name')).opacity,
        warnTitle: warn ? warn.title : null,
        warnAria: warn ? warn.getAttribute('aria-label') : null,
        warnTooltip: warn ? warn.getAttribute('data-tooltip') : null,
        warnTabIndex: warn ? warn.tabIndex : -1,
        warnColor: warn ? getComputedStyle(warn).color : null,
        badge: !!row.querySelector('.resource-caps-status'),
        rowText: row.textContent,
      };
    })()`);
    assert(mem, "mem row renders");
    assert(mem.capability === "unsupported", `mem row marked unsupported, got ${mem.capability}`);
    assert(!mem.checked, `stored enabled:true must render OFF when unsupported, got checked=${mem.checked}`);
    assert(mem.disabled, "unsupported switch is disabled");
    assert(mem.selectDisabled, "unsupported preset select is disabled");
    assert(mem.warnTitle === "当前系统不支持内存管控", `warning hover title, got ${JSON.stringify(mem.warnTitle)}`);
    assert(mem.warnAria === mem.warnTitle, "warning icon is announced to assistive tech");
    assert(mem.warnTooltip === null, "custom data-tooltip is gone; the native title is the only tooltip");
    assert(mem.warnTabIndex === 0, "warning icon is keyboard-focusable so the hint is not mouse-only");
    assert(!mem.badge, "no kernel status badge");
    assert(!mem.rowText.includes("不可用") && !mem.rowText.includes("平台不支持") && !mem.rowText.includes("需要 root"), `no technical copy in the row: ${JSON.stringify(mem.rowText)}`);
    assert(mem.nameColor === "rgb(119, 132, 126)" && mem.nameOpacity === "0.55", `dark theme greys the name, got ${mem.nameColor}/${mem.nameOpacity}`);
    assert(mem.warnColor === "rgb(176, 138, 90)", `dark theme warning tint, got ${mem.warnColor}`);

    const cpu = await browser.evaluate(`(() => {
      const row = document.querySelector('.resource-cap-row[data-dimension="cpu"]');
      const sw = row.querySelector('input[type="checkbox"]');
      return { checked: sw.checked, disabled: sw.disabled, capability: row.dataset.capability, warn: !!row.querySelector('.resource-cap-warning') };
    })()`);
    assert(cpu.checked && !cpu.disabled && cpu.capability === "manageable" && !cpu.warn, "cpu row stays operable without a warning icon");

    // Light theme keeps the greyed language readable.
    await browser.evaluate(`localStorage.setItem('timem-web-appearance-v1', JSON.stringify({theme:'light'})); 0`);
    await browser.call("Page.reload");
    await waitFor(() => browser.evaluate(`document.body.textContent.includes('Resource caps acceptance turn')`), "snapshot after light theme");
    await openSystemPane(browser);
    const light = await browser.evaluate(`(() => {
      const row = document.querySelector('.resource-cap-row[data-dimension="mem"]');
      return {
        checked: row.querySelector('input[type="checkbox"]').checked,
        nameColor: getComputedStyle(row.querySelector('.resource-cap-name')).color,
        warnColor: getComputedStyle(row.querySelector('.resource-cap-warning')).color,
      };
    })()`);
    assert(!light.checked, "light theme keeps the unsupported switch OFF");
    assert(light.nameColor === "rgb(101, 118, 111)", `light theme greys the name, got ${light.nameColor}`);
    assert(light.warnColor === "rgb(138, 109, 47)", `light theme warning tint, got ${light.warnColor}`);

    // Saving still works for the manageable dimension and submits the
    // unsupported one as OFF, never the stale stored ON flag.
    await browser.evaluate(`(() => {
      const select = document.querySelector('.resource-cap-row[data-dimension="cpu"] select');
      const setValue = Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, 'value').set;
      setValue.call(select, '70');
      select.dispatchEvent(new Event('change', { bubbles: true }));
    })()`);
    await waitFor(() => browser.evaluate(`!document.querySelector('.system-resource-caps .resource-caps-actions button').disabled`), "dirty caps enable save");
    await browser.evaluate(`document.querySelector('.system-resource-caps .resource-caps-actions button').click()`);
    await waitFor(() => host.commands.some((c) => c.type === "system_resource_caps_update"), "caps save command");
    const saved = host.commands.find((c) => c.type === "system_resource_caps_update");
    assert(saved.mem_enabled === false, `unsupported dimension must submit OFF, got ${saved.mem_enabled}`);
    assert(saved.cpu_enabled === true && saved.cpu_percent === 70, "manageable dimension keeps its draft");

    console.log("PASS Unsupported dimension: stored ON renders OFF, greyed with hoverable warning, inoperable, no technical copy (dark/light).");
  } finally {
    await browser.close();
    await host.close();
  }
}

// A facility that exists but cannot be applied right now (for example a
// cgroup root shared with other processes) must not be misreported as a
// permanent platform limitation: same inert controls, different explanation.
async function unavailableFlow() {
  const caps = {
    cpu: DEFAULT_CAPS.cpu,
    mem: { enabled: true, percent: 85, kernel: "unavailable", limit: null, reason: "instance root shared with other processes", escalation_required: false },
    scope: DEFAULT_CAPS.scope,
  };
  const host = await startHost(makeSession(), caps);
  const browser = await startBrowser(host.url);
  try {
    await browser.call("Page.bringToFront");
    await waitFor(() => browser.evaluate(`document.body.textContent.includes('Resource caps acceptance turn')`), "initial snapshot");
    await browser.evaluate(`localStorage.setItem('timem-web-locale-v1','zh'); localStorage.setItem('timem-web-appearance-v1', JSON.stringify({theme:'dark'}))`);
    await browser.call("Page.reload");
    await waitFor(() => browser.evaluate(`document.body.textContent.includes('Resource caps acceptance turn')`), "snapshot after locale/theme");

    await openSystemPane(browser);
    const mem = await browser.evaluate(`(() => {
      const row = document.querySelector('.resource-cap-row[data-dimension="mem"]');
      const sw = row.querySelector('input[type="checkbox"]');
      const warn = row.querySelector('.resource-cap-warning');
      return {
        capability: row.dataset.capability,
        checked: sw.checked,
        disabled: sw.disabled,
        selectDisabled: row.querySelector('select').disabled,
        inertClass: row.classList.contains('is-inert'),
        warnTitle: warn ? warn.title : null,
        warnAria: warn ? warn.getAttribute('aria-label') : null,
        warnTooltip: warn ? warn.getAttribute('data-tooltip') : null,
        rowText: row.textContent,
      };
    })()`);
    assert(mem, "mem row renders");
    assert(mem.capability === "unavailable", `mem row marked temporarily unavailable, got ${mem.capability}`);
    assert(!mem.checked && mem.disabled && mem.selectDisabled, "temporarily unavailable dimension is OFF and inoperable");
    assert(mem.inertClass, "inert styling applies to a temporarily unavailable dimension");
    assert(mem.warnTitle === "当前系统暂时无法使用内存管控", `temporarily-unavailable title, got ${JSON.stringify(mem.warnTitle)}`);
    assert(mem.warnAria === mem.warnTitle, "warning icon is announced to assistive tech");
    assert(mem.warnTooltip === null, "custom data-tooltip is gone; the native title is the only tooltip");
    assert(!mem.rowText.includes("不支持"), `temporarily unavailable is not described as unsupported: ${JSON.stringify(mem.rowText)}`);

    // Saving submits the inert dimension as OFF, never the stale stored ON.
    await browser.evaluate(`(() => {
      const select = document.querySelector('.resource-cap-row[data-dimension="cpu"] select');
      const setValue = Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, 'value').set;
      setValue.call(select, '70');
      select.dispatchEvent(new Event('change', { bubbles: true }));
    })()`);
    await waitFor(() => browser.evaluate(`!document.querySelector('.system-resource-caps .resource-caps-actions button').disabled`), "dirty caps enable save");
    await browser.evaluate(`document.querySelector('.system-resource-caps .resource-caps-actions button').click()`);
    await waitFor(() => host.commands.some((c) => c.type === "system_resource_caps_update"), "caps save command");
    const saved = host.commands.find((c) => c.type === "system_resource_caps_update");
    assert(saved.mem_enabled === false, `temporarily unavailable dimension must submit OFF, got ${saved.mem_enabled}`);
    assert(saved.cpu_enabled === true && saved.cpu_percent === 70, "manageable dimension keeps its draft");

    console.log("PASS Temporarily unavailable dimension: distinct hint, inert controls, native title only, submits OFF.");
  } finally {
    await browser.close();
    await host.close();
  }
}

// A dimension with a facility but no permission keeps an interactive switch
// that renders OFF, and toggling it on opens the root password dialog at once.
async function escalationFlow() {
  const caps = {
    cpu: DEFAULT_CAPS.cpu,
    mem: { enabled: true, percent: 85, kernel: "unavailable", limit: null, reason: "memory controller not enabled", escalation_required: true },
    scope: DEFAULT_CAPS.scope,
  };
  const host = await startHost(makeSession(), caps);
  const browser = await startBrowser(host.url);
  try {
    await browser.call("Page.bringToFront");
    await waitFor(() => browser.evaluate(`document.body.textContent.includes('Resource caps acceptance turn')`), "initial snapshot");
    await browser.evaluate(`localStorage.setItem('timem-web-locale-v1','zh'); localStorage.setItem('timem-web-appearance-v1', JSON.stringify({theme:'dark'}))`);
    await browser.call("Page.reload");
    await waitFor(() => browser.evaluate(`document.body.textContent.includes('Resource caps acceptance turn')`), "snapshot after locale/theme");

    await openSystemPane(browser);
    const before = await browser.evaluate(`(() => {
      const row = document.querySelector('.resource-cap-row[data-dimension="mem"]');
      const sw = row.querySelector('input[type="checkbox"]');
      return {
        checked: sw.checked,
        disabled: sw.disabled,
        capability: row.dataset.capability,
        badge: !!row.querySelector('.resource-caps-status'),
        warn: !!row.querySelector('.resource-cap-warning'),
        rowText: row.textContent,
      };
    })()`);
    assert(before.checked === false, `stored enabled:true must render OFF while unavailable, got checked=${before.checked}`);
    assert(before.disabled === false, "permission-gated switch stays interactive");
    assert(before.capability === "needs-permission", `capability=${before.capability}`);
    assert(!before.badge && !before.warn, "no badge and no warning icon on the permission-gated row");
    assert(!before.rowText.includes("需要 root") && !before.rowText.includes("不可用"), "no technical copy before toggling");

    // Toggling ON opens the password dialog immediately, without a save step.
    const switchPoint = await center(browser, '.resource-cap-row[data-dimension="mem"] .resource-cap-switch');
    await click(browser, switchPoint);
    await waitFor(() => browser.evaluate(`!!document.querySelector('.resource-caps-modal')`), "password dialog opens on toggle");
    assert(await browser.evaluate(`document.querySelector('.resource-cap-row[data-dimension="mem"] input[type="checkbox"]').checked`) === false, "draft stays OFF while the dialog is open");
    assert(await browser.evaluate(`document.activeElement === document.querySelector('.resource-caps-modal input[type="password"]')`), "password field receives focus when the dialog opens");

    // Tab must stay inside the modal instead of escaping to the settings page.
    for (let index = 0; index < 4; index += 1) {
      await browser.call("Input.dispatchKeyEvent", { type: "keyDown", key: "Tab", code: "Tab", windowsVirtualKeyCode: 9 });
      await browser.call("Input.dispatchKeyEvent", { type: "keyUp", key: "Tab", code: "Tab", windowsVirtualKeyCode: 9 });
    }
    assert(await browser.evaluate(`document.querySelector('.resource-caps-modal').contains(document.activeElement)`), "Tab keeps focus inside the password dialog");

    // Escape works from the focused action button too, not only the input.
    await browser.evaluate(`document.querySelector('.resource-caps-modal-actions button').focus()`);
    await browser.call("Input.dispatchKeyEvent", { type: "keyDown", key: "Escape", code: "Escape", windowsVirtualKeyCode: 27 });
    await browser.call("Input.dispatchKeyEvent", { type: "keyUp", key: "Escape", code: "Escape", windowsVirtualKeyCode: 27 });
    await waitFor(() => browser.evaluate(`!document.querySelector('.resource-caps-modal')`), "Escape closes the dialog from a button");
    const afterEscape = await browser.evaluate(`({
      modal: !!document.querySelector('.resource-caps-modal'),
      settings: !!document.querySelector('.settings-center'),
      checked: document.querySelector('.resource-cap-row[data-dimension="mem"] input[type="checkbox"]')?.checked,
    })`);
    assert(!afterEscape.modal, "Escape closes only the password dialog");
    assert(afterEscape.settings, `Escape keeps the Settings center open: ${JSON.stringify(afterEscape)}`);
    assert(afterEscape.checked === false, `Escape leaves the switch OFF: ${JSON.stringify(afterEscape)}`);

    // Cancelling keeps the dimension OFF. Escape above closed the dialog, so
    // reopen it first.
    const cancelPoint = await center(browser, '.resource-cap-row[data-dimension="mem"] .resource-cap-switch');
    await click(browser, cancelPoint);
    await waitFor(() => browser.evaluate(`!!document.querySelector('.resource-caps-modal')`), "dialog reopens for cancel test");
    await browser.evaluate(`[...document.querySelectorAll('.resource-caps-modal-actions button')].find(b => b.textContent.trim() === '取消').click()`);
    await waitFor(() => browser.evaluate(`!document.querySelector('.resource-caps-modal')`), "dialog closes on cancel");
    assert(await browser.evaluate(`document.querySelector('.resource-cap-row[data-dimension="mem"] input[type="checkbox"]').checked`) === false, "cancel leaves the switch OFF");

    // Clicking the dimmed backdrop is a familiar non-technical exit too.
    const reopenPoint = await center(browser, '.resource-cap-row[data-dimension="mem"] .resource-cap-switch');
    await click(browser, reopenPoint);
    await waitFor(() => browser.evaluate(`!!document.querySelector('.resource-caps-modal')`), "dialog reopens for backdrop test");
    await click(browser, { x: 10, y: 10 });
    await waitFor(() => browser.evaluate(`!document.querySelector('.resource-caps-modal')`), "backdrop click cancels the dialog");
    assert(await browser.evaluate(`document.querySelector('.resource-cap-row[data-dimension="mem"] input[type="checkbox"]').checked`) === false, "backdrop cancel leaves the switch OFF");

    // Toggling again and confirming escalates only the target dimension.
    const point = await center(browser, '.resource-cap-row[data-dimension="mem"] .resource-cap-switch');
    await click(browser, point);
    await waitFor(() => browser.evaluate(`!!document.querySelector('.resource-caps-modal input[type="password"]')`), "dialog reopens");
    await browser.evaluate(`(() => {
      const input = document.querySelector('.resource-caps-modal input[type="password"]');
      const setValue = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set;
      setValue.call(input, 'test-only-password');
      input.dispatchEvent(new Event('input', { bubbles: true }));
    })()`);
    await browser.evaluate(`[...document.querySelectorAll('.resource-caps-modal-actions button')].find(b => b.textContent.trim() === '授权并开启').click()`);
    await waitFor(() => host.commands.some((c) => c.type === "system_resource_caps_escalate"), "escalation command");
    const escalated = host.commands.find((c) => c.type === "system_resource_caps_escalate");
    assert(escalated.mem_enabled === true, `target dimension is forced ON, got ${escalated.mem_enabled}`);
    assert(escalated.cpu_enabled === true, `the other dimension keeps its draft, got ${escalated.cpu_enabled}`);
    assert(escalated.cpu_percent === 85 && escalated.mem_percent === 85, "draft percents are preserved");
    assert(escalated.password === "test-only-password", "the one-shot password rides the escalate command");
    await waitFor(() => browser.evaluate(`!document.querySelector('.resource-caps-modal')`), "dialog closes after confirm");

    console.log("PASS Permission-gated dimension: stored ON renders OFF, toggle opens the password dialog, cancel keeps OFF, confirm escalates the target only.");
  } finally {
    await browser.close();
    await host.close();
  }
}

async function main() {
  await saveActionFlow();
  await unsupportedFlow();
  await unavailableFlow();
  await escalationFlow();
}

main().catch((error) => { console.error("FAIL Resource caps save action:", error); process.exitCode = 1; });
