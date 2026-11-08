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
const headerEndpoint = {
  id: "ep-header", name: "Header Model", model: "header-model-1",
  api_protocol: "openai_chat", response_protocol: "openai_chat",
  base_url: "http://127.0.0.1:1/v1",
  max_llm_input_tokens: 16000, max_llm_output_tokens: 4096,
  stream: true, function_calling: true, api_key_configured: true,
  http_headers: {}, request_fields: {},
  allow_cross_origin_redirects: false, private_ca_configured: false,
  requirements: {
    version: 1, provider: null, adaptive_reasoning: null,
    allowed_reasoning: ["low", "medium", "high"], field_sources: {},
  },
};
const makeSession = () => ({
  session_id: "session-insights", display_name: "Insights acceptance", ordinal: 0,
  state: "working", current_dir: "/work", max_llm_input_tokens: 16000,
  runtime_profile: {
    model_endpoint_id: "ep-header", active_model_endpoint_id: "ep-header",
    model_endpoint_ids: ["ep-header"], reasoning_effort: "high",
    model: "header-model-1", api_protocol: "openai_chat",
    response_protocol: "openai_chat", base_url: "http://127.0.0.1:1/v1",
    timeout_secs: 600, max_llm_input_tokens: 16000, max_llm_output_tokens: 4096,
    stream: true, max_rounds: "64", bash_approval: "ask",
    work_instructions: "", api_key_configured: true,
  },
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
    mcp_servers: [], model_endpoints: [headerEndpoint],
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

// Mirrors the Host's authoritative projection for the model-config commands:
// the header can only show what the Host accepted, and the active route is
// updated before the next model request (the mock updates state synchronously).
function withEndpointMaxInput(session, endpointId, tokens) {
  const profile = session.runtime_profile ?? {};
  const overrides = { ...(profile.endpoint_max_input_tokens ?? {}) };
  if (tokens === null || tokens === undefined) delete overrides[endpointId];
  else overrides[endpointId] = tokens;
  const effective = overrides[endpointId] ?? headerEndpoint.max_llm_input_tokens;
  return {
    ...session,
    max_llm_input_tokens: effective,
    runtime_profile: { ...profile, endpoint_max_input_tokens: overrides, max_llm_input_tokens: effective },
  };
}
function withReasoningEffort(session, effort) {
  const profile = session.runtime_profile ?? {};
  return {
    ...session,
    runtime_profile: { ...profile, reasoning_effort: effort, endpoint_reasoning_efforts: { ...(profile.endpoint_reasoning_efforts ?? {}), [profile.active_model_endpoint_id ?? profile.model_endpoint_id]: effort } },
  };
}
function withEndpointReasoningEffort(session, endpointId, effort) {
  const profile = session.runtime_profile ?? {};
  return {
    ...session,
    runtime_profile: { ...profile, endpoint_reasoning_efforts: { ...(profile.endpoint_reasoning_efforts ?? {}), [endpointId]: effort } },
  };
}
function withSessionMaxInput(session, tokens) {
  const profile = session.runtime_profile ?? {};
  return {
    ...session,
    max_llm_input_tokens: tokens,
    runtime_profile: { ...profile, max_llm_input_tokens: tokens },
  };
}
function withSessionReasoning(session, effort) {
  const profile = session.runtime_profile ?? {};
  return { ...session, runtime_profile: { ...profile, reasoning_effort: effort } };
}
function withChain(session, ids) {
  const profile = session.runtime_profile ?? {};
  const active = profile.active_model_endpoint_id;
  return {
    ...session,
    runtime_profile: {
      ...profile,
      model_endpoint_ids: ids,
      model_endpoint_id: ids[0] ?? null,
      active_model_endpoint_id: ids.includes(active) ? active : (ids[0] ?? null),
    },
  };
}

async function startHost(session) {
  let authoritative = session;
  let authoritativeCaps = fullResourceCaps;
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
      if (command.type === "session_endpoint_max_input_tokens_update") {
        authoritative = withEndpointMaxInput(authoritative, command.endpoint_id, command.max_input_tokens);
        broadcastHello();
      } else if (command.type === "session_reasoning_effort_update") {
        authoritative = withReasoningEffort(authoritative, command.reasoning_effort);
        broadcastHello();
      } else if (command.type === "session_endpoint_reasoning_effort_update") {
        authoritative = withEndpointReasoningEffort(authoritative, command.endpoint_id, command.reasoning_effort);
        broadcastHello();
      } else if (command.type === "session_runtime_update" && command.key === "TIMEM_MAX_LLM_INPUT") {
        authoritative = withSessionMaxInput(authoritative, Number(command.value));
        broadcastHello();
      } else if (command.type === "session_runtime_update" && command.key === "TIMEM_REASONING_EFFORT") {
        authoritative = withSessionReasoning(authoritative, command.value);
        broadcastHello();
      } else if (command.type === "model_endpoint_chain_apply") {
        const ids = Array.isArray(command.endpoint_ids) ? [...command.endpoint_ids] : [];
        // Keep the pending draft visible long enough to prove the UI does not
        // treat a submitted command as an applied Host state.
        setTimeout(() => {
          authoritative = withChain(authoritative, ids);
          broadcastHello();
        }, 350);
      } else if (command.type === "session_resource_usage_query") {
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
    commands,
    setSession(next) { authoritative = next; broadcastHello(); },
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
    // Headless otherwise reports (hover: none), which would skip the real
    // desktop hover-reveal behavior under test; force desktop hover/pointer
    // capabilities to match the production environment.
    "--blink-settings=primaryHoverType=2,availableHoverType=2,primaryPointerType=4,availablePointerType=4",
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

    // Responsive sidebar regression: at desktop widths the collapsed role
    // panel is the 44px right rail; below 1051px it must become a fixed
    // off-canvas drawer instead of wrapping into a grid row under the left
    // sidebar. At <=720px the left sidebar must also stay off-canvas.
    for (const width of [1440, 1200, 1060, 1051]) {
      await browser.call("Emulation.setDeviceMetricsOverride", { width, height: 900, deviceScaleFactor: 1, mobile: false });
      await browser.evaluate(`new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)))`);
      await sleep(260);
      const layout = await browser.evaluate(`(() => {
        const panel = document.querySelector('.worker-role-panel');
        const restore = document.querySelector('.worker-role-restore');
        const p = panel.getBoundingClientRect();
        return { position: getComputedStyle(panel).position, left: p.left, top: p.top, width: p.width, restoreDisplay: getComputedStyle(restore).display, viewport: innerWidth };
      })()`);
      assert(layout.position === 'relative', `desktop role panel stays in the grid at ${width}: ${JSON.stringify(layout)}`);
      assert(Math.abs(layout.left - (layout.viewport - 44)) <= 1, `desktop role panel occupies the right rail at ${width}: ${JSON.stringify(layout)}`);
      assert(layout.top === 0, `desktop role panel starts at the top at ${width}: ${JSON.stringify(layout)}`);
      assert(layout.restoreDisplay === 'grid', `desktop restore control is visible at ${width}: ${JSON.stringify(layout)}`);
    }
    for (const width of [1050, 900, 721]) {
      await browser.call("Emulation.setDeviceMetricsOverride", { width, height: 900, deviceScaleFactor: 1, mobile: false });
      await browser.evaluate(`new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)))`);
      await sleep(260);
      const layout = await browser.evaluate(`(() => {
        const panel = document.querySelector('.worker-role-panel');
        const restore = document.querySelector('.worker-role-restore');
        const p = panel.getBoundingClientRect();
        return { position: getComputedStyle(panel).position, left: p.left, top: p.top, visibility: getComputedStyle(panel).visibility, restoreDisplay: getComputedStyle(restore).display, viewport: innerWidth };
      })()`);
      assert(layout.position === 'fixed', `mid-width role panel must be a fixed drawer at ${width}: ${JSON.stringify(layout)}`);
      assert(layout.top === 0, `mid-width role panel must be anchored to the top at ${width}: ${JSON.stringify(layout)}`);
      assert(layout.left >= layout.viewport - 1, `mid-width role panel must stay off-canvas on the right at ${width}: ${JSON.stringify(layout)}`);
      assert(layout.visibility === 'hidden', `mid-width role panel must be hidden at ${width}: ${JSON.stringify(layout)}`);
      assert(layout.restoreDisplay === 'none', `mid-width role restore control must be hidden at ${width}: ${JSON.stringify(layout)}`);
    }
    for (const width of [720, 390]) {
      await browser.call("Emulation.setDeviceMetricsOverride", { width, height: 900, deviceScaleFactor: 1, mobile: false });
      await browser.evaluate(`new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)))`);
      await sleep(260);
      const layout = await browser.evaluate(`(() => {
        const panel = document.querySelector('.worker-role-panel');
        const sidebar = document.querySelector('.sidebar');
        const p = panel.getBoundingClientRect();
        const s = sidebar.getBoundingClientRect();
        return { panelPosition: getComputedStyle(panel).position, panelLeft: p.left, panelTop: p.top, sidebarPosition: getComputedStyle(sidebar).position, sidebarLeft: s.left, sidebarRight: s.right, sidebarTop: s.top, viewport: innerWidth };
      })()`);
      assert(layout.panelPosition === 'fixed', `mobile role panel must be a fixed drawer at ${width}: ${JSON.stringify(layout)}`);
      assert(layout.panelLeft >= layout.viewport - 1, `mobile role panel must stay off-canvas on the right at ${width}: ${JSON.stringify(layout)}`);
      assert(layout.sidebarPosition === 'fixed', `mobile left sidebar must be fixed at ${width}: ${JSON.stringify(layout)}`);
      assert(layout.sidebarRight <= 0, `mobile left sidebar must stay off-canvas on the left at ${width}: ${JSON.stringify(layout)}`);
      assert(layout.sidebarTop === 0, `mobile left sidebar must not take a grid row at ${width}: ${JSON.stringify(layout)}`);
    }
    await browser.call("Emulation.setDeviceMetricsOverride", { width: 1440, height: 900, deviceScaleFactor: 1, mobile: false });
    await browser.evaluate(`localStorage.setItem('timem-web-locale-v1','en'); localStorage.removeItem('timem.sessionInsights.order.v1'); localStorage.removeItem('timem.sessionInsights.position.v1'); localStorage.removeItem('timem.sessionInsights.pinned.v1')`);
    await browser.call("Page.reload");
    await waitFor(() => browser.evaluate(`document.body.textContent.includes('Insights acceptance turn')`), "snapshot after preference reset");

    // Header elements must share one vertical center line at every responsive
    // width and in both themes: the endpoint/model pill, the insights chip,
    // the context-actions divider band, and the right-side icon actions.
    const headerCenterSnapshot = () => browser.evaluate(`(() => {
      const rect = (element) => {
        if (!element) return null;
        const r = element.getBoundingClientRect();
        return { center: r.top + r.height / 2, height: r.height };
      };
      const visibleActionButton = Array.from(document.querySelectorAll('.header-actions .icon-button'))
        .find((element) => element.getBoundingClientRect().height > 0);
      const contextActions = document.querySelector('.header-context-actions');
      const contextVisible = !!contextActions && contextActions.getBoundingClientRect().height > 0;
      return {
        header: rect(document.querySelector('.chat-header')),
        cluster: rect(document.querySelector('.header-session-cluster')),
        pill: rect(document.querySelector('.header-model-control')),
        contextActions: contextVisible ? rect(contextActions) : null,
        insightsChip: contextVisible ? rect(document.querySelector('.session-insights-chip')) : null,
        actions: rect(document.querySelector('.header-actions')),
        actionButton: rect(visibleActionButton),
      };
    })()`);
    const headerCentersFor = (snapshot) => ['cluster', 'pill', 'contextActions', 'insightsChip', 'actions', 'actionButton']
      .map((key) => snapshot[key]?.center)
      .filter((center) => typeof center === 'number');
    for (const [width, theme] of [[1440, 'dark'], [1200, 'dark'], [1051, 'dark'], [1050, 'dark'], [900, 'dark'], [720, 'dark'], [390, 'dark'], [1440, 'light']]) {
      await browser.call("Emulation.setDeviceMetricsOverride", { width, height: 900, deviceScaleFactor: 1, mobile: false });
      await browser.evaluate(`document.documentElement.dataset.theme = '${theme}'`);
      await browser.evaluate(`new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)))`);
      await sleep(80);
      const snapshot = await headerCenterSnapshot();
      const centers = headerCentersFor(snapshot);
      assert(centers.length >= 4, `header alignment targets present at ${width}px/${theme}: ${JSON.stringify(snapshot)}`);
      assert(
        Math.max(...centers) - Math.min(...centers) <= 1,
        `header controls share one horizontal center line at ${width}px/${theme}: ${JSON.stringify(snapshot)}`,
      );
      assert(
        Math.abs(snapshot.header.center - centers[0]) <= 1.5,
        `header controls are centered in the header band at ${width}px/${theme}: ${JSON.stringify(snapshot)}`,
      );
    }
    await browser.call("Emulation.setDeviceMetricsOverride", { width: 1440, height: 900, deviceScaleFactor: 1, mobile: false });
    await browser.evaluate(`document.documentElement.dataset.theme = 'dark'`);

    // Header model/context cluster: collapsed it shows the current model name,
    // reasoning strength and the context donut in that fixed order; expanding it
    // is the single place for model switching, reasoning, context length and tools.
    assert(await browser.evaluate(`!!document.querySelector('.header-model-control')`), "collapsed header renders the model control");
    assert(await browser.evaluate(`document.querySelector('.header-model-control-name').textContent === 'header-model-1'`), "collapsed control shows the current model name");
    assert(await browser.evaluate(`(() => {
      const control = document.querySelector('.header-model-control');
      const name = control.querySelector('.header-model-control-name');
      const reasoning = control.querySelector('.header-model-control-reasoning');
      const donut = control.querySelector('.context-donut');
      return !!name && !!reasoning && !!donut
        && name.compareDocumentPosition(reasoning) === Node.DOCUMENT_POSITION_FOLLOWING
        && reasoning.compareDocumentPosition(donut) === Node.DOCUMENT_POSITION_FOLLOWING;
    })()`), "collapsed order is model name, reasoning, donut");
    assert(await browser.evaluate(`!document.querySelector('.header-model-control .lucide-sparkles')`), "collapsed control has no decorative sparkles");
    assert(await browser.evaluate(`document.querySelector('.header-model-control').title.includes('Context usage 7%')`), "collapsed control exposes context usage on hover");
    const headerAlignment = await browser.evaluate(`(() => {
      const rect = (element) => {
        if (!element) return null;
        const r = element.getBoundingClientRect();
        return { top: r.top, bottom: r.bottom, center: r.top + r.height / 2, height: r.height };
      };
      const visibleActionButton = Array.from(document.querySelectorAll('.header-actions .icon-button'))
        .find((element) => element.getBoundingClientRect().height > 0);
      return {
        header: rect(document.querySelector('.chat-header')),
        cluster: rect(document.querySelector('.header-session-cluster')),
        pill: rect(document.querySelector('.header-model-control')),
        contextActions: rect(document.querySelector('.header-context-actions')),
        insightsChip: rect(document.querySelector('.session-insights-chip')),
        actions: rect(document.querySelector('.header-actions')),
        actionButton: rect(visibleActionButton),
      };
    })()`);
    const headerCenters = ['cluster', 'pill', 'contextActions', 'insightsChip', 'actions', 'actionButton']
      .map((key) => headerAlignment[key]?.center)
      .filter((center) => typeof center === 'number');
    assert(headerCenters.length === 6, `header alignment probe found every target: ${JSON.stringify(headerAlignment)}`);
    assert(
      Math.max(...headerCenters) - Math.min(...headerCenters) <= 1,
      `all header controls share one horizontal center line: ${JSON.stringify(headerAlignment)}`,
    );
    assert(
      Math.abs(headerAlignment.header.center - headerCenters[0]) <= 1.5,
      `header controls are centered in the header band: ${JSON.stringify(headerAlignment)}`,
    );
    assert(await browser.evaluate(`(() => {
      const title = document.querySelector('.header-model-control').title;
      return !title.includes(' / ') && !title.includes('/ ') && !title.includes(' /');
    })()`), "collapsed header context readout has no padded slash");
    const headerPillSpacing = await browser.evaluate(`(() => {
      const control = document.querySelector('.header-model-control');
      const name = control.querySelector('.header-model-control-name');
      const path = control.querySelector('.header-model-control-chevron path');
      const cr = control.getBoundingClientRect();
      const range = document.createRange();
      range.selectNodeContents(name);
      const tr = range.getBoundingClientRect();
      const pr = path.getBoundingClientRect();
      const cs = getComputedStyle(control);
      return {
        leftGap: tr.left - cr.left,
        rightGap: cr.right - pr.right,
        padding: cs.paddingLeft + ' ' + cs.paddingRight,
      };
    })()`);
    assert(Math.abs(headerPillSpacing.leftGap - headerPillSpacing.rightGap) <= 1, `header model pill optical padding is symmetric: ${JSON.stringify(headerPillSpacing)}`);
    const headerClusterSpacing = await browser.evaluate(`(() => {
      const sidebar = document.querySelector('.sidebar');
      const pill = document.querySelector('.header-model-control');
      const divider = document.querySelector('.header-context-actions');
      const sr = sidebar.getBoundingClientRect();
      const pr = pill.getBoundingClientRect();
      const dr = divider.getBoundingClientRect();
      return {
        gapSidebarToPill: pr.left - sr.right,
        gapPillToDivider: dr.left - pr.right,
      };
    })()`);
    assert(Math.abs(headerClusterSpacing.gapSidebarToPill - headerClusterSpacing.gapPillToDivider) <= 1.5, `model pill sits symmetrically between the sidebar edge and the insights divider: ${JSON.stringify(headerClusterSpacing)}`);
    await waitFor(() => browser.evaluate(`document.fonts.check('600 11px "IBM Plex Mono"')`), "bold IBM Plex Mono face loads");
    const modelNameFont = await browser.evaluate(`(() => {
      const name = document.querySelector('.header-model-control-name');
      const style = getComputedStyle(name);
      return { family: style.fontFamily, weight: style.fontWeight, loaded: document.fonts.check('600 11px "IBM Plex Mono"') };
    })()`);
    assert(modelNameFont.family.includes('IBM Plex Mono') && modelNameFont.weight === '600' && modelNameFont.loaded, `model name renders in bold IBM Plex Mono: ${JSON.stringify(modelNameFont)}`);
    assert(await browser.evaluate(`!document.querySelector('.header-actions .header-context-control')`), "the old standalone context chip is gone");
    const modelControlRest = await browser.evaluate(`(() => {
      const control = document.querySelector('.header-model-control');
      const style = getComputedStyle(control);
      return { border: style.borderTopColor, background: style.backgroundImage, shadow: style.boxShadow };
    })()`);
    assert(modelControlRest.border === 'rgba(0, 0, 0, 0)' && modelControlRest.background === 'none' && modelControlRest.shadow === 'none', `header model control is unboxed at rest: ${JSON.stringify(modelControlRest)}`);
    const modelControl = await center(browser, ".header-model-control");
    await browser.call("Input.dispatchMouseEvent", { type: "mouseMoved", x: modelControl.x, y: modelControl.y });
    await sleep(220);
    const modelControlHovered = await browser.evaluate(`(() => {
      const control = document.querySelector('.header-model-control');
      const style = getComputedStyle(control);
      return { border: style.borderTopColor, background: style.backgroundImage };
    })()`);
    assert(modelControlHovered.border !== 'rgba(0, 0, 0, 0)' && modelControlHovered.background !== 'none', `hovering the header model control reveals its pill: ${JSON.stringify(modelControlHovered)}`);
    await click(browser, modelControl);
    await waitFor(() => browser.evaluate(`!!document.querySelector('#header-model-popover')`), "model popover opens");
    const popoverChrome = await browser.evaluate(`(() => {
      const pop = document.querySelector('#header-model-popover');
      const ps = getComputedStyle(pop);
      const heading = document.querySelector('.header-model-popover-heading');
      const hs = heading ? getComputedStyle(heading) : null;
      const action = document.querySelector('.header-model-popover-action');
      const as = getComputedStyle(action);
      const select = document.querySelector('.header-model-popover-select');
      const ss = getComputedStyle(select);
      return {
        radius: ps.borderRadius,
        borderColor: ps.borderTopColor,
        background: ps.backgroundColor,
        headingBackground: hs?.backgroundColor ?? null,
        actionRadius: as.borderRadius,
        actionHeight: Math.round(action.getBoundingClientRect().height),
        actionBackground: as.backgroundColor,
        selectBorder: ss.borderTopColor,
        selectRadius: ss.borderRadius,
      };
    })()`);
    assert(popoverChrome.radius === '9px' && popoverChrome.borderColor === 'rgb(43, 61, 71)' &&
      popoverChrome.background === 'rgb(16, 25, 31)' && popoverChrome.headingBackground === 'rgb(19, 33, 42)' &&
      popoverChrome.actionRadius === '5px' && popoverChrome.actionHeight === 23 &&
      popoverChrome.actionBackground === 'rgba(0, 0, 0, 0)' &&
      popoverChrome.selectBorder === 'rgb(34, 52, 61)' && popoverChrome.selectRadius === '5px',
      `model popover adopts the endpoint card language: ${JSON.stringify(popoverChrome)}`);
    const lightPopoverChrome = await browser.evaluate(`(() => {
      const original = document.documentElement.dataset.theme || 'dark';
      document.documentElement.dataset.theme = 'light';
      const pop = document.querySelector('#header-model-popover');
      const ps = getComputedStyle(pop);
      const heading = document.querySelector('.header-model-popover-heading');
      const hs = heading ? getComputedStyle(heading) : null;
      const contextValue = document.querySelector('.header-model-popover-context > span:last-child');
      const lengthUnit = document.querySelector('.header-model-popover-length span');
      const result = {
        background: ps.backgroundColor,
        borderColor: ps.borderTopColor,
        headingBackground: hs?.backgroundColor ?? null,
        contextColor: contextValue ? getComputedStyle(contextValue).color : null,
        lengthUnitColor: lengthUnit ? getComputedStyle(lengthUnit).color : null,
      };
      document.documentElement.dataset.theme = original;
      return result;
    })()`);
    assert(lightPopoverChrome.background === 'rgb(255, 255, 255)' &&
      lightPopoverChrome.borderColor === 'rgb(201, 214, 218)' &&
      lightPopoverChrome.headingBackground === 'rgb(243, 248, 249)' &&
      lightPopoverChrome.contextColor === 'rgb(51, 72, 79)' &&
      lightPopoverChrome.lengthUnitColor === 'rgb(109, 129, 137)',
      `model popover mirrors the endpoint card in light theme: ${JSON.stringify(lightPopoverChrome)}`);
    const headerPopoverLayout = await browser.evaluate(`(() => {
      const labels = [...document.querySelectorAll('.header-model-popover-label')];
      const control = document.querySelector('.header-model-control');
      const controlStyle = getComputedStyle(control);
      return {
        labels: labels.map((el) => {
          const r = el.getBoundingClientRect();
          const style = getComputedStyle(el);
          return { text: el.textContent, h: Math.round(r.height), ws: style.whiteSpace };
        }),
        controlHeight: Math.round(control.getBoundingClientRect().height),
        controlBackground: controlStyle.backgroundImage,
        controlBorder: controlStyle.borderTopWidth,
        pillSize: getComputedStyle(document.querySelector('.header-model-control-reasoning')).fontSize,
        reasoningSelectWidth: Math.round(document.querySelector('#header-model-popover .header-model-popover-select').getBoundingClientRect().width),
      };
    })()`);
    assert(headerPopoverLayout.labels.length === 4 && headerPopoverLayout.labels.every((label) => label.h <= 16 && label.ws === 'nowrap'), `popover labels stay on one line: ${JSON.stringify(headerPopoverLayout)}`);
    assert(headerPopoverLayout.controlHeight === 30 && headerPopoverLayout.controlBorder === '1px' && headerPopoverLayout.controlBackground.includes('linear-gradient'), `header control surface: ${JSON.stringify(headerPopoverLayout)}`);
    assert(headerPopoverLayout.pillSize === '8.5px', `reasoning pill size: ${JSON.stringify(headerPopoverLayout)}`);
    assert(headerPopoverLayout.reasoningSelectWidth <= 120, `reasoning select hugs its content: ${JSON.stringify(headerPopoverLayout)}`);
    assert(await browser.evaluate(`document.querySelector('#header-model-popover').textContent.includes('Context management')`), "popover carries the context management row");
    assert(await browser.evaluate(`document.querySelector('#header-model-popover .header-model-popover-length input').value === '16000'`), "popover shows the current context length");
    assert(await browser.evaluate(`document.querySelector('#header-model-popover .header-model-popover-context').textContent.includes('1.0K/16K')`), "context row shows used and limit tokens");
    assert(await browser.evaluate(`!!document.querySelector('#header-model-popover .header-model-popover-context .context-donut')`), "context row renders the donut");
    assert(await browser.evaluate(`(() => {
      const buttons = [...document.querySelectorAll('#header-model-popover .header-model-popover-length-actions button')];
      return buttons.length === 2 && buttons[0].disabled && buttons[1].disabled;
    })()`), "apply and default start disabled");
    // Clear is unavailable while the session is busy (it would discard the
    // active turn's context); it appears once the session is idle.
    assert(await browser.evaluate(`(() => {
      const buttons = [...document.querySelectorAll('#header-model-popover .header-model-popover-context-actions button')];
      return buttons.map(b => b.textContent).join(',') === 'Compress';
    })()`), "busy session offers compress only");
    host.setSession({ ...makeSession(), state: "ready" });
    await waitFor(() => browser.evaluate(`(() => {
      const buttons = [...document.querySelectorAll('#header-model-popover .header-model-popover-context-actions button')];
      return buttons.map(b => b.textContent).join(',') === 'Compress,Clear';
    })()`), "idle session offers compress and clear");
    const compressPoint = await browser.evaluate(`(() => { const b = document.querySelectorAll('#header-model-popover .header-model-popover-context-actions button')[0]; const r = b.getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2 }; })()`);
    await click(browser, compressPoint);
    await waitFor(() => host.commands.some((command) => command.type === 'session_request_context_compress'), "context compression command");
    await waitFor(() => browser.evaluate(`(() => {
      const buttons = [...document.querySelectorAll('#header-model-popover .header-model-popover-context-actions button')];
      return buttons.length === 2
        && buttons[0].textContent === 'Compressing...'
        && buttons[0].disabled
        && buttons[1].textContent === 'Clear'
        && buttons[1].disabled;
    })()`), "compressing locks compress and clear");
    const lockedCompressPoint = await browser.evaluate(`(() => { const b = document.querySelectorAll('#header-model-popover .header-model-popover-context-actions button')[0]; const r = b.getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2 }; })()`);
    await click(browser, lockedCompressPoint);
    await sleep(120);
    assert(host.commands.filter((command) => command.type === 'session_request_context_compress').length === 1, "compressing ignores a second compress click");
    assert(!host.commands.some((command) => command.type === 'session_clear_context'), "compressing never clears the context");
    host.setSession(makeSession());
    await waitFor(() => browser.evaluate(`document.querySelectorAll('#header-model-popover .header-model-popover-context-actions button').length === 1`), "busy session hides clear again");
    await browser.evaluate(`(() => {
      const select = document.querySelector('.header-model-popover-select');
      const setter = Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, 'value').set;
      setter.call(select, 'medium');
      select.dispatchEvent(new Event('change', { bubbles: true }));
    })()`);
    await waitFor(() => host.commands.some((command) => command.type === 'session_reasoning_effort_update' && command.reasoning_effort === 'medium'), "reasoning update command");
    await waitFor(() => browser.evaluate(`document.querySelector('.header-model-control-reasoning').textContent === 'medium'`), "collapsed reasoning follows the host projection");
    await browser.evaluate(`(() => {
      const input = document.querySelector('#header-model-popover .header-model-popover-length input');
      const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set;
      setter.call(input, '32000');
      input.dispatchEvent(new Event('input', { bubbles: true }));
    })()`);
    assert(await browser.evaluate(`!document.querySelectorAll('#header-model-popover .header-model-popover-length-actions button')[0].disabled`), "apply enables after a dirty draft");
    const applyPoint = await browser.evaluate(`(() => { const b = document.querySelectorAll('#header-model-popover .header-model-popover-length-actions button')[0]; const r = b.getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2 }; })()`);
    await click(browser, applyPoint);
    await waitFor(() => host.commands.some((command) => command.type === 'session_endpoint_max_input_tokens_update' && command.endpoint_id === 'ep-header' && command.max_input_tokens === 32000), "context length update command");
    await waitFor(() => browser.evaluate(`document.querySelector('#header-model-popover .header-model-popover-length input').value === '32000'
      && document.querySelector('#header-model-popover .header-model-popover-context').textContent.includes('1.0K/32K')`), "applied context window comes back from the host");
    await waitFor(() => browser.evaluate(`!document.querySelectorAll('#header-model-popover .header-model-popover-length-actions button')[1].disabled`), "default enables once an override exists");
    const resetPoint = await browser.evaluate(`(() => { const b = document.querySelectorAll('#header-model-popover .header-model-popover-length-actions button')[1]; const r = b.getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2 }; })()`);
    await click(browser, resetPoint);
    await waitFor(() => host.commands.some((command) => command.type === 'session_endpoint_max_input_tokens_update' && command.max_input_tokens === null), "context length reset command");
    await waitFor(() => browser.evaluate(`document.querySelector('#header-model-popover .header-model-popover-length input').value === '16000'
      && document.querySelector('#header-model-popover .header-model-popover-context').textContent.includes('1.0K/16K')
      && document.querySelectorAll('#header-model-popover .header-model-popover-length-actions button')[1].disabled`), "reset restores the endpoint default");
    // An unsaved draft is discarded when the popover closes.
    await browser.evaluate(`(() => {
      const input = document.querySelector('#header-model-popover .header-model-popover-length input');
      const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set;
      setter.call(input, '33000');
      input.dispatchEvent(new Event('input', { bubbles: true }));
    })()`);
    await waitFor(() => browser.evaluate(`document.querySelector('#header-model-popover .header-model-popover-length input').value === '33000'`), "dirty draft typed");
    await browser.call("Input.dispatchKeyEvent", { type: "keyDown", key: "Escape", code: "Escape" });
    await browser.call("Input.dispatchKeyEvent", { type: "keyUp", key: "Escape", code: "Escape" });
    await waitFor(() => browser.evaluate(`!document.querySelector('#header-model-popover')`), "Escape closes the model popover");
    await click(browser, modelControl);
    await waitFor(() => browser.evaluate(`document.querySelector('#header-model-popover .header-model-popover-length input').value === '16000'`), "reopening discards the unsaved draft");
    const switchPoint = await browser.evaluate(`(() => { const b = document.querySelector('#header-model-popover .header-model-popover-row .header-model-popover-action'); const r = b.getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2 }; })()`);
    await click(browser, switchPoint);
    await waitFor(() => browser.evaluate(`!!document.querySelector('.endpoint-menu')`), "Switch opens the model endpoint panel");
    assert(await browser.evaluate(`!!document.querySelector('.endpoint-menu-close')`), "endpoint panel exposes a close control");
    const endpointPanelLayout = await browser.evaluate(`(() => {
      const box = (el) => el ? el.getBoundingClientRect().toJSON() : null;
      const menu = document.querySelector('.endpoint-menu');
      const captureTheme = () => {
        const menuStyle = getComputedStyle(menu);
        const headingStyle = getComputedStyle(document.querySelector('.endpoint-menu-heading'));
        return {
          menu: {
            background: menuStyle.backgroundColor,
            border: menuStyle.borderTopColor,
            radius: menuStyle.borderTopLeftRadius,
            shadow: menuStyle.boxShadow,
          },
          heading: {
            background: headingStyle.backgroundColor,
            border: headingStyle.borderBottomColor,
          },
        };
      };
      const originalTheme = document.documentElement.dataset.theme || 'dark';
      document.documentElement.dataset.theme = 'dark';
      const darkTheme = captureTheme();
      document.documentElement.dataset.theme = 'light';
      const lightTheme = captureTheme();
      document.documentElement.dataset.theme = originalTheme;
      const chainTitleStyle = getComputedStyle(document.querySelector('.endpoint-chain-title'));
      const availableRow = document.querySelector('.endpoint-chain-col.available .endpoint-chain-row');
      const availableStyle = getComputedStyle(availableRow);
      const picks = [...document.querySelectorAll('.endpoint-chain-pick')].map((node) => {
        const cs = getComputedStyle(node);
        return { paddingLeft: cs.paddingLeft, borderLeftWidth: cs.borderLeftWidth };
      });
      const efforts = [...document.querySelectorAll('.endpoint-chain-effort')].map((node) => Math.round(node.getBoundingClientRect().width * 10) / 10);
      return {
        menuWidth: Math.round(menu.getBoundingClientRect().width),
        themes: { dark: darkTheme, light: lightTheme },
        chainTitle: {
          font: chainTitleStyle.fontFamily,
          size: chainTitleStyle.fontSize,
          transform: chainTitleStyle.textTransform,
        },
        availableBorder: availableStyle.borderTopColor,
        availableBackground: availableStyle.backgroundColor,
        close: box(document.querySelector('.endpoint-menu-close')),
        picks,
        efforts,
      };
    })()`);
    assert(endpointPanelLayout.menuWidth >= 460 && endpointPanelLayout.menuWidth <= 600, `endpoint panel width follows its content: ${JSON.stringify(endpointPanelLayout)}`);
    assert(endpointPanelLayout.close.width === 23 && endpointPanelLayout.close.height === 23, `endpoint close target: ${JSON.stringify(endpointPanelLayout)}`);
    const { dark: darkEndpointTheme, light: lightEndpointTheme } = endpointPanelLayout.themes;
    assert(
      darkEndpointTheme.menu.radius === '9px' &&
        darkEndpointTheme.menu.background === 'rgb(16, 25, 31)' &&
        darkEndpointTheme.menu.border === 'rgb(43, 61, 71)' &&
        darkEndpointTheme.menu.shadow.includes('18px'),
      `dark endpoint panel adopts the session insights card surface: ${JSON.stringify(darkEndpointTheme.menu)}`,
    );
    assert(
      lightEndpointTheme.menu.radius === '9px' &&
        lightEndpointTheme.menu.background === 'rgb(255, 255, 255)' &&
        lightEndpointTheme.menu.border === 'rgb(201, 214, 218)' &&
        lightEndpointTheme.menu.shadow.includes('18px'),
      `light endpoint panel adopts the session insights card surface: ${JSON.stringify(lightEndpointTheme.menu)}`,
    );
    assert(
      darkEndpointTheme.heading.background === 'rgb(19, 33, 42)' && darkEndpointTheme.heading.border === 'rgb(34, 50, 58)',
      `dark endpoint panel heading uses the quiet insights band: ${JSON.stringify(darkEndpointTheme.heading)}`,
    );
    assert(
      lightEndpointTheme.heading.background === 'rgb(243, 248, 249)' && lightEndpointTheme.heading.border === 'rgb(219, 229, 232)',
      `light endpoint panel heading uses the quiet insights band: ${JSON.stringify(lightEndpointTheme.heading)}`,
    );
    assert(
      endpointPanelLayout.chainTitle.font.includes('Space Grotesk') &&
        endpointPanelLayout.chainTitle.size === '9px' &&
        endpointPanelLayout.chainTitle.transform === 'uppercase',
      `endpoint column titles share the insights title voice: ${JSON.stringify(endpointPanelLayout.chainTitle)}`,
    );
    assert(endpointPanelLayout.availableBorder === 'rgba(0, 0, 0, 0)' && endpointPanelLayout.availableBackground === 'rgba(0, 0, 0, 0)', `available endpoint rows are flat at rest, not buttons: ${JSON.stringify(endpointPanelLayout)}`);
    assert(await browser.evaluate(`document.querySelectorAll('.endpoint-chain-active-badge').length === 0`), "no in-use check badge in either column");
    const selectedCurrentRow = await browser.evaluate(`(() => {
      const row = document.querySelector('.endpoint-chain-col.selected .endpoint-chain-row.current');
      if (!row) return null;
      const style = getComputedStyle(row);
      return {
        badges: row.querySelectorAll('.endpoint-chain-active-badge').length,
        borderColor: style.borderTopColor,
        shadow: style.boxShadow,
      };
    })()`);
    assert(selectedCurrentRow, "selected chain marks the in-use endpoint");
    assert(selectedCurrentRow.badges === 0, `the in-use selected row does not use a check badge: ${JSON.stringify(selectedCurrentRow)}`);
    assert(selectedCurrentRow.borderColor === 'rgb(59, 130, 246)' && !selectedCurrentRow.shadow.includes('59, 130, 246'), `the in-use selected row is marked by a blue border only: ${JSON.stringify(selectedCurrentRow)}`);
    assert(endpointPanelLayout.picks.every((pick) => pick.paddingLeft === '0px' && pick.borderLeftWidth === '0px'), `chain rows keep the runtime-card divider out: ${JSON.stringify(endpointPanelLayout.picks)}`);
    assert(endpointPanelLayout.efforts.every((width) => width <= 84.5), `reasoning selects stay compact: ${JSON.stringify(endpointPanelLayout.efforts)}`);
    // Chain edits are a pending draft until the Host projection confirms them.
    const chainRow = await center(browser, ".endpoint-chain-col.available .endpoint-chain-row");
    await browser.call("Input.dispatchMouseEvent", { type: "mouseMoved", x: chainRow.x, y: chainRow.y });
    await sleep(220);
    const availableHovered = await browser.evaluate(`(() => {
      const row = document.querySelector('.endpoint-chain-col.available .endpoint-chain-row');
      const style = getComputedStyle(row);
      return { border: style.borderTopColor, background: style.backgroundColor };
    })()`);
    assert(availableHovered.border !== 'rgba(0, 0, 0, 0)' && availableHovered.background !== 'rgba(0, 0, 0, 0)', `hovering an available endpoint reveals its surface: ${JSON.stringify(availableHovered)}`);
    await click(browser, chainRow);
    await waitFor(() => browser.evaluate(`document.querySelector('.endpoint-chain-pending')?.textContent === 'Applying…'`), "chain edit marks the pending draft");
    await waitFor(() => host.commands.some((command) => command.type === 'model_endpoint_chain_apply' && Array.isArray(command.endpoint_ids) && command.endpoint_ids.length === 0), "chain removal command");
    await waitFor(() => browser.evaluate(`!document.querySelector('.endpoint-chain-pending') && document.querySelectorAll('.endpoint-chain-col.selected .endpoint-chain-row').length === 0`), "chain removal is confirmed by the host");
    const restoreChainRow = await center(browser, ".endpoint-chain-col.available .endpoint-chain-row");
    await click(browser, restoreChainRow);
    await waitFor(() => host.commands.some((command) => command.type === 'model_endpoint_chain_apply' && Array.isArray(command.endpoint_ids) && command.endpoint_ids.length === 1), "chain restore command");
    await waitFor(() => browser.evaluate(`document.querySelectorAll('.endpoint-chain-col.selected .endpoint-chain-row').length === 1 && !document.querySelector('.endpoint-chain-pending')`), "chain restore is confirmed by the host");
    const closeEndpointPanel = await center(browser, ".endpoint-menu-close");
    await click(browser, closeEndpointPanel);
    await waitFor(() => browser.evaluate(`!document.querySelector('.endpoint-menu')`), "close control dismisses the endpoint panel");
    // The MCP dropdown belongs to the top-right trigger; it must not open at
    // the old fixed left inset.
    const mcpButtonPoint = await center(browser, ".mcp-button");
    await click(browser, mcpButtonPoint);
    await waitFor(() => browser.evaluate(`!!document.querySelector('.mcp-panel')`), "MCP panel opens");
    const mcpPanelLayout = await browser.evaluate(`(() => {
      const button = document.querySelector('.mcp-button').getBoundingClientRect();
      const panel = document.querySelector('.mcp-panel').getBoundingClientRect();
      return { button: button.toJSON(), panel: panel.toJSON(), viewport: innerWidth };
    })()`);
    assert(Math.abs(mcpPanelLayout.panel.right - mcpPanelLayout.button.right) <= 1, `MCP panel anchors to its trigger: ${JSON.stringify(mcpPanelLayout)}`);
    assert(mcpPanelLayout.panel.left > mcpPanelLayout.viewport / 2, `MCP panel opens on the right half: ${JSON.stringify(mcpPanelLayout)}`);
    const closeMcpPanelPoint = await center(browser, ".mcp-panel-header-actions .icon-button");
    await click(browser, closeMcpPanelPoint);
    await waitFor(() => browser.evaluate(`!document.querySelector('.mcp-panel')`), "MCP panel closes");
    // Timers and session info are anchored cards too. They must not replace
    // the worker-role sidebar or open inside the grid as a third column.
    for (const [buttonSelector, panelSelector] of [
      [".timer-header-button", "#timer-panel"],
      [".mailbox-header-button", "#mailbox-panel"],
    ]) {
      await click(browser, await center(browser, buttonSelector));
      await waitFor(() => browser.evaluate(`!!document.querySelector(${JSON.stringify(panelSelector)})`), `${panelSelector} opens`);
      const cardLayout = await browser.evaluate(`(() => {
        const button = document.querySelector(${JSON.stringify(buttonSelector)}).getBoundingClientRect();
        const panel = document.querySelector(${JSON.stringify(panelSelector)});
        const rect = panel.getBoundingClientRect();
        return {
          button: button.toJSON(),
          panel: rect.toJSON(),
          position: getComputedStyle(panel).position,
          radius: getComputedStyle(panel).borderTopLeftRadius,
          sidebar: !!document.querySelector('.worker-role-panel'),
          viewport: innerWidth,
        };
      })()`);
      assert(cardLayout.position === 'absolute', `session card floats: ${JSON.stringify(cardLayout)}`);
      assert(cardLayout.radius === '12px', `session card uses the shared card radius: ${JSON.stringify(cardLayout)}`);
      assert(Math.abs(cardLayout.panel.right - cardLayout.button.right) <= 1, `session card anchors to its trigger: ${JSON.stringify(cardLayout)}`);
      assert(cardLayout.panel.top > cardLayout.button.bottom && cardLayout.panel.top - cardLayout.button.bottom <= 32, `session card drops just below its trigger: ${JSON.stringify(cardLayout)}`);
      assert(cardLayout.panel.left > cardLayout.viewport / 2, `session card opens on the right half: ${JSON.stringify(cardLayout)}`);
      assert(cardLayout.sidebar, `worker-role sidebar stays mounted: ${JSON.stringify(cardLayout)}`);
      await click(browser, await center(browser, `${panelSelector} .side-panel-header .icon-button`));
      await waitFor(() => browser.evaluate(`!document.querySelector(${JSON.stringify(panelSelector)})`), `${panelSelector} closes`);
    }
    assert(await browser.evaluate(`!!document.querySelector('.mailbox-header-button svg.lucide-messages-square')`), "session info uses the direct-conversation icon");
    // A session without an endpoint chain falls back to the session-level
    // runtime setting; the Host projection still drives the next request.
    const unbound = makeSession();
    unbound.runtime_profile = {
      ...unbound.runtime_profile,
      // The effort pill is an observable proof that this snapshot (and not the
      // previous bound one) has been applied before the next interaction.
      reasoning_effort: "low",
      model_endpoint_id: null,
      active_model_endpoint_id: null,
      model_endpoint_ids: [],
    };
    host.setSession(unbound);
    await waitFor(() => browser.evaluate(`document.querySelector('.header-model-control-reasoning')?.textContent === 'low'`), "unbound snapshot applied");
    assert(await browser.evaluate(`document.querySelector('.header-model-control-name').textContent === 'header-model-1'`), "unbound header keeps the model identity");
    const unboundControl = await center(browser, ".header-model-control");
    await click(browser, unboundControl);
    await waitFor(() => browser.evaluate(`!!document.querySelector('#header-model-popover')`), "unbound popover opens");
    await browser.evaluate(`(() => {
      const select = document.querySelector('.header-model-popover-select');
      const setter = Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, 'value').set;
      setter.call(select, 'high');
      select.dispatchEvent(new Event('change', { bubbles: true }));
    })()`);
    await waitFor(() => host.commands.some((command) => command.type === 'session_runtime_update' && command.key === 'TIMEM_REASONING_EFFORT' && command.value === 'high'), "unbound reasoning falls back to the session runtime setting");
    await waitFor(() => browser.evaluate(`document.querySelector('.header-model-control-reasoning').textContent === 'high'`), "unbound reasoning is echoed by the host");
    await browser.evaluate(`(() => {
      const input = document.querySelector('#header-model-popover .header-model-popover-length input');
      const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set;
      setter.call(input, '24000');
      input.dispatchEvent(new Event('input', { bubbles: true }));
    })()`);
    await waitFor(() => browser.evaluate(`!document.querySelectorAll('#header-model-popover .header-model-popover-length-actions button')[0].disabled`), "unbound apply enables after a dirty draft");
    const unboundApply = await browser.evaluate(`(() => { const b = document.querySelectorAll('#header-model-popover .header-model-popover-length-actions button')[0]; const r = b.getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2 }; })()`);
    await click(browser, unboundApply);
    await waitFor(() => host.commands.some((command) => command.type === 'session_runtime_update' && command.key === 'TIMEM_MAX_LLM_INPUT' && command.value === '24000'), "unbound context update falls back to the session runtime setting");
    await waitFor(() => browser.evaluate(`document.querySelector('#header-model-popover .header-model-popover-length input').value === '24000'`), "session-level context update is echoed by the host");
    host.setSession(makeSession());
    await browser.call("Input.dispatchKeyEvent", { type: "keyDown", key: "Escape", code: "Escape" });
    await browser.call("Input.dispatchKeyEvent", { type: "keyUp", key: "Escape", code: "Escape" });
    await waitFor(() => browser.evaluate(`!document.querySelector('#header-model-popover')`), "unbound popover closes");

    // Collapsed header: only the top item + chevron, no title bar.
    assert(await browser.evaluate(`!!document.querySelector('.session-insights-chip')`), "compact chip exists in the header");
    assert(await browser.evaluate(`!document.querySelector('.session-insights-panel')`), "panel starts collapsed");
    assert(await browser.evaluate(`!!document.querySelector('.session-insights-shield.full')`), "collapsed header shows the full-protection shield");
    await browser.evaluate(`document.documentElement.dataset.theme = 'dark'`);
    await sleep(200);
    const fullShieldStyle = await browser.evaluate(`(() => {
      const el = document.querySelector('.session-insights-shield.full');
      if (!el) return null;
      const style = getComputedStyle(el);
      const rect = el.getBoundingClientRect();
      const silhouette = el.querySelector('svg path');
      return {
        background: style.backgroundColor,
        color: style.color,
        width: rect.width,
        height: rect.height,
        silhouetteFill: silhouette ? getComputedStyle(silhouette).fill : null,
        silhouetteStroke: silhouette ? getComputedStyle(silhouette).stroke : null,
      };
    })()`);
    assert(fullShieldStyle?.background === 'rgba(0, 0, 0, 0)', `full shield keeps a transparent button field: ${fullShieldStyle?.background}`);
    assert(fullShieldStyle?.silhouetteFill === 'rgb(22, 163, 74)', `only the shield silhouette is filled vivid green: ${fullShieldStyle?.silhouetteFill}`);
    assert(fullShieldStyle?.color === 'rgb(255, 255, 255)' && fullShieldStyle?.silhouetteStroke === 'rgb(255, 255, 255)', `full shield keeps white lines: ${JSON.stringify(fullShieldStyle)}`);
    assert(fullShieldStyle?.width === 20 && fullShieldStyle?.height === 20, `full shield hit target stays 20x20: ${JSON.stringify(fullShieldStyle)}`);
    const chipTextStyle = await browser.evaluate(`(() => {
      const text = document.querySelector('.session-insights-chip-text');
      const label = document.querySelector('.session-insights-chip-label');
      const value = document.querySelector('.session-insights-chip-value');
      if (!text || !label || !value) return null;
      return {
        alignItems: getComputedStyle(text).alignItems,
        labelInText: text.contains(label),
        valueInText: text.contains(value),
      };
    })()`);
    assert(chipTextStyle?.alignItems === 'baseline', `chip label and value share a baseline: ${chipTextStyle?.alignItems}`);
    assert(chipTextStyle?.labelInText && chipTextStyle?.valueInText, "chip label and value live in one baseline-aligned row");
    const shieldPoint = await center(browser, ".session-insights-shield.full");
    await browser.call("Input.dispatchMouseEvent", { type: "mouseMoved", x: shieldPoint.x, y: shieldPoint.y });
    await sleep(260);
    const hoveredShield = await browser.evaluate(`(() => {
      const el = document.querySelector('.session-insights-shield.full');
      const style = getComputedStyle(el);
      const silhouette = el.querySelector('svg path');
      return { background: style.backgroundColor, fill: getComputedStyle(silhouette).fill, color: style.color };
    })()`);
    assert(hoveredShield.background === 'rgb(23, 36, 43)', `hover uses a neutral field instead of a green square: ${JSON.stringify(hoveredShield)}`);
    assert(hoveredShield.fill === 'rgb(22, 163, 74)' && hoveredShield.color === 'rgb(255, 255, 255)', `hover keeps the green silhouette and white lines: ${JSON.stringify(hoveredShield)}`);
    await browser.call("Input.dispatchMouseEvent", { type: "mouseMoved", x: 8, y: 8 });
    await sleep(200);
    await browser.evaluate(`document.documentElement.dataset.theme = 'light'`);
    await sleep(200);
    const lightShield = await browser.evaluate(`(() => {
      const el = document.querySelector('.session-insights-shield.full');
      const style = getComputedStyle(el);
      const silhouette = el.querySelector('svg path');
      return { background: style.backgroundColor, fill: getComputedStyle(silhouette).fill, color: style.color };
    })()`);
    assert(lightShield.background === 'rgba(0, 0, 0, 0)' && lightShield.fill === 'rgb(22, 163, 74)' && lightShield.color === 'rgb(255, 255, 255)', `light theme keeps the green silhouette only: ${JSON.stringify(lightShield)}`);
    const lightShieldPoint = await center(browser, ".session-insights-shield.full");
    await browser.call("Input.dispatchMouseEvent", { type: "mouseMoved", x: lightShieldPoint.x, y: lightShieldPoint.y });
    // The hover surface cross-fades in; wait for the transition to settle so
    // slow runners do not sample the mid-fade alpha instead of the endpoint.
    await waitFor(
      () => browser.evaluate(`getComputedStyle(document.querySelector('.session-insights-shield.full')).backgroundColor === 'rgb(238, 245, 247)'`),
      "light hover settles on the neutral surface",
    );
    const lightHover = await browser.evaluate(`(() => {
      const el = document.querySelector('.session-insights-shield.full');
      return { background: getComputedStyle(el).backgroundColor, fill: getComputedStyle(el.querySelector('svg path')).fill };
    })()`);
    assert(lightHover.background === 'rgb(238, 245, 247)' && lightHover.fill === 'rgb(22, 163, 74)', `light hover uses the neutral surface and keeps the green silhouette: ${JSON.stringify(lightHover)}`);
    await browser.call("Input.dispatchMouseEvent", { type: "mouseMoved", x: 8, y: 8 });
    await browser.evaluate(`document.documentElement.dataset.theme = 'dark'`);
    await sleep(200);
    assert(await browser.evaluate(`document.querySelector('.session-insights-shield').title.includes('Full protection')`), "shield exposes the current protection status on hover");
    host.setResourceCaps(resourceCaps("applied", "disabled"));
    await waitFor(() => browser.evaluate(`!!document.querySelector('.session-insights-shield.partial')`), "partial-protection shield");
    assert(await browser.evaluate(`document.querySelector('.session-insights-shield').title.includes('Partial protection: CPU capped at 85% only')`), "partial shield hover text");
    host.setResourceCaps(resourceCaps("disabled", "unavailable"));
    // Nothing enforced means nothing to guard: the shield disappears from
    // the header instead of showing a shield-x badge.
    await waitFor(() => browser.evaluate(`!document.querySelector('.session-insights-shield')`), "no-protection state hides the shield entirely");
    host.setResourceCaps(fullResourceCaps);
    await waitFor(() => browser.evaluate(`!!document.querySelector('.session-insights-shield.full')`), "full-protection shield restored");
    const shield = await center(browser, ".session-insights-shield");
    await click(browser, shield);
    await waitFor(() => browser.evaluate(`!!document.querySelector('#settings-center .system-resource-caps')`), "shield opens system resource settings");
    assert(await browser.evaluate(`document.querySelector('.settings-center-nav button.active')?.textContent.includes('System')`), "system settings section is active");
    await browser.evaluate(`document.querySelectorAll('.settings-center-nav button')[1].click()`);
    await waitFor(() => browser.evaluate(`!!document.querySelector('.endpoint-settings-pane')`), "endpoint settings pane opens for the spacing regression");
    await sleep(200);
    const spacing = () => browser.evaluate(`(() => {
      const content = document.querySelector('.settings-center-content');
      const nav = document.querySelector('.settings-center-nav');
      const pane = document.querySelector('.endpoint-settings-pane');
      const modal = document.querySelector('.settings-center');
      const nr = nav.getBoundingClientRect();
      const pr = pane.getBoundingClientRect();
      const mr = modal.getBoundingClientRect();
      return {
        leftGap: pr.left - nr.right,
        rightGap: mr.right - pr.right,
        gutter: getComputedStyle(content).scrollbarGutter,
        scrollbar: content.offsetWidth - content.clientWidth,
        scrollable: content.scrollHeight > content.clientHeight,
      };
    })()`);
    const spacingAtRest = await spacing();
    assert(spacingAtRest.gutter === 'stable both-edges', `settings content reserves symmetric scrollbar gutters: ${JSON.stringify(spacingAtRest)}`);
    assert(Math.abs(spacingAtRest.leftGap - spacingAtRest.rightGap) <= 1, `endpoint pane is centered between the nav and modal edge at rest: ${JSON.stringify(spacingAtRest)}`);
    await browser.evaluate(`(() => { const spacer = document.createElement('div'); spacer.id = 'spacing-spacer'; spacer.style.height = '1600px'; document.querySelector('.settings-center-content').appendChild(spacer); })()`);
    await waitFor(() => browser.evaluate(`(() => { const c = document.querySelector('.settings-center-content'); return c.scrollHeight > c.clientHeight; })()`), "endpoint content becomes scrollable");
    const spacingScrolled = await spacing();
    assert(spacingScrolled.scrollable, `spacing probe reproduces a scrollable endpoint pane: ${JSON.stringify(spacingScrolled)}`);
    assert(Math.abs(spacingScrolled.leftGap - spacingScrolled.rightGap) <= 1, `a visible scrollbar does not shift the endpoint pane: ${JSON.stringify(spacingScrolled)}`);
    await browser.evaluate(`document.querySelector('#spacing-spacer')?.remove()`);
    const rowSurfaceAtRest = await browser.evaluate(`(() => {
      const row = document.querySelector('.endpoint-settings-row');
      const style = getComputedStyle(row);
      return { background: style.backgroundColor, shadow: style.boxShadow, borderTop: style.borderTopWidth };
    })()`);
    assert(rowSurfaceAtRest.background === 'rgba(0, 0, 0, 0)' && rowSurfaceAtRest.shadow === 'none' && parseFloat(rowSurfaceAtRest.borderTop) === 0, `endpoint row is a flat list row at rest, not a button: ${JSON.stringify(rowSurfaceAtRest)}`);
    const rowActionsAtRest = await browser.evaluate(`(() => {
      const actions = document.querySelector('.endpoint-settings-row .endpoint-settings-actions');
      const style = getComputedStyle(actions);
      return { visibility: style.visibility, opacity: style.opacity };
    })()`);
    assert(rowActionsAtRest.visibility === 'hidden' && parseFloat(rowActionsAtRest.opacity) === 0, `endpoint row actions stay hidden until hover: ${JSON.stringify(rowActionsAtRest)}`);
    const rowPoint = await center(browser, ".endpoint-settings-row");
    await browser.call("Input.dispatchMouseEvent", { type: "mouseMoved", x: rowPoint.x, y: rowPoint.y });
    await sleep(280);
    const rowActionsHovered = await browser.evaluate(`(() => {
      const actions = document.querySelector('.endpoint-settings-row .endpoint-settings-actions');
      const style = getComputedStyle(actions);
      return { visibility: style.visibility, opacity: style.opacity };
    })()`);
    assert(rowActionsHovered.visibility === 'visible' && parseFloat(rowActionsHovered.opacity) === 1, `hovering the endpoint row reveals its actions: ${JSON.stringify(rowActionsHovered)}`);
    const rowSurfaceHovered = await browser.evaluate(`(() => {
      const row = document.querySelector('.endpoint-settings-row');
      const style = getComputedStyle(row);
      return { background: style.backgroundColor, shadow: style.boxShadow };
    })()`);
    assert(rowSurfaceHovered.background !== 'rgba(0, 0, 0, 0)' && rowSurfaceHovered.shadow !== 'none', `hovering the endpoint row reveals its surface: ${JSON.stringify(rowSurfaceHovered)}`);
    const editPoint = await center(browser, ".endpoint-settings-edit");
    await browser.call("Input.dispatchMouseEvent", { type: "mouseMoved", x: editPoint.x, y: editPoint.y });
    await sleep(200);
    await click(browser, editPoint);
    await waitFor(() => browser.evaluate(`!!document.querySelector('.endpoint-settings-pane.editing')`), "hover-revealed edit action is really clickable");
    const closeSettings = await center(browser, ".settings-center-header .icon-button");
    await click(browser, closeSettings);
    await waitFor(() => browser.evaluate(`!document.querySelector('#settings-center')`), "settings closed after shield jump");
    assert(await browser.evaluate(`document.querySelector('.session-insights-chip-label').textContent === 'tokens/s'`), "default top item is the token speed label");
    const chipValueTracking = await browser.evaluate(`getComputedStyle(document.querySelector('.session-insights-chip-value')).letterSpacing`);
    assert(parseFloat(chipValueTracking) <= -0.1, `chip value tracking tightened: ${chipValueTracking}`);
    assert(await browser.evaluate(`!document.querySelector('.chat-header').textContent.includes('Session Insights')`), "collapsed header does not render the Session Insights title");

    const chip = await center(browser, ".session-insights-chip");
    await click(browser, chip);
    await waitFor(() => browser.evaluate(`!!document.querySelector('.session-insights-panel')`), "panel opens on chip click");
    assert(await browser.evaluate(`document.querySelectorAll('.session-insight-row').length === 10`), "panel lists all ten items");
    assert(await browser.evaluate(`!document.querySelector('.session-insights-shield')`), "shield leaves the header while the panel is open");
    // Typography: compact and refined; values keep the bold tabular scale,
    // labels stay sentence-case at a small size, especially for CJK text.
    await waitFor(() => browser.evaluate(`document.fonts.check('600 9.5px "Space Grotesk"')
      && document.fonts.check('400 10px "IBM Plex Mono"')
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
        valueLetterSpacing: value.letterSpacing,
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
    assert(typography.valueSize === '10px' && typography.valueWeight === '400', `value typography: ${JSON.stringify(typography)}`);
    assert(typography.valueFamily.includes('IBM Plex Mono'), `value font family: ${JSON.stringify(typography)}`);
    assert(typography.labelSize === '9.5px' && typography.labelWeight === '500' && typography.labelTransform === 'none', `label typography: ${JSON.stringify(typography)}`);
    assert(typography.panelFamily.includes('Source Han Sans SC') && typography.labelFamily.includes('Noto Sans CJK SC'), `Chinese stack: ${JSON.stringify(typography)}`);
    assert(typography.titleFamily.includes('Space Grotesk') && typography.titleWeight === '600', `title font: ${JSON.stringify(typography)}`);
    assert(typography.detailFamily.includes('IBM Plex Mono') && typography.flowNumberFamily.includes('IBM Plex Mono'), `mono values: ${JSON.stringify(typography)}`);
    assert(typography.valueFeatures.includes('tnum'), `tabular numerals enabled: ${JSON.stringify(typography)}`);
    assert(parseFloat(typography.valueLetterSpacing) <= -0.15, `insight value tracking tightened: ${JSON.stringify(typography)}`);
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
    assert(await browser.evaluate(`(() => {
      const text = document.querySelector('.session-insights-chip-value').textContent;
      const parts = text.split('/');
      return parts.length === 2 && parts.every((part) => part.length > 0 && part === part.trim());
    })()`), "collapsed chip shows average/latest with a compact, unpadded slash");

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
    // The sidebar always starts collapsed after a reload, so verify the
    // accessible names now and the visible expanded title at the end.
    assert(await browser.evaluate(`document.querySelector('.worker-role-panel')?.getAttribute('aria-label') === '角色'`), "right sidebar accessible name must read 角色 in Chinese");
    assert(await browser.evaluate(`document.querySelector('.worker-role-restore')?.getAttribute('aria-label') === '显示角色面板'`), "collapsed right sidebar control must be localized in Chinese");
    assert(await browser.evaluate(`document.querySelector('.mobile-role-button')?.getAttribute('aria-label') === '打开角色面板'`), "mobile role entry must be localized in Chinese");
    assert(await browser.evaluate(`document.querySelector('.mailbox-header-button')?.getAttribute('title') === '跨会话信息，在 AI 提示词中提供会话ID 可进行跨会话 AI 交流合作。'`), "mailbox header hover must explain cross-session collaboration");
    await browser.evaluate(`document.querySelector('.mailbox-header-button')?.click()`);
    await waitFor(() => browser.evaluate(`!!document.querySelector('#mailbox-panel')`), "mailbox panel opens in Chinese");
    assert(await browser.evaluate(`document.querySelector('#mailbox-panel .side-panel-title strong')?.textContent === '跨会话信息'`), "mailbox panel title must read 跨会话信息");
    assert(await browser.evaluate(`[...document.querySelectorAll('#mailbox-panel .mailbox-address-label')].some((node) => node.textContent === '本会话 ID')`), "mailbox session label must read 本会话 ID");
    await browser.evaluate(`document.querySelector('#mailbox-panel .side-panel-header .icon-button')?.click()`);
    await waitFor(() => browser.evaluate(`!document.querySelector('#mailbox-panel')`), "mailbox panel closes after Chinese copy check");
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

    // Expand the sidebar last: its visible header must render 角色, not the
    // old hard-coded English title.
    await browser.evaluate(`document.querySelector('.worker-role-restore')?.click()`);
    await waitFor(() => browser.evaluate(`document.querySelector('.worker-role-panel > header > span')?.textContent?.trim() === '角色'`), "right sidebar title must read 角色 in Chinese");

    console.log("PASS Session Insights: compact chip, draggable panel, persisted order/position, pin and collapse all verified in a real browser.");
  } finally {
    await browser.close();
    await host.close();
  }
}

main().catch((error) => { console.error("FAIL Session Insights:", error); process.exitCode = 1; });
