// Timer panel UI acceptance: drives real Chrome against the built dist with
// a scripted WebSocket host. Verifies header indicator, panel rendering,
// toggle/delete/create round-trips, validation gating, and layout across
// themes and widths.
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
const timers = () => [
  {
    timer_id: "timer-u1", name: "每日提醒",
    prompt: "检查长构建结果并汇报：跑 ctest --output-on-failure，失败用例列出清单与原因；\n全绿则一句话汇报，并顺带检查磁盘占用，超过 85% 时清理 target 旧产物。",
    mode: "once", after_ms: 60000, at: null, interval_ms: null,
    created_by: "user", created_at_ms: now, enabled: true,
    next_fire_ms: now + 60000, last_fired_ms: null,
  },
];
let authoritativeTimers = timers();

const timerFiredTurn = () => ({
  turn_id: "turn-timer-fired", state: "finished", created_at_ms: now,
  user_entries: [{
    kind: "timer", text: "[timer:每日提醒] 检查长构建结果并汇报。",
    created_at_ms: now,
  }],
  events: [], final_answer: "已汇报。", completion: { stop_reason: "TurnComplete" },
});
const plainTaskTurn = () => ({
  turn_id: "turn-plain-task", state: "finished", created_at_ms: now,
  user_entries: [{ kind: "task", text: "手动输入的任务", created_at_ms: now }],
  events: [], final_answer: "完成。", completion: { stop_reason: "TurnComplete" },
});
const worker = (state) => ({
  worker_id: "worker-1", context_id: "context-1", display_name: "Primary",
  ordinal: 0, state, parent_worker_id: null,
});
const session = () => ({
  session_id: "session-1", display_name: "Timer acceptance", ordinal: 0,
  state: "ready", current_dir: "/work", max_llm_input_tokens: 100000,
  tools: [], mcp_server_ids: [],
  timers: authoritativeTimers,
  contexts: [{ context_id: "context-1", current_dir: "/work", worker_ids: ["worker-1"] }],
  workers: [worker("ready")], active_context_id: "context-1", primary_worker_id: "worker-1",
  attachments: [], roles: [], messages: [], turns: [plainTaskTurn(), timerFiredTurn()],
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
  let eventSequence = 0;
  const commands = [];
  const peers = new Set();
  const mime = {
    ".html": "text/html; charset=utf-8", ".js": "text/javascript; charset=utf-8",
    ".css": "text/css; charset=utf-8", ".woff": "font/woff", ".woff2": "font/woff2",
    ".ttf": "font/ttf", ".svg": "image/svg+xml",
  };
  const broadcast = (event) => {
    eventSequence += 1;
    for (const peer of peers) peer.send({ type: "semantic_event", event_seq: eventSequence, event });
  };
  const broadcastTimers = () => broadcast({
    type: "timers_updated", session_id: "session-1", timers: authoritativeTimers,
  });
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
      if (command.type === "timer_toggle") {
        authoritativeTimers = authoritativeTimers.map((timer) =>
          timer.timer_id === command.timer_id ? { ...timer, enabled: command.enabled } : timer,
        );
        broadcastTimers();
      } else if (command.type === "timer_delete") {
        authoritativeTimers = authoritativeTimers.filter((timer) => timer.timer_id !== command.timer_id);
        broadcastTimers();
      } else if (command.type === "timer_update") {
        authoritativeTimers = authoritativeTimers.map((timer) =>
          timer.timer_id === command.timer_id ? { ...timer, prompt: command.prompt } : timer,
        );
        broadcastTimers();
      } else if (command.type === "timer_create") {
        authoritativeTimers = [
          ...authoritativeTimers,
          {
            timer_id: `timer-created-${authoritativeTimers.length}`,
            name: command.name || `timer${authoritativeTimers.length}`,
            prompt: command.prompt, mode: command.mode || "once",
            after_ms: command.after_ms ?? null, at: command.at ?? null,
            interval_ms: command.interval_ms ?? null, created_by: "user",
            created_at_ms: Date.now(), enabled: true,
            next_fire_ms: Date.now() + (command.after_ms ?? 60000),
            last_fired_ms: null,
          },
        ];
        broadcastTimers();
      }
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
    const portLine = text.trim().split("\n")[0];
    const port = Number(portLine);
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
  const profile = await mkdtemp(join(tmpdir(), "timem-timer-ui-"));
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
    // Native confirm() dialogs must not stall the run: the driver answers
    // them from a closure flag (setDialogAccept) and records the message.
    const dialogs = [];
    let dialogAccept = true;
    socket.addEventListener("message", ({ data }) => {
      const message = JSON.parse(String(data));
      if (message.method === "Page.javascriptDialogOpening") {
        dialogs.push(message.params.message);
        socket.send(JSON.stringify({
          id: ++sequence, method: "Page.handleJavaScriptDialog",
          params: { accept: dialogAccept },
        }));
      }
    });
    const evaluate = async (expression) => {
      const result = await call("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
      if (result.exceptionDetails) throw new Error(result.exceptionDetails.text);
      return result.result.value;
    };
    return {
      call, evaluate, dialogs,
      setDialogAccept: (accept) => { dialogAccept = accept; },
      async close() { socket.close(); await stopBrowserProcess(child, profile); },
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
  const click = async (js) => browser.evaluate(js);
  const lastCommand = (type) => [...host.commands].reverse().find((c) => c.type === type);
  try {
    await waitFor(() => exists(".timer-header-button"), "timer header button missing");
    // Indicator: wait for the hello snapshot to hydrate the session, then the
    // count badge and highlight must appear for active timers.
    await waitFor(
      () => browser.evaluate(
        `document.querySelector('.timer-header-button')?.classList.contains('has-timers') === true`,
      ),
      "header button must highlight with active timers",
    );
    assert(
      await browser.evaluate(`document.querySelector('.timer-header-count')?.textContent === '1'`),
      "header count must show 1",
    );

    // Open the panel.
    await click(`document.querySelector('.timer-header-button').click()`);
    await waitFor(() => exists("#timer-panel"), "timer panel did not open");
    await waitFor(() => exists(".timer-item"), "timer item did not render");
    const itemText = await browser.evaluate(
      `document.querySelector('.timer-item').textContent`,
    );
    assert(itemText.includes("每日提醒"), "timer name missing");
    assert(itemText.includes("用户"), "user origin badge missing");
    assert(itemText.includes("单次"), "mode label missing");
    assert(itemText.includes("检查长构建结果并汇报"), "prompt missing");
    assert(itemText.includes("下次"), "next-fire line missing");

    // Toggle: pause then resume through the scripted host.
    await click(`document.querySelector('.timer-toggle').click()`);
    await waitFor(() => browser.evaluate(
      `document.querySelector('.timer-item-schedule')?.textContent?.includes('已暂停') ?? false`,
    ), "paused state not shown after toggle");
    let toggle = lastCommand("timer_toggle");
    assert(toggle && toggle.timer_id === "timer-u1" && toggle.enabled === false, "timer_toggle command missing or wrong");
    await click(`document.querySelector('.timer-toggle').click()`);
    await waitFor(() => browser.evaluate(
      `document.querySelector('.timer-item-schedule')?.textContent?.includes('下次') ?? false`,
    ), "enabled state not restored");
    toggle = lastCommand("timer_toggle");
    assert(toggle && toggle.enabled === true, "re-enable toggle missing");

    // Prompt clamp: long prompts clamp to two lines, expand restores full text.
    assert(
      await browser.evaluate(`document.querySelector('.timer-item-prompt')?.classList.contains('clamped') === true`),
      "long prompt must clamp",
    );
    const promptBox = await browser.evaluate(
      `(() => { const el = document.querySelector('.timer-item-prompt'); return { client: el.clientHeight, scroll: el.scrollHeight }; })()`,
    );
    assert(promptBox.scroll > promptBox.client, `clamped prompt must hide overflow ${JSON.stringify(promptBox)}`);
    await click(`document.querySelector('.timer-item-prompt-toggle')?.click()`);
    await waitFor(() => browser.evaluate(
      `document.querySelector('.timer-item-prompt')?.classList.contains('clamped') === false`,
    ), "expand must remove clamp");
    const expandedBox = await browser.evaluate(
      `(() => { const el = document.querySelector('.timer-item-prompt'); return { client: el.clientHeight, scroll: el.scrollHeight }; })()`,
    );
    assert(expandedBox.scroll <= expandedBox.client + 1, `expanded prompt must show all text ${JSON.stringify(expandedBox)}`);
    await click(`document.querySelector('.timer-item-prompt-toggle')?.click()`);
    await waitFor(() => browser.evaluate(
      `document.querySelector('.timer-item-prompt')?.classList.contains('clamped') === true`,
    ), "collapse must restore clamp");

    // Edit prompt: pencil opens a textarea prefilled; save sends timer_update
    // and the host echo re-renders the new prompt with schedule untouched.
    // The schedule line embeds a live relative time ("in 60 seconds"), which
    // re-renders as the wall clock advances; compare the stable structure
    // instead: the absolute-fire title (from next_fire_ms) and the mode
    // prefix. A prompt-only edit must leave both untouched.
    const scheduleBefore = await browser.evaluate(
      `(() => { const el = document.querySelector('.timer-item-schedule');
        return { title: el?.getAttribute('title') ?? null, text: el?.textContent ?? "" }; })()`,
    );
    await click(`document.querySelector('.timer-item-actions .timer-edit')?.click()`);
    await waitFor(() => browser.evaluate(`!!document.querySelector('.timer-item-edit textarea')`), "edit textarea missing");
    assert(
      (await browser.evaluate(`document.querySelector('.timer-item-edit textarea')?.value`)).includes("检查长构建结果并汇报"),
      "edit textarea must be prefilled",
    );
    // Empty draft keeps save disabled.
    await browser.evaluate(
      `(() => { const el = document.querySelector('.timer-item-edit textarea'); el.focus();
        Object.getOwnPropertyDescriptor(el.constructor.prototype, 'value').set.call(el, '   ');
        el.dispatchEvent(new Event('input', { bubbles: true })); })()`,
    );
    assert(
      await browser.evaluate(`document.querySelector('.timer-edit-save')?.disabled === true`),
      "save must stay disabled for blank prompt",
    );
    await browser.evaluate(
      `(() => { const el = document.querySelector('.timer-item-edit textarea'); el.focus();
        Object.getOwnPropertyDescriptor(el.constructor.prototype, 'value').set.call(el, '编辑后的提示：每晚 22 点汇总当日提交并输出日报。');
        el.dispatchEvent(new Event('input', { bubbles: true })); })()`,
    );
    await click(`document.querySelector('.timer-edit-save')?.click()`);
    await waitFor(() => browser.evaluate(
      `document.querySelector('.timer-item-prompt')?.textContent.includes('编辑后的提示') ?? false`,
    ), "host echo must re-render the edited prompt");
    const update = lastCommand("timer_update");
    assert(update && update.timer_id === "timer-u1" && update.prompt.includes("编辑后的提示"), `timer_update command missing or wrong: ${JSON.stringify(update)}`);
    const scheduleAfter = await browser.evaluate(
      `(() => { const el = document.querySelector('.timer-item-schedule');
        return { title: el?.getAttribute('title') ?? null, text: el?.textContent ?? "" }; })()`,
    );
    assert(
      scheduleAfter.title === scheduleBefore.title && scheduleAfter.title !== null,
      `prompt edit must not touch the fire time: ${JSON.stringify({ before: scheduleBefore, after: scheduleAfter })}`,
    );
    assert(
      scheduleAfter.text.startsWith("单次"),
      `prompt edit must not change the mode prefix: ${JSON.stringify({ before: scheduleBefore.text, after: scheduleAfter.text })}`,
    );
    // Cancel path: pencil then cancel leaves the prompt unchanged.
    await click(`document.querySelector('.timer-item-actions .timer-edit')?.click()`);
    await browser.evaluate(
      `(() => { const el = document.querySelector('.timer-item-edit textarea'); el.focus();
        Object.getOwnPropertyDescriptor(el.constructor.prototype, 'value').set.call(el, '不应保存的内容');
        el.dispatchEvent(new Event('input', { bubbles: true })); })()`,
    );
    await click(`document.querySelector('.timer-edit-cancel')?.click()`);
    await waitFor(() => browser.evaluate(
      `document.querySelector('.timer-item-prompt')?.textContent.includes('编辑后的提示') ?? false`,
    ), "cancel must discard the draft");

    // Create area stays collapsed behind the "new timer" toggle until clicked.
    assert(
      await browser.evaluate(`!document.querySelector('.timer-create-form')`),
      "create form must stay collapsed until the toggle is clicked",
    );
    assert(
      await browser.evaluate(`document.querySelector('.timer-create-toggle')?.getAttribute('aria-expanded') === 'false'`),
      "toggle must advertise collapsed state",
    );
    await click(`document.querySelector('.timer-create-toggle').click()`);
    await waitFor(() => exists(".timer-create-form"), "create form must expand after toggle");
    assert(
      await browser.evaluate(`document.querySelector('.timer-create-toggle')?.getAttribute('aria-expanded') === 'true'`),
      "toggle must advertise expanded state",
    );
    assert(
      await browser.evaluate(`document.activeElement?.tagName === 'TEXTAREA' && document.activeElement?.closest('.timer-create-form') !== null`),
      "expanding must focus the work assignment field",
    );
    assert(
      (await browser.evaluate(`document.querySelector('.timer-create-form label')?.textContent`)).includes("工作内容"),
      "work assignment label missing",
    );

    // Create form validation: prompt required, delay required.
    assert(
      await browser.evaluate(`document.querySelector('.timer-create-submit')?.disabled === true`),
      "create must be disabled with empty form",
    );
    const setNativeValue = (selector, value) => browser.evaluate(
      `(() => { const el = document.querySelector(${JSON.stringify(selector)}); el.focus();
        Object.getOwnPropertyDescriptor(el.constructor.prototype, 'value').set.call(el, ${JSON.stringify(value)});
        el.dispatchEvent(new Event('input', { bubbles: true })); })()`,
    );
    await setNativeValue(".timer-create-form textarea", "定时汇报构建进度");
    await browser.evaluate(
      `[...document.querySelectorAll('.timer-create-form input[type=number]')][0]?.focus?.()`,
    );
    // Still disabled: no delay provided.
    assert(
      await browser.evaluate(`document.querySelector('.timer-create-submit')?.disabled === true`),
      "create must stay disabled without a delay",
    );
    assert(
      await browser.evaluate(`Boolean(document.querySelector('.timer-form-hint.error'))`),
      "invalid-input hint must appear",
    );
    await setNativeValue(".timer-create-form input[type=number]", "5");
    await waitFor(() => browser.evaluate(
      `document.querySelector('.timer-create-submit')?.disabled === false`,
    ), "create still disabled with valid input");
    await click(`document.querySelector('.timer-create-submit').click()`);
    await waitFor(() => lastCommand("timer_create") !== null, "timer_create command not sent");
    const created = lastCommand("timer_create");
    assert(created.prompt === "定时汇报构建进度", "created prompt wrong");
    assert(created.after_ms === 300000, "after_ms must be minutes converted to ms");
    assert(created.mode === "once", "mode default must be once");
    // Panel shows the new timer (count badge updates).
    await waitFor(() => browser.evaluate(
      `document.querySelector('.timer-header-count')?.textContent === '2'`,
    ), "header count must show 2 after create");

    // Successful create collapses the form back to the toggle.
    await waitFor(() => exists(".timer-create-toggle"), "toggle must return after create");
    assert(
      await browser.evaluate(`!document.querySelector('.timer-create-form')`),
      "form must collapse after a successful create",
    );
    // Collapse button discards drafts and closes the form.
    await click(`document.querySelector('.timer-create-toggle').click()`);
    await waitFor(() => exists(".timer-create-form"), "form must reopen");
    await setNativeValue(".timer-create-form textarea", "草稿会被丢弃");
    await click(`document.querySelector('.timer-create-collapse').click()`);
    await waitFor(async () => !(await exists(".timer-create-form")), "collapse must close the form");
    assert(
      await browser.evaluate(`document.activeElement?.classList.contains('timer-create-toggle') === true`),
      "collapse must return focus to the toggle",
    );
    await click(`document.querySelector('.timer-create-toggle').click()`);
    await waitFor(() => exists(".timer-create-form"), "form must reopen for repeat mode");
    assert(
      await browser.evaluate(`document.querySelector('.timer-create-form textarea')?.value === ''`),
      "reopened form must start empty",
    );

    // Repeat mode reveals the interval field and sends interval_ms.
    await browser.evaluate(
      `document.querySelector('.timer-create-form select').value = 'repeat';
       document.querySelector('.timer-create-form select').dispatchEvent(new Event('change', { bubbles: true }));`,
    );
    await waitFor(() => browser.evaluate(
      `document.querySelectorAll('.timer-create-form input[type=number]').length === 2`,
    ), "repeat mode must reveal interval field");
    await setNativeValue(".timer-create-form textarea", "循环巡检");
    const numberInputs = () => browser.evaluate(
      `[...document.querySelectorAll('.timer-create-form input[type=number]')].map((el) => el.value)`,
    );
    await setNativeValue(".timer-create-form input[type=number]", "2");
    // Second number input is the interval: fill through DOM order.
    await browser.evaluate(
      `(() => { const inputs = [...document.querySelectorAll('.timer-create-form input[type=number]')];
        const el = inputs[inputs.length - 1]; el.focus();
        Object.getOwnPropertyDescriptor(el.constructor.prototype, 'value').set.call(el, '15');
        el.dispatchEvent(new Event('input', { bubbles: true })); })()`,
    );
    await waitFor(() => browser.evaluate(
      `document.querySelector('.timer-create-submit')?.disabled === false`,
    ), "repeat form should be submittable");
    await click(`document.querySelector('.timer-create-submit').click()`);
    await waitFor(() => (lastCommand("timer_create")?.prompt === "循环巡检"), "repeat create not sent");
    const repeatCreated = lastCommand("timer_create");
    assert(repeatCreated.mode === "repeat" && repeatCreated.interval_ms === 900000,
      `repeat interval_ms wrong: ${JSON.stringify(repeatCreated)}`);

    // Delete: wait for the repeat timer to render before sampling, so the
    // before/after comparison cannot race the timers_updated round-trip.
    const contains = (selector, text) => browser.evaluate(
      `[...document.querySelectorAll(${JSON.stringify(selector)})].some((node) => node.textContent?.includes(${JSON.stringify(text)}) === true)`,
    );
    await waitFor(() => contains(".timer-item", "循环巡检"), "repeat timer item did not render");
    const beforeDelete = await browser.evaluate(`document.querySelectorAll('.timer-item').length`);
    // Dismissing the confirm must not delete nor send any command.
    browser.setDialogAccept(false);
    const dismissedDialogCount = browser.dialogs.length;
    await click(`document.querySelector('.timer-delete').click()`);
    await sleep(300);
    assert(
      await browser.evaluate(`document.querySelectorAll('.timer-item').length === ${beforeDelete}`),
      "dismissed confirm must keep the timer",
    );
    assert(lastCommand("timer_delete") == null, "dismissed confirm must not send timer_delete");
    const dismissedMessage = browser.dialogs[dismissedDialogCount] ?? "";
    assert(dismissedMessage.includes("每日提醒"), `confirm dialog must name the timer: ${JSON.stringify(dismissedMessage)}`);
    // Accepting the confirm deletes the same timer.
    const dialogCountBefore = browser.dialogs.length;
    browser.setDialogAccept(true);
    await click(`document.querySelector('.timer-delete').click()`);
    await waitFor(() => browser.evaluate(
      `document.querySelectorAll('.timer-item').length === ${beforeDelete - 1}`,
    ), "timer item not removed after confirmed delete");
    assert(lastCommand("timer_delete") !== null, "timer_delete command not sent after confirm");
    const dialogMessage = browser.dialogs[dialogCountBefore] ?? "";
    assert(dialogMessage.includes("每日提醒"), `confirm dialog must name the timer: ${JSON.stringify(dialogMessage)}`);

    await waitFor(async () => !(await exists(".timer-create-form")), "repeat create must collapse the form");

    // Timer-fired entry presents tag + alarm badge + timer name, and the
    // body shows the bare prompt without the raw "[timer:name]" prefix.
    const timerEntryText = await browser.evaluate(
      `document.querySelector('.turn-user-entry.timer')?.innerText`,
    );
    assert(timerEntryText?.includes("每日提醒"), `timer name must render: ${JSON.stringify(timerEntryText)}`);
    assert(timerEntryText?.includes("检查长构建结果并汇报"), `timer prompt must render: ${JSON.stringify(timerEntryText)}`);
    assert(!timerEntryText?.includes("[timer:"), `timer entry must strip the raw prefix: ${JSON.stringify(timerEntryText)}`);
    assert(await exists(".turn-user-entry.timer .timer-entry-badge"), "timer alarm badge must render");
    assert(await exists(".turn-user-entry.timer .timer-entry-name"), "timer name element must render");

    // Timer-fired turn renders with the blue tint distinct from the plain
    // user bubble; assert per theme against the computed style.
    for (const theme of ["dark", "light"]) {
      await browser.evaluate(`document.documentElement.dataset.theme = ${JSON.stringify(theme)}`);
      await browser.evaluate(`new Promise((resolveFrame) => requestAnimationFrame(() => requestAnimationFrame(resolveFrame)))`);
      const timerBg = await browser.evaluate(
        `getComputedStyle(document.querySelector('.turn-user-entry.timer')).backgroundColor`,
      );
      const plainBg = await browser.evaluate(
        `getComputedStyle(document.querySelector('.turn-user-entry.task')).backgroundColor`,
      );
      const bubbleBg = await browser.evaluate(
        `getComputedStyle(document.querySelector('.turn-user-content')).backgroundColor`,
      );
      assert(timerBg && timerBg !== "rgba(0, 0, 0, 0)" && timerBg !== plainBg,
        `timer entry must tint: theme=${theme} timer=${timerBg} plain=${plainBg}`);
      assert(timerBg.startsWith("rgba(59, 130, 246,"),
        `timer tint must use the existing accent blue: theme=${theme} got=${timerBg}`);
      assert(bubbleBg !== timerBg, "bubble and timer tint must differ");
    }

    // Layout acceptance across themes and widths (panel + create form).
    for (const theme of ["dark", "light"]) {
      for (const width of [1440, 390]) {
        await browser.call("Emulation.setDeviceMetricsOverride", { width, height: 1000, deviceScaleFactor: 1, mobile: false });
        await browser.evaluate(`document.documentElement.dataset.theme = ${JSON.stringify(theme)}`);
        await browser.evaluate(`new Promise((resolveFrame) => requestAnimationFrame(() => requestAnimationFrame(resolveFrame)))`);
        const overflow = await browser.evaluate(
          `(() => { const panel = document.querySelector('#timer-panel'); if (!panel) return -1;
             return [...panel.querySelectorAll('input, select, textarea, button')].filter((el) => {
               const rect = el.getBoundingClientRect();
               return rect.width > 0 && (rect.left < -1 || rect.right > innerWidth + 1);
             }).length; })()`,
        );
        assert(overflow === 0, `timer panel controls overflow at ${theme}/${width}`);
        console.log(`timer panel acceptance: ${theme} ${width}px passed`);
      }
    }

    // Escape closes the panel and restores focus to the header button.
    await browser.call("Input.dispatchKeyEvent", { type: "keyDown", key: "Escape", code: "Escape", windowsVirtualKeyCode: 27 });
    await waitFor(
      async () => !(await exists("#timer-panel")),
      "Escape must close the timer panel",
    );
    assert(
      await browser.evaluate(
        `document.activeElement?.classList.contains('timer-header-button') === true`,
      ),
      "Escape must restore focus to the timer header button",
    );
    console.log("timer panel acceptance: all checks passed");
  } finally {
    await browser.close();
    host.close();
  }
}

main().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
