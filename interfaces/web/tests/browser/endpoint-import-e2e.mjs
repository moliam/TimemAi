// Real-binary endpoint import acceptance: launches a real Chrome against a
// real `timem` Web host, walks Settings -> Model Endpoints -> Import, scans a
// fixture Codex config directory, and asserts the scanned candidate and the
// imported endpoint both reach the authoritative UI. Run manually:
//   cargo build -p timem
//   ./target/debug/timem --space /tmp/timem-e2e-mem --no-open --port 23400 &
//   node tests/browser/endpoint-import-e2e.mjs
// It covers the direct-event delivery path that unit tests cannot reach.
import { spawn } from "node:child_process";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";

const URL = process.env.TIMEM_E2E_URL || "http://127.0.0.1:23400/";
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// 自建 fixture：独立于任何真实用户配置，保证可重复
const fixtureRoot = await mkdtemp(tmpdir() + "/timem-e2e-import-");
const runId = Date.now().toString(36);
const { mkdir, writeFile, rm } = await import("node:fs/promises");
await mkdir(fixtureRoot + "/codex", { recursive: true });
await mkdir(fixtureRoot + "/mem", { recursive: true });
await writeFile(fixtureRoot + "/codex/config.toml", `model = "glm-5.3"
model_provider = "zai"
model_reasoning_effort = "high"

[model_providers.zai]
name = "ZAI E2E ${runId}"
base_url = "https://open.bigmodel.cn/api/v1"
wire_api = "responses"

[model_providers.other]
name = "Other E2E ${runId}"
base_url = "https://other.example.test/v1"
experimental_bearer_token = "other-e2e-secret"
wire_api = "responses"
`);
await writeFile(fixtureRoot + "/codex/zai.config.toml", `model = "glm-5.3"
model_provider = "zai"
model_reasoning_effort = "high"
`);
await writeFile(fixtureRoot + "/codex/other.config.toml", `model = "other-model"
model_provider = "other"
model_reasoning_effort = "medium"
`);

const assert = (c, m) => { if (!c) throw new Error(m); };
async function waitFor(check, message, timeout = 15000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    try { if (await check()) return; } catch {}
    await sleep(100);
  }
  throw new Error("TIMEOUT: " + message);
}

const chrome = spawn("/usr/bin/google-chrome", [
  "--headless=new", "--no-sandbox", "--disable-gpu", "--disable-dev-shm-usage",
  "--remote-debugging-port=0", "--user-data-dir=" + await mkdtemp(tmpdir() + "/timem-e2e-chrome-"),
  URL,
], { stdio: ["ignore", "pipe", "pipe"] });
let devtoolsUrl = "";
const lineBuf = [];
await new Promise((resolve, reject) => {
  const timer = setTimeout(() => reject(new Error("chrome devtools url not found")), 10000);
  chrome.stderr.on("data", (chunk) => {
    const text = chunk.toString();
    for (const line of text.split("\n")) {
      const m = line.match(/DevTools listening on (ws:\/\/.+)/);
      if (m) { devtoolsUrl = m[1]; clearTimeout(timer); resolve(); }
    }
  });
  chrome.on("exit", () => reject(new Error("chrome exited early")));
});

const ws = new WebSocket(devtoolsUrl);
await new Promise((resolve, reject) => { ws.onopen = resolve; ws.onerror = reject; });
let msgId = 0;
const pending = new Map();
ws.onmessage = (event) => {
  const msg = JSON.parse(event.data);
  if (msg.id && pending.has(msg.id)) { pending.get(msg.id)(msg); pending.delete(msg.id); }
};
const send = (method, params = {}) => new Promise((resolve, reject) => {
  const id = ++msgId;
  pending.set(id, resolve);
  ws.send(JSON.stringify({ id, method, params }));
  setTimeout(() => reject(new Error("cdp timeout: " + method)), 15000);
});
const raw = (method, params = {}) => new Promise((resolve, reject) => {
  const id = ++msgId; pending.set(id, resolve);
  ws.send(JSON.stringify({ id, method, params }));
  setTimeout(() => reject(new Error("cdp timeout: " + method)), 15000);
});
const targets = (await raw("Target.getTargets")).result.targetInfos;
const page = targets.find((t) => t.type === "page");
const sessionId = (await raw("Target.attachToTarget", { targetId: page.targetId, flatten: true })).result.sessionId;
const call = (method, params = {}) => new Promise((resolve, reject) => {
  const id = ++msgId;
  pending.set(id, (msg) => msg.error ? reject(new Error(method + ": " + JSON.stringify(msg.error))) : resolve(msg.result));
  ws.send(JSON.stringify({ id, method, params, sessionId }));
});

const evaluate = async (expression) => {
  const r = await call("Runtime.evaluate", { expression, returnByValue: true, awaitPromise: true });
  if (r.exceptionDetails) throw new Error("eval failed: " + JSON.stringify(r.exceptionDetails));
  return r.result.value;
};

await waitFor(() => evaluate("!!document.querySelector('.endpoint-settings-toolbar') || !!document.querySelector('nav, aside, .settings')"), "app rendered");

await sleep(2500);
// 打开设置（aria-label/title 为中文）
await evaluate(`(() => {
  const btns = [...document.querySelectorAll('button')];
  const settingsBtn = btns.find(b => /打开设置|open settings/i.test(b.getAttribute('aria-label') || b.title || ''));
  if (!settingsBtn) throw new Error('settings button not found');
  settingsBtn.click(); return true;
})()`);
// 切到模型接入点分区
await waitFor(() => evaluate(`(() => {
  const btns = [...document.querySelectorAll('button')];
  const ep = btns.find(b => /模型接入点|model endpoints/i.test(b.textContent || ''));
  if (!ep) return false;
  ep.click(); return true;
})()`), "endpoints section button");
try {
  await waitFor(() => evaluate("!!document.querySelector('.endpoint-settings-toolbar')"), "endpoint settings pane");
} catch (error) {
  const buttons = await evaluate("[...document.querySelectorAll('button')].map(b => (b.getAttribute('aria-label') || b.title || b.textContent || '').trim()).filter(Boolean).join(' | ')");
  throw new Error(error.message + " BUTTONS: " + buttons);
}

// click Import
await evaluate(`(() => {
  const btn = [...document.querySelectorAll('.endpoint-settings-toolbar button')].find(b => /import|导入/i.test(b.textContent || ''));
  if (!btn) throw new Error('import button not found');
  btn.click(); return true;
})()`);
await waitFor(() => evaluate("!!document.querySelector('.endpoint-import-panel')"), "import panel open");

// fill codex dir and scan
await evaluate(`(() => {
  const labels = [...document.querySelectorAll('.endpoint-import-panel label')];
  const codex = labels.find(l => /codex/i.test(l.textContent || ''));
  const input = codex.querySelector('input');
  const setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set;
  setter.call(input, ${JSON.stringify(fixtureRoot + "/codex")});
  input.dispatchEvent(new Event('input', { bubbles: true }));
  return input.value;
})()`);
const scanned = await evaluate(`(() => {
  const btn = [...document.querySelectorAll('.endpoint-import-panel button')].find(b => /scan|扫描/i.test(b.textContent || ''));
  if (!btn || btn.disabled) throw new Error('scan button missing or disabled');
  btn.click(); return true;
})()`);
assert(scanned === true, "scan clicked");

await waitFor(() => evaluate("document.querySelectorAll('.endpoint-import-candidate').length > 0"), "import candidates appear after scan");

const candidateText = await evaluate("document.querySelector('.endpoint-import-candidate')?.textContent || ''");
assert(candidateText.includes(`ZAI E2E ${runId}`), "candidate shows provider name: " + candidateText);
assert(/glm-5\.3/.test(candidateText), "candidate shows model: " + candidateText);
assert(/high/i.test(candidateText), "candidate shows reasoning effort: " + candidateText);
const candidateCount = await evaluate("document.querySelectorAll('.endpoint-import-candidate').length");
assert(candidateCount === 2, "profile overlay providers are scanned: " + candidateCount);
const candidatesText = await evaluate("[...document.querySelectorAll('.endpoint-import-candidate')].map(n => n.textContent || '').join('\\n')");
assert(candidatesText.includes(`Other E2E ${runId}`), "profile overlay provider candidate appears: " + candidatesText);
assert(/other-model/.test(candidatesText), "overlay model is used instead of the default model: " + candidatesText);
assert(!candidatesText.includes("other-e2e-secret"), "inline provider token is not exposed");

// import them（多次运行会累积 endpoint，断言必须与既有状态无关）
const rowsBefore = await evaluate("document.querySelectorAll('.endpoint-settings-row').length");
await evaluate(`(() => {
  const btn = [...document.querySelectorAll('.endpoint-import-panel button')].find(b => /import selected|导入所选/i.test(b.textContent || ''));
  if (!btn || btn.disabled) throw new Error('import button missing or disabled');
  btn.click(); return true;
})()`);
await waitFor(() => evaluate(`(() => {
  const rows = document.querySelectorAll('.endpoint-settings-row');
  const count = ${rowsBefore} + 2;
  return rows.length === count
    && [...rows].some(row => /Other E2E/.test(row.textContent || ''))
    && [...rows].some(row => /ZAI E2E/.test(row.textContent || ''));
})()`), "imported endpoint appears in endpoint list");

// Delete both imported endpoints as one confirmed batch.
await evaluate(`(() => {
  const button = [...document.querySelectorAll('.endpoint-settings-toolbar button')].find(b => /delete|删除/i.test(b.textContent || ''));
  if (!button || button.disabled) throw new Error('delete button missing or disabled');
  button.click(); return true;
})()`);
await waitFor(() => evaluate("document.querySelectorAll('.endpoint-delete-checkbox').length > 0"), "endpoint delete checkboxes appear");
await evaluate(`(() => {
  const rows = [...document.querySelectorAll('.endpoint-settings-row')];
  const imported = rows.filter(row => {
    const text = row.textContent || '';
    return text.includes(${JSON.stringify("ZAI E2E " + runId)}) || text.includes(${JSON.stringify("Other E2E " + runId)});
  });
  if (imported.length !== 2) throw new Error('expected two imported rows, got ' + imported.length);
  for (const row of imported) row.querySelector('.endpoint-delete-checkbox').click();
  return true;
})()`);
const checkboxesAligned = await evaluate(`(() => {
  const boxes = [...document.querySelectorAll('.endpoint-delete-checkbox')];
  return boxes.length === 2 && boxes.every(box => box.getBoundingClientRect().x === boxes[0].getBoundingClientRect().x);
})()`);
assert(checkboxesAligned === true, "endpoint delete checkboxes share the same horizontal position");
await waitFor(() => evaluate(`(() => {
  const button = [...document.querySelectorAll('.endpoint-settings-toolbar button')].find(b => /delete selected|删除所选/i.test(b.textContent || ''));
  return !!button && !button.disabled && /2/.test(button.textContent || '');
})()`), "batch delete button enabled for two endpoints");
await evaluate(`(() => {
  const button = [...document.querySelectorAll('.endpoint-settings-toolbar button')].find(b => /delete selected|删除所选/i.test(b.textContent || ''));
  button.click(); return true;
})()`);
await waitFor(() => evaluate("!!document.querySelector('.endpoint-delete-backdrop')"), "endpoint delete confirmation appears");
const cancelClosed = await evaluate(`(() => {
  const button = [...document.querySelectorAll('.endpoint-delete-backdrop .decision-actions button')].find(b => /cancel|取消/i.test(b.textContent || ''));
  if (!button || button.disabled) throw new Error('cancel button unavailable');
  button.click();
  return new Promise(resolve => setTimeout(() => resolve(!document.querySelector('.endpoint-delete-backdrop')), 0));
})()`);
assert(cancelClosed === true, "endpoint delete confirmation closes on cancel");
await waitFor(() => evaluate(`(() => {
  const boxes = [...document.querySelectorAll('.endpoint-delete-checkbox')];
  return boxes.length === 2 && boxes.every(box => box.checked);
})()`), "cancel preserves delete selection");
await evaluate(`(() => {
  const button = [...document.querySelectorAll('.endpoint-settings-toolbar button')].find(b => /delete selected|删除所选/i.test(b.textContent || ''));
  if (!button || button.disabled) throw new Error('delete selected button unavailable after cancel');
  button.click(); return true;
})()`);
await waitFor(() => evaluate("!!document.querySelector('.endpoint-delete-backdrop')"), "endpoint delete confirmation reopens after cancel");
const confirmationClosedWithoutWaitingForHost = await evaluate(`(() => {
  const button = [...document.querySelectorAll('.endpoint-delete-backdrop .decision-actions button')].find(b => /delete 2 endpoints|删除 2 个接入点/i.test(b.textContent || ''));
  if (!button || button.disabled) throw new Error('confirm batch delete button unavailable');
  button.click();
  return new Promise(resolve => setTimeout(() => resolve(!document.querySelector('.endpoint-delete-backdrop')), 0));
})()`);
assert(confirmationClosedWithoutWaitingForHost === true, "endpoint delete confirmation closes immediately after confirmation");
await waitFor(() => evaluate(`(() => {
  const rows = document.querySelectorAll('.endpoint-settings-row');
  const texts = [...rows].map(row => row.textContent || '');
  return rows.length === ${rowsBefore}
    && !texts.some(text => text.includes(${JSON.stringify("ZAI E2E " + runId)}))
    && !texts.some(text => text.includes(${JSON.stringify("Other E2E " + runId)}));
})()`), "confirmed batch removes both imported endpoints");

console.log("E2E PASS: scan -> preview -> import -> batch delete");
chrome.kill();
await rm(fixtureRoot, { recursive: true, force: true });
process.exit(0);
