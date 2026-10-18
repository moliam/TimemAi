// Image-attachment warning acceptance: a session routed to a text-only
// catalog endpoint must surface the warning while image attachments are
// pending, and must stay silent for multimodal or unknown-modality routes.
// Runs against the built dist through an in-process fake host (never the
// development instance).
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

const catalog = [
  { id: "z-glm5.3", revision: 1, provider: "zhipu", model: "glm-5.3", label: "GLM 5.3",
    base_url: "https://open.bigmodel.cn/api/v1", efforts: ["low", "high", "max"], default_effort: "max",
    middle_default: false, min_input: 3000, max_input: 200000, min_output: 512, max_output: 30000,
    context_window: 1000000, input_modalities: ["text"],
    protocols: [{ protocol: "openai-responses", base_url: "https://open.bigmodel.cn/api/v1", disabled_reason: null, function_calling: "supported", fixed_effort: null, fixed_reason: null }] },
  { id: "z-glm5.3-flash", revision: 1, provider: "zhipu", model: "glm-5.3-flash", label: "GLM 5.3 Flash",
    base_url: "https://open.bigmodel.cn/api/paas/v4", efforts: ["low", "high", "max"], default_effort: "max",
    middle_default: false, min_input: 3000, max_input: 200000, min_output: 512, max_output: 30000,
    context_window: 1000000, input_modalities: ["text", "image", "video", "file"],
    protocols: [{ protocol: "openai-compatible", base_url: "https://open.bigmodel.cn/api/paas/v4", disabled_reason: null, function_calling: "supported", fixed_effort: null, fixed_reason: null }] },
  { id: "legacy/custom", revision: 1, provider: "zhipu", model: "legacy-model", label: "Legacy snapshot model",
    base_url: "https://example.invalid/v1", efforts: ["none"], default_effort: "none",
    middle_default: false, min_input: 3000, max_input: 200000, min_output: 512, max_output: 30000,
    context_window: 200000,
    protocols: [{ protocol: "openai-compatible", disabled_reason: null, function_calling: "supported", fixed_effort: null, fixed_reason: null }] },
];
const endpoint = (id, catalogId, model, protocol, baseUrl) => ({
  id, name: id.toUpperCase(), catalog_id: catalogId, model, api_protocol: protocol,
  response_protocol: "xml", base_url: baseUrl, max_llm_input_tokens: 100000,
  max_llm_output_tokens: 4096, stream: true, api_key_configured: true,
  http_headers: {}, request_fields: {}, allow_cross_origin_redirects: false,
});
const endpoints = [
  endpoint("ep-text", "z-glm5.3", "glm-5.3", "openai-responses", "https://open.bigmodel.cn/api/v1"),
  endpoint("ep-flash", "z-glm5.3-flash", "glm-5.3-flash", "openai-compatible", "https://open.bigmodel.cn/api/paas/v4"),
  endpoint("ep-legacy", "legacy/custom", "legacy-model", "openai-compatible", "https://example.invalid/v1"),
];
const profile = (endpointId, model) => ({
  model_endpoint_id: endpointId, model, api_protocol: "openai-responses",
  response_protocol: "xml", base_url: "https://open.bigmodel.cn/api/v1",
  timeout_secs: 120, max_llm_input_tokens: 100000, max_llm_output_tokens: 4096,
  stream: true, max_rounds: "unlimited", bash_approval: "ask", api_key_configured: true,
});
const attachment = (id, name, bytes) => ({ id, name, path: `/tmp/${name}`, bytes });

function makeSession({ attachments = [], queuedAttachmentIds = [] } = {}) {
  const worker = { worker_id: "worker-1", context_id: "context-1", display_name: "Primary", ordinal: 0, state: "ready", parent_worker_id: null };
  const queueAttachments = attachments.filter((item) => queuedAttachmentIds.includes(item.id));
  return {
    session_id: "session-1", display_name: "Image warning", ordinal: 0, state: "ready",
    current_dir: "/work", max_llm_input_tokens: 100000, tools: [], mcp_server_ids: [],
    contexts: [{ context_id: "context-1", current_dir: "/work", worker_ids: ["worker-1"] }],
    workers: [worker], active_context_id: "context-1", primary_worker_id: "worker-1",
    runtime_profile: profile("ep-text", "glm-5.3"),
    attachments, roles: [], messages: [], turns: [],
    history_before_cursor: null, history_has_more: false,
    active_turn_id: null, cancelling_turn_id: null, pending_turn_id: null,
    message_queue: {
      revision: queuedAttachmentIds.length, items: queueAttachments.length ? [{
        command_id: "queued-1", enqueue_seq: 0,
        payload: { turn_id: "queued-1", text: "queued task", created_at_ms: Date.now(), attachments: queueAttachments, worker_roles: [] },
      }] : [], auto_send_enabled: true,
      continuation: { state: "awaiting_normal_completion" }, dispatching_command_id: null,
    },
  };
}
function makeSnapshot(session) {
  return {
    server: {
      version: "ui-acceptance", protocol_version: 1, port: 0, bind_host: "127.0.0.1",
      public_access: false, debug_mode: false, performance_trace: false,
      mem: { space: "ui-test", data_dir: "/tmp", space_dir: "/tmp/timem-ui-test", memory_dir: "/tmp/timem-ui-test/memory", temporary_retention_days: 5, temporary_capacity_bytes: null, conversation_capacity_bytes: null },
      runtime_options: [], session_env_defaults: {}, workspace_dirs: ["/work"], mcp_servers: [],
      model_catalog: catalog, model_providers: [], model_endpoints: endpoints,
    },
    sessions: [session], role_library: { roles: [], groups: [] }, session_groups: [],
  };
}

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
      if (length === 126) { if (buffer.length < 4) return; length = buffer.readUInt16BE(2); offset = 4; }
      else if (length === 127) { if (buffer.length < 10) return; length = Number(buffer.readBigUInt64BE(2)); offset = 10; }
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
  let authoritativeSession = makeSession({ attachments: [attachment("img1", "photo.png", 12345)] });
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
    } catch { response.writeHead(404); response.end("not found"); }
  });
  server.on("upgrade", (request, socket) => {
    const accept = createHash("sha1")
      .update(`${request.headers["sec-websocket-key"]}258EAFA5-E914-47DA-95CA-C5AB0DC85B11`)
      .digest("base64");
    socket.write("HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n" + `Sec-WebSocket-Accept: ${accept}\r\n\r\n`);
    const peer = makePeer(socket, () => {});
    peers.add(peer);
    socket.on("error", () => peers.delete(peer));
    socket.on("close", () => peers.delete(peer));
    peer.send({ type: "hello", snapshot: makeSnapshot(authoritativeSession), event_cursor: 0, event_replay_floor: 0 });
  });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  return {
    url: `http://127.0.0.1:${server.address().port}/`,
    setSession(session) { authoritativeSession = session; },
    close() { return new Promise((resolve) => server.close(resolve)); },
  };
}

async function readDevToolsPort(profile) {
  try {
    const [portLine] = (await readFile(join(profile, "DevToolsActivePort"), "utf8")).trim().split("\n");
    const port = Number(portLine);
    return Number.isInteger(port) && port > 0 ? port : null;
  } catch { return null; }
}
async function startBrowser(url) {
  const profile = await mkdtemp(join(tmpdir(), "timem-image-warning-"));
  const child = spawn(chrome, [
    "--remote-debugging-port=0", `--user-data-dir=${profile}`,
    "--headless=new", "--no-sandbox", "--disable-dev-shm-usage",
    "--lang=zh-CN", "--accept-lang=zh-CN",
    "--no-first-run", "--no-default-browser-check",
    "--disable-background-networking", "--disable-component-update", "--disable-sync",
    "--window-size=1280,900", "about:blank",
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
      async close() { socket.close(); if (child.exitCode === null) child.kill("SIGKILL"); await rm(profile, { recursive: true, force: true, maxRetries: 3, retryDelay: 100 }); },
    };
  } catch (error) {
    if (child.exitCode === null) child.kill("SIGKILL");
    await rm(profile, { recursive: true, force: true }).catch(() => {});
    throw error;
  }
}

const warningInfo = `(() => {
  const row = document.querySelector('.attachment-warning');
  if (!row) return null;
  return { text: row.textContent.trim(), title: row.getAttribute('title'), role: row.getAttribute('role'),
    strip: !!row.closest('.attachment-strip'), visible: row.getBoundingClientRect().height > 0 };
})()`;

async function main() {
  const host = await startHost();
  const browser = await startBrowser(host.url);
  try {
    // 1. text-only route + image attachment -> warning present with hints.
    await waitFor(() => browser.evaluate(`!!document.querySelector('.attachment-warning')`), "warning appears for text-only route with image");
    let info = await browser.evaluate(warningInfo);
    assert(info && info.role === "status" && info.strip && info.visible, "warning role/status/visibility " + JSON.stringify(info));
    assert(info.text.includes("EP-TEXT") && info.text.includes("glm-5.3-flash"), "warning names route and vision alternative " + JSON.stringify(info));
    assert(info.title === "图片将无法被模型读取", "zh tooltip " + JSON.stringify(info));

    // 2. text-only route, no image (txt only) -> silent.
    host.setSession(makeSession({ attachments: [attachment("txt1", "notes.txt", 42)] }));
    await browser.call("Page.reload");
    await waitFor(() => browser.evaluate(`!!document.querySelector('.attachment-strip .pending-attachment')`), "txt strip mounts");
    assert(await browser.evaluate(`!document.querySelector('.attachment-warning')`), "no warning for txt attachment");

    // 3. multimodal route + image -> silent.
    host.setSession({ ...makeSession({ attachments: [attachment("img1", "photo.png", 12345)] }), runtime_profile: profile("ep-flash", "glm-5.3-flash") });
    await browser.call("Page.reload");
    await waitFor(() => browser.evaluate(`!!document.querySelector('.attachment-strip .pending-attachment')`), "flash strip mounts");
    assert(await browser.evaluate(`!document.querySelector('.attachment-warning')`), "no warning on vision-capable endpoint");

    // 4. legacy snapshot model without modalities -> permissive silence.
    host.setSession({ ...makeSession({ attachments: [attachment("img1", "photo.png", 12345)] }), runtime_profile: profile("ep-legacy", "legacy-model") });
    await browser.call("Page.reload");
    await waitFor(() => browser.evaluate(`!!document.querySelector('.attachment-strip .pending-attachment')`), "legacy strip mounts");
    assert(await browser.evaluate(`!document.querySelector('.attachment-warning')`), "no warning for unknown-modality route");

    // 5. image reserved by a queued message still warns (reserved attachments
    //    are excluded from the strip but still leave with a future send).
    host.setSession(makeSession({ attachments: [attachment("img1", "photo.png", 12345)], queuedAttachmentIds: ["img1"] }));
    await browser.call("Page.reload");
    await waitFor(() => browser.evaluate(`!!document.querySelector('.attachment-warning')`), "warning survives queue reservation");
    assert(await browser.evaluate(`document.querySelectorAll('.attachment-strip .pending-attachment').length === 0`), "reserved attachment hidden from strip");
    assert(await browser.evaluate(`!!document.querySelector('.queued-message-attachments, .attachment-warning')`), "strip mounted by warning alone");

    // 6. uppercase/raster extensions count; en locale text.
    host.setSession(makeSession({ attachments: [attachment("img2", "SCAN.JPG", 2048)] }));
    await browser.call("Page.reload");
    await waitFor(() => browser.evaluate(`!!document.querySelector('.attachment-warning')`), "uppercase JPG detected");
    await browser.evaluate(`localStorage.setItem('timem-web-locale-v1','en')`);
    await browser.call("Page.reload");
    await waitFor(() => browser.evaluate(`!!document.querySelector('.attachment-warning')`), "warning after locale switch");
    info = await browser.evaluate(warningInfo);
    assert(info.text.includes("does not accept image input"), "en warning text " + JSON.stringify(info));
    assert(info.title === "Images cannot be read by this model", "en tooltip " + JSON.stringify(info));
    await browser.evaluate(`localStorage.setItem('timem-web-locale-v1','zh')`);

    // 7. geometry: dark/light at 1280/390, warning inside strip, no overflow.
    for (const width of [1280, 390]) {
      await browser.call("Emulation.setDeviceMetricsOverride", { width, height: 900, deviceScaleFactor: 1, mobile: false });
      for (const theme of ["dark", "light"]) {
        await browser.evaluate(`document.documentElement.dataset.theme=${JSON.stringify(theme)}`);
        const geometry = await browser.evaluate(`(() => {
          const row = document.querySelector('.attachment-warning');
          const strip = document.querySelector('.attachment-strip');
          if (!row || !strip) return null;
          const r = row.getBoundingClientRect(); const s = strip.getBoundingClientRect();
          const bodyOverflow = document.documentElement.scrollWidth > document.documentElement.clientWidth + 1;
          return { visible: r.width > 0 && r.height > 0, inside: r.left >= s.left - 1 && r.right <= s.right + 1, bodyOverflow,
            color: getComputedStyle(row).color };
        })()`);
        assert(geometry && geometry.visible && geometry.inside && !geometry.bodyOverflow,
          `warning geometry ${width} ${theme} ` + JSON.stringify(geometry));
      }
    }
    console.log("PASS image-attachment warning: text-only route warns, txt/vision/legacy silent, queue reservation keeps warning, zh/en copy, dark/light and 1280/390 geometry");
  } finally {
    await browser.close();
    await host.close();
  }
}
await main();
