import { createHash } from "node:crypto";
import { createServer } from "node:http";
import { existsSync } from "node:fs";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { spawn } from "node:child_process";
import { tmpdir } from "node:os";
import { extname, join, resolve } from "node:path";

const root = process.cwd();
const dist = resolve(root, "interfaces/web/dist");
const output = resolve(root, "docs/assets/timem-web.png");
const chrome = [
  process.env.CHROME_BIN,
  "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
  "/usr/bin/google-chrome", "/usr/bin/google-chrome-stable",
  "/usr/bin/chromium", "/usr/bin/chromium-browser",
].filter(Boolean).find(existsSync);
if (!chrome) throw new Error("Chrome/Chromium is required");

const createdAt = Date.UTC(2026, 7, 31, 9, 30, 0);
const roles = [
  { id: "role-architect", name: "架构师", description: "梳理系统边界、核心概念与长期演进方向。" },
  { id: "role-developer", name: "开发者", description: "把设计落实为最小、完整、可测试的实现。" },
  { id: "role-reviewer", name: "审阅者", description: "核查正确性、风险、测试覆盖与交付质量。" },
];
const answer = `# Timem 是什么？

Timem 是一个 **local-first AI agent**：它把对话、记忆、工具与工作现场保存在本地 MEM 中，并通过 Web 与 Shell 提供一致的执行体验。

## 一、核心定位

### 1. 本地优先

数据默认保存在本地 MEM。Session、历史、模型配置和工具状态都由用户掌控，工作现场可以持续积累而不依赖浏览器页面。

### 2. 持续工作的智能体

Timem 不只回答单轮问题。复杂任务可以暂停、恢复并继续推进，运行时重启后也能根据本地历史找回现场。

## 二、工作如何组织

### 3. Session、Context 与 Turn

- **Session**：隔离不同主题与工作现场
- **Context**：承载当前目录和执行上下文
- **Turn**：记录一次完整任务及其结果

### 4. 多角色协作

你可以为消息组合右侧的“架构师”“开发者”和“审阅者”。Role 提供稳定的职责与方法，而每个 Session 仍保有自己的对话历史。

> 同一个 Timem，可以针对不同任务采用不同工作视角。

## 三、可靠执行

### 5. 统一 Core，多种界面

Web 和 Shell 共享同一套 Agent、Session 与 Turn 语义。界面负责交互和呈现，Core 负责权威状态、执行与持久化。

### 6. 工具与验证闭环

Timem 可以读取文件、运行命令和调用扩展工具，并根据真实结果继续推理。复杂工作遵循“计划 → 执行 → 验证”的闭环。

| 能力 | 说明 |
|---|---|
| 本地记忆 | 保存 Session、历史与配置 |
| 工具执行 | 读取文件、运行命令并验证结果 |
| Markdown | 渲染分级章节、表格、引用与代码 |

## 四、长期价值

### 7. 适合哪些工作

Timem 适合软件开发、资料研究、结构化写作、本地自动化、运维排查与长期项目协作。

### 8. 从一个问题开始

你可以从一个简单问题开始，再逐步扩展为有上下文、有工具、有验证步骤的完整工作流。

\`\`\`bash
timem                 # 打开 Web UI
timem --shell         # 使用终端界面
\`\`\``;

function session(id, name, groupId, ordinal, cwd, withIntroduction = false) {
  const turns = withIntroduction ? [{
    turn_id: "turn-intro", state: "finished", created_at_ms: createdAt,
    user_entries: [{
      command_id: "demo-question", kind: "task",
      text: "请介绍一下 Timem 是什么，并说明它如何支持 Session、Role 和长期任务。",
      worker_roles: roles, created_at_ms: createdAt,
    }],
    events: [], sub_answers: [], final_answer: answer,
    completion: {
      stats: { llm_calls: 1, tool_calls: 0, prompt_tokens: 60000, completion_tokens: 726, total_tokens: 60726, cached_tokens: 57000 },
      latest_usage: { prompt_tokens: 60000, completion_tokens: 726, total_tokens: 60726, cached_tokens: 57000 },
      elapsed_ms: 8420, stop_reason: "completed",
    },
  }] : [];
  return {
    session_id: id, display_name: name, group_id: groupId, ordinal,
    state: "ready", current_dir: cwd, max_llm_input_tokens: 200000,
    tools: [], mcp_server_ids: [],
    runtime_profile: {
      model: "gpt-5.6", api_protocol: "openai-compatible", response_protocol: "xml",
      base_url: "http://127.0.0.1:8080/v1", timeout_secs: 120,
      max_llm_input_tokens: 200000, max_llm_output_tokens: 10000,
      stream: true, max_rounds: "unlimited", bash_approval: "ask",
      work_instructions: "silent", api_key_configured: true,
    },
    contexts: [{ context_id: `context-${id}`, current_dir: cwd, worker_ids: [`worker-${id}`] }],
    workers: [{ worker_id: `worker-${id}`, context_id: `context-${id}`, display_name: name, ordinal: 0, state: "ready", parent_worker_id: null }],
    active_context_id: `context-${id}`, primary_worker_id: `worker-${id}`,
    attachments: [], roles, messages: [], turns,
    history_before_cursor: null, history_has_more: false,
    active_turn_id: null, cancelling_turn_id: null, pending_turn_id: null,
    message_queue: { revision: 0, items: [], auto_send_enabled: true, continuation: { state: "granted" }, dispatching_command_id: null },
  };
}

const snapshot = {
  server: {
    version: "2.0.0-demo", protocol_version: 1, port: 0, bind_host: "127.0.0.1",
    public_access: false, debug_mode: false, performance_trace: false,
    mem: {
      space: "demo", data_dir: "/workspace/.timem", space_dir: "/workspace/.timem/demo",
      memory_dir: "/workspace/.timem/demo/memory", temporary_retention_days: 5,
      temporary_capacity_bytes: null, conversation_capacity_bytes: null,
      claude_codex_tool_discovery: true,
    },
    runtime_options: [], session_env_defaults: {}, workspace_dirs: ["/workspace/timem"], mcp_servers: [],
    model_endpoints: [{
      id: "local-model", name: "Local model", model: "gpt-5.6",
      api_protocol: "openai-compatible", response_protocol: "xml",
      base_url: "http://127.0.0.1:8080/v1", max_llm_input_tokens: 200000,
      max_llm_output_tokens: 10000, stream: true, api_key_configured: true,
      http_headers: {}, request_fields: {}, allow_cross_origin_redirects: false,
      private_ca_configured: false,
    }],
  },
  sessions: [
    session("session-intro", "认识 Timem", "group-learning", 0, "/workspace/timem", true),
    session("session-architecture", "架构设计", "group-building", 0, "/workspace/timem/core"),
    session("session-web", "Web UI 开发", "group-building", 1, "/workspace/timem/interfaces/web"),
    session("session-release", "发布检查", "group-delivery", 0, "/workspace/timem"),
  ],
  role_library: { roles, groups: [{ id: "role-group-product", name: "产品研发", role_ids: roles.map(({ id }) => id) }] },
  session_groups: [
    { id: "group-learning", name: "探索" },
    { id: "group-building", name: "开发" },
    { id: "group-delivery", name: "交付" },
  ],
};

function frame(value) {
  const payload = Buffer.from(JSON.stringify(value));
  if (payload.length < 126) return Buffer.concat([Buffer.from([0x81, payload.length]), payload]);
  if (payload.length <= 0xffff) {
    const header = Buffer.alloc(4); header[0] = 0x81; header[1] = 126; header.writeUInt16BE(payload.length, 2);
    return Buffer.concat([header, payload]);
  }
  const header = Buffer.alloc(10); header[0] = 0x81; header[1] = 127; header.writeBigUInt64BE(BigInt(payload.length), 2);
  return Buffer.concat([header, payload]);
}

async function host() {
  const mime = { ".html": "text/html; charset=utf-8", ".js": "text/javascript; charset=utf-8", ".css": "text/css; charset=utf-8", ".png": "image/png", ".woff": "font/woff", ".woff2": "font/woff2", ".ttf": "font/ttf" };
  const upgradedSockets = new Set();
  const server = createServer(async (request, response) => {
    try {
      const pathname = new URL(request.url, "http://localhost").pathname;
      const file = resolve(dist, pathname === "/" ? "index.html" : pathname.slice(1));
      if (!file.startsWith(dist)) throw new Error("unsafe path");
      response.writeHead(200, { "content-type": mime[extname(file)] ?? "application/octet-stream" });
      response.end(await readFile(file));
    } catch { response.writeHead(404); response.end("not found"); }
  });
  server.on("upgrade", (request, socket) => {
    upgradedSockets.add(socket);
    socket.on("close", () => upgradedSockets.delete(socket));
    const accept = createHash("sha1").update(`${request.headers["sec-websocket-key"]}258EAFA5-E914-47DA-95CA-C5AB0DC85B11`).digest("base64");
    socket.write(`HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: ${accept}\r\n\r\n`);
    socket.write(frame({ type: "hello", snapshot, event_cursor: 0, event_replay_floor: 0 }));
  });
  await new Promise((done) => server.listen(0, "127.0.0.1", done));
  return {
    url: `http://127.0.0.1:${server.address().port}/`,
    close: () => new Promise((done) => {
      for (const socket of upgradedSockets) socket.destroy();
      server.close(done);
    }),
  };
}

const pause = (milliseconds) => new Promise((done) => setTimeout(done, milliseconds));
async function until(check, error, timeout = 12000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    try { if (await check()) return; } catch {}
    await pause(40);
  }
  throw new Error(error);
}
async function debuggingPort(profile) {
  try { return Number((await readFile(join(profile, "DevToolsActivePort"), "utf8")).trim().split("\n")[0]); }
  catch { return 0; }
}

const webHost = await host();
const profile = await mkdtemp(join(tmpdir(), "timem-readme-demo-"));
const browser = spawn(chrome, [
  "--remote-debugging-port=0", `--user-data-dir=${profile}`, "--headless=new", "--no-sandbox",
  "--disable-dev-shm-usage", "--no-first-run", "--no-default-browser-check",
  "--disable-background-networking", "--disable-component-update", "--disable-sync",
  "--force-device-scale-factor=2", "--window-size=1680,1000", "about:blank",
], { stdio: ["ignore", "ignore", "pipe"] });
let socket;
try {
  let port = 0;
  await until(async () => {
    port = await debuggingPort(profile);
    if (!port) return false;
    try { return (await fetch(`http://127.0.0.1:${port}/json/version`)).ok; } catch { return false; }
  }, "Chrome DevTools did not start");
  const target = await (await fetch(`http://127.0.0.1:${port}/json/new?${encodeURIComponent(webHost.url)}`, { method: "PUT" })).json();
  socket = new WebSocket(target.webSocketDebuggerUrl);
  await new Promise((done, reject) => { socket.addEventListener("open", done, { once: true }); socket.addEventListener("error", reject, { once: true }); });
  let id = 0;
  const pending = new Map();
  socket.addEventListener("message", ({ data }) => {
    const message = JSON.parse(String(data));
    const request = pending.get(message.id);
    if (!request) return;
    pending.delete(message.id);
    message.error ? request.reject(new Error(message.error.message)) : request.resolve(message.result);
  });
  const call = (method, params = {}) => new Promise((done, reject) => {
    const requestId = ++id; pending.set(requestId, { resolve: done, reject });
    socket.send(JSON.stringify({ id: requestId, method, params }));
  });
  const evaluate = async (expression) => {
    const result = await call("Runtime.evaluate", { expression, returnByValue: true, awaitPromise: true });
    if (result.exceptionDetails) throw new Error(result.exceptionDetails.text);
    return result.result.value;
  };
  await call("Runtime.enable"); await call("Page.enable");
  await call("Emulation.setDeviceMetricsOverride", {
    width: 1680, height: 1000, deviceScaleFactor: 2, mobile: false,
  });
  await until(() => evaluate(`document.querySelectorAll('.session').length >= 4 && document.querySelectorAll('.worker-role-item').length >= 3 && Boolean(document.querySelector('.turn-final-delivery')) && Boolean(document.querySelector('.final-answer-outline'))`), "Demo content did not render");
  await evaluate(`(async () => {
    await document.fonts.ready;
    const viewport = document.querySelector('.chat-scroll.aui-thread-viewport');
    if (!viewport) throw new Error('active chat viewport missing');
    const appShell = document.querySelector('.app-shell');
    if (!appShell) throw new Error('app shell missing');
    appShell.style.setProperty('--left-sidebar-width', '200px');
    appShell.style.setProperty('--right-sidebar-width', '200px');
    viewport.scrollTop = 0;
    const outlineToggle = document.querySelector('.final-answer-outline-toggle');
    if (outlineToggle) outlineToggle.click();
    await new Promise(requestAnimationFrame);
    await new Promise(requestAnimationFrame);
    const sidebar = document.querySelector('#session-navigation');
    const roleSidebar = document.querySelector('#worker-role-panel');
    const sidebarWidth = sidebar?.getBoundingClientRect().width ?? 0;
    const roleSidebarWidth = roleSidebar?.getBoundingClientRect().width ?? 0;
    if (Math.abs(sidebarWidth - 200) > 0.5 || Math.abs(roleSidebarWidth - 200) > 0.5) {
      throw new Error('unexpected symmetric sidebar widths: left=' + sidebarWidth + ' right=' + roleSidebarWidth);
    }
    const clippedSessionNames = [...document.querySelectorAll('.session-name')].filter((name) => name.scrollWidth > name.clientWidth + 0.5);
    if (clippedSessionNames.length > 0) {
      throw new Error('demo session names are clipped: ' + clippedSessionNames.map((name) => name.textContent).join(', '));
    }
    const clippedRoleNames = [...document.querySelectorAll('.worker-role-item strong')].filter((name) => name.scrollWidth > name.clientWidth + 0.5);
    if (clippedRoleNames.length > 0) {
      throw new Error('demo role names are clipped: ' + clippedRoleNames.map((name) => name.textContent).join(', '));
    }
    const question = document.querySelector('[data-session-timeline-active="true"] .turn-user-frame');
    const title = document.querySelector('[data-session-timeline-active="true"] .turn-final-delivery h1');
    const outline = document.querySelector('.final-answer-outline.docked.expanded');
    const primarySections = [...document.querySelectorAll('.final-answer-outline-card nav > button.level-2')];
    const secondarySections = [...document.querySelectorAll('.final-answer-outline-card nav > button.level-3')];
    const expectedPrimary = ['核心定位', '工作如何组织', '可靠执行', '长期价值'];
    const expectedSecondary = [
      '本地优先', '持续工作的智能体', 'Session、Context 与 Turn', '多角色协作',
      '统一 Core，多种界面', '工具与验证闭环', '适合哪些工作', '从一个问题开始',
    ];
    if (primarySections.length !== expectedPrimary.length || expectedPrimary.some((title, index) => !primarySections[index]?.textContent?.includes(title))) {
      throw new Error('chapter outline does not contain the expected four primary sections');
    }
    const normalizedSecondary = secondarySections.map((section) => {
      const text = section.textContent ?? '';
      const separator = text.indexOf('. ');
      return separator >= 0 ? text.slice(separator + 2) : text;
    });
    if (secondarySections.length !== 8) {
      throw new Error('secondary chapter count is ' + secondarySections.length + ', expected 8');
    }
    for (let index = 0; index < expectedSecondary.length; index += 1) {
      if (normalizedSecondary[index] !== expectedSecondary[index]) {
        throw new Error('secondary chapter mismatch at index ' + index + ': actual=' + JSON.stringify(normalizedSecondary[index]) + ' expected=' + JSON.stringify(expectedSecondary[index]));
      }
    }
    if (secondarySections.some((section) => getComputedStyle(section).paddingLeft === getComputedStyle(primarySections[0]).paddingLeft)) {
      throw new Error('secondary chapter outline entries are not visually nested');
    }
    const outlineCard = outline?.querySelector('.final-answer-outline-card');
    const markdownBody = document.querySelector('[data-session-timeline-active="true"] .turn-final-delivery .markdown-body');
    const visible = (node) => {
      if (!node) return false;
      const box = node.getBoundingClientRect();
      return box.bottom > 0 && box.top < innerHeight && box.right > 0 && box.left < innerWidth;
    };
    if (!visible(question) || !visible(title) || !visible(outline) || !visible(outlineCard) || !visible(markdownBody)) {
      throw new Error('demo question, title, docked chapter outline, or Markdown body is outside the viewport');
    }
    const outlineBox = outlineCard.getBoundingClientRect();
    const bodyBox = markdownBody.getBoundingClientRect();
    if (outlineBox.right > bodyBox.left) {
      throw new Error('docked chapter outline overlaps Markdown body by ' + (outlineBox.right - bodyBox.left) + 'px');
    }
    if (document.documentElement.scrollWidth > document.documentElement.clientWidth) {
      throw new Error('demo page has horizontal overflow');
    }
    const contextLabel = document.querySelector('.header-context')?.getAttribute('aria-label') ?? '';
    if (!contextLabel.includes('Context usage 30%') || !contextLabel.includes('cache: 95.0%')) {
      throw new Error('unexpected demo context label: ' + contextLabel);
    }
    const stableStyle = document.createElement('style');
    stableStyle.dataset.readmeDemo = 'stable-frame';
    stableStyle.textContent = [
      '*, *::before, *::after {',
      'animation: none !important;',
      'transition: none !important;',
      'caret-color: transparent !important;',
      'scroll-behavior: auto !important;',
      '}',
      '::-webkit-scrollbar { visibility: hidden !important; }',
    ].join('\\n');
    document.head.append(stableStyle);
    document.activeElement?.blur();
    return true;
  })()`);
  const shot = await call("Page.captureScreenshot", { format: "png", fromSurface: true, captureBeyondViewport: false });
  await writeFile(output, Buffer.from(shot.data, "base64"));
  console.log(`Wrote ${output}`);
} finally {
  socket?.close();
  if (browser.exitCode === null) browser.kill("SIGTERM");
  await Promise.race([
    new Promise((done) => browser.once("exit", done)),
    pause(2500),
  ]);
  if (browser.exitCode === null) browser.kill("SIGKILL");
  await webHost.close();
  await rm(profile, { recursive: true, force: true, maxRetries: 3, retryDelay: 100 });
}
