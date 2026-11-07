// Run only against an isolated Timem host, never the development instance.
import { spawn } from "node:child_process";
import { mkdtemp, writeFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { existsSync } from "node:fs";
const URL = process.env.TIMEM_CATALOG_TEST_URL;
if (!URL) throw new Error("TIMEM_CATALOG_TEST_URL must identify an isolated test host");
const sleep = (ms) => new Promise(r => setTimeout(r, ms));
const waitForExit = (child, timeout) => new Promise((resolve) => {
  if (!child || child.exitCode !== null || child.signalCode !== null) {
    resolve(true);
    return;
  }
  const onExit = () => {
    clearTimeout(timer);
    resolve(true);
  };
  const timer = setTimeout(() => {
    child.off("exit", onExit);
    resolve(false);
  }, timeout);
  child.once("exit", onExit);
});
const terminateChrome = async (child) => {
  if (!child || child.exitCode !== null || child.signalCode !== null) return;
  const gracefulExit = waitForExit(child, 3000);
  child.kill("SIGTERM");
  if (await gracefulExit) return;
  const forcedExit = waitForExit(child, 3000);
  child.kill("SIGKILL");
  if (!await forcedExit) throw new Error("chrome did not exit after SIGKILL");
};
const fixtureRoot = await mkdtemp(tmpdir() + "/timem-catalog-e2e-");
const runId = Date.now().toString(36);
const assert = (c, m) => { if (!c) throw new Error(m); };
async function waitFor(check, message, timeout = 15000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    try { if (await check()) return; } catch {}
    await sleep(100);
  }
  throw new Error("TIMEOUT: " + message);
}

const chromeProfile = await mkdtemp(tmpdir() + "/timem-e2e-chrome-");
let chrome;
let ws;
try {
const chromeCandidates = [
  process.env.CHROME_BIN,
  "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
  "/usr/bin/google-chrome",
  "/usr/bin/google-chrome-stable",
  "/usr/bin/chromium",
].filter(Boolean);
const chromePath = chromeCandidates.find((candidate) => existsSync(candidate));
if (!chromePath) {
  throw new Error(`Chrome/Chromium not found; checked: ${chromeCandidates.join(", ")}`);
}
chrome = spawn(chromePath, [
  "--headless=new", "--no-sandbox", "--disable-gpu", "--disable-dev-shm-usage",
  "--remote-debugging-port=0", "--user-data-dir=" + chromeProfile,
  URL,
], { stdio: ["ignore", "pipe", "pipe"] });
process.on("exit", () => chrome.kill());
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

ws = new WebSocket(devtoolsUrl);
await new Promise((resolve, reject) => {
  const timer = setTimeout(() => reject(new Error("chrome devtools websocket open timeout")), 10000);
  ws.onopen = () => { clearTimeout(timer); resolve(); };
  ws.onerror = (error) => { clearTimeout(timer); reject(error); };
});
let msgId = 0;
const pending = new Map();
ws.onmessage = (event) => {
  const msg = JSON.parse(event.data);
  if (!msg.id) return;
  const settle = pending.get(msg.id);
  if (!settle) return;
  pending.delete(msg.id);
  settle(msg);
};
const cdpRequest = (method, params = {}, sessionId, unwrapResult = false) => new Promise((resolve, reject) => {
  const id = ++msgId;
  const timer = setTimeout(() => {
    if (!pending.delete(id)) return;
    reject(new Error("cdp timeout: " + method));
  }, 15000);
  pending.set(id, (msg) => {
    clearTimeout(timer);
    if (msg.error) {
      reject(new Error(method + ": " + JSON.stringify(msg.error)));
      return;
    }
    resolve(unwrapResult ? msg.result : msg);
  });
  try {
    ws.send(JSON.stringify(sessionId === undefined
      ? { id, method, params }
      : { id, method, params, sessionId }));
  } catch (error) {
    clearTimeout(timer);
    pending.delete(id);
    reject(error);
  }
});
const raw = (method, params = {}) => cdpRequest(method, params);
const targets = (await raw("Target.getTargets")).result.targetInfos;
const page = targets.find((target) => target.type === "page" && target.url === URL)
  ?? targets.find((target) => target.type === "page");
assert(page, "chrome page target not found");
const sessionId = (await raw("Target.attachToTarget", { targetId: page.targetId, flatten: true })).result.sessionId;
const call = (method, params = {}) => cdpRequest(method, params, sessionId, true);

const evaluate = async (expression) => {
  const r = await call("Runtime.evaluate", { expression, returnByValue: true, awaitPromise: true });
  if (r.exceptionDetails) throw new Error("eval failed: " + JSON.stringify(r.exceptionDetails));
  return r.result.value;
};
await call("Page.enable");
await call("Runtime.enable");
await waitFor(
  () => evaluate(`location.href === ${JSON.stringify(URL)}`),
  "isolated catalog test page navigation",
);
const hitTest = async (selector, label) => {
  const hit = await evaluate(`(() => {
    const expected = ${selector};
    if (!expected) throw new Error('${label} not found');
    const rect = expected.getBoundingClientRect();
    const x = rect.x + rect.width / 2;
    const y = rect.y + rect.height / 2;
    const actual = document.elementFromPoint(x, y);
    return {
      x,
      y,
      disabled: expected.disabled === true,
      isHitTarget: actual === expected || expected.contains(actual),
      hitTag: actual?.tagName,
      hitClass: typeof actual?.className === 'string' ? actual.className : '',
    };
  })()`);
  assert(hit.isHitTarget === true, `${label} is not the pointer hit target: ${hit.hitTag} ${hit.hitClass}`);
  assert(hit.disabled === false, `${label} is disabled`);
  return hit;
};
const realMouseClick = async (x, y) => {
  await call("Input.dispatchMouseEvent", { type: "mouseMoved", x, y, button: "none", buttons: 0 });
  await call("Input.dispatchMouseEvent", { type: "mousePressed", x, y, button: "left", buttons: 1, clickCount: 1 });
  await call("Input.dispatchMouseEvent", { type: "mouseReleased", x, y, button: "left", buttons: 0, clickCount: 1 });
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


await evaluate(`document.querySelector('.endpoint-settings-toolbar button.primary').click()`);
await waitFor(() => evaluate("!!document.querySelector('.endpoint-catalog-picker select')"), "catalog editor");
const choose = async (id) => {await evaluate(`(() => {const e=document.querySelector('.endpoint-catalog-picker select');e.value=${JSON.stringify(id)};e.dispatchEvent(new Event('change',{bubbles:true}));})()`);await sleep(150);};
await choose('openai/gpt-6-astra');
assert(await evaluate(`(() => {const options=[...document.querySelector('.endpoint-catalog-picker select').options];return options.filter(o=>o.value).every(o=>o.textContent.includes(': ')) && options.find(o=>o.value==='openai/gpt-6-astra').textContent==='OpenAI: GPT-6 Astra' && options.find(o=>o.value==='z-glm5.3').textContent==='智谱: z-glm5.3' && options.find(o=>o.value==='kimicode_k3.1-pro').textContent==='Kimi Code: k3.1-pro' && options[0].textContent==='无';})()`),'template labels include provider and preserve none');

assert(await evaluate("document.querySelector('.endpoint-catalog-picker select').value === 'openai/gpt-6-astra'"), 'Astra selected');
console.log(await evaluate("document.querySelector('.endpoint-editor').innerText"));
console.log(await evaluate("[...document.querySelectorAll('.endpoint-editor input,.endpoint-editor select')].map(e=>({tag:e.tagName,value:e.value,min:e.min,max:e.max,disabled:e.disabled}))"));
await call('Emulation.setDeviceMetricsOverride',{width:1280,height:1100,deviceScaleFactor:1,mobile:false});
await writeFile(fixtureRoot + '/desktop.png',Buffer.from((await call('Page.captureScreenshot',{format:'png'})).data,'base64'));

const topActions = async () => {
  for(const theme of ['dark','light']) {
    await evaluate(`document.documentElement.dataset.theme=${JSON.stringify(theme)}`);
    for(const bottom of [false,true]) {
      await evaluate(`(() => {const p=document.querySelector('.settings-center-content');p.scrollTop=${bottom} ? p.scrollHeight : 0;})()`);
      await sleep(100);
      const geometry=await evaluate(`(() => {
        const bar=document.querySelector('.endpoint-editor-topbar');
        const r=bar.getBoundingClientRect();const p=document.querySelector('.settings-center-content').getBoundingClientRect();
        const buttons=[...bar.querySelectorAll('button')];
        return {visible:r.top>=p.top-1 && r.bottom<=p.bottom,inside:buttons.every(b=>{const x=b.getBoundingClientRect();return x.left>=r.left && x.right<=r.right;}),count:buttons.length,sticky:getComputedStyle(bar).position};
      })()`);
      assert(geometry.visible && geometry.inside && geometry.count===2 && geometry.sticky==='sticky','top actions '+theme+' '+bottom+': '+JSON.stringify(geometry));
      await hitTest("document.querySelector('.endpoint-editor-buttons button.secondary')",'sticky cancel');
      await hitTest("document.querySelector('.endpoint-editor-buttons button.primary')",'sticky save');
    }
  }
  await evaluate(`document.documentElement.dataset.theme='dark'`);
};
await topActions();
console.log('PASS new endpoint top actions stay visible while scrolling in both themes');

const setField = async (selector, value) => {
 await evaluate(`(() => {const e=document.querySelector(${JSON.stringify(selector)});const proto=e.tagName==='SELECT'?HTMLSelectElement.prototype:HTMLInputElement.prototype;Object.getOwnPropertyDescriptor(proto,'value').set.call(e,${JSON.stringify(value)});e.dispatchEvent(new Event(e.tagName==='SELECT'?'change':'input',{bubbles:true}));})()`); await sleep(100);
};
const protocol = '.endpoint-api-protocol select';
const effort = '.endpoint-editor-grid select:has(option[value="max"])';
assert(await evaluate(`!document.querySelector('.endpoint-api-protocol option[value="anthropic"]')`),'OpenAI does not offer Anthropic');
await choose('openai/gpt-6-sol');
await setField(protocol,'openai-compatible');
assert(await evaluate(`document.querySelector('.endpoint-editor button.primary').disabled`),'Sol conflicting daily effort blocks save');
await setField(effort,'none');
assert(await evaluate(`!document.querySelector('.endpoint-editor button.primary').disabled`),'Sol explicit none admissible');
await choose('openai/gpt-6.1-sol');
assert(await evaluate(`document.querySelector(${JSON.stringify(protocol)}).value==='openai-compatible' && document.querySelector('.endpoint-editor button.primary').disabled`),'template preserves conflicting user protocol');
// Start clean drafts to test suggestions separately from explicit user overrides.
const fresh = async () => {
 await evaluate(`document.querySelector('.endpoint-editor-heading button').click()`);
 await waitFor(()=>evaluate(`!!document.querySelector('.endpoint-settings-toolbar button.primary')`),'editor closed');
 await evaluate(`document.querySelector('.endpoint-settings-toolbar button.primary').click()`);
 await waitFor(()=>evaluate(`!!document.querySelector('.endpoint-catalog-picker select')`),'fresh draft');
};
await fresh();
const baseUrl = '.endpoint-editor-grid input[placeholder="https://api.example.com/v1"]';

// Regression for the real Kimi Code failure: the catalog option must survive
// the complete browser -> WebSocket -> Host validation -> persistence path with
// the same versioned requirements emitted by the editor.
await choose('kimicode_k3.1-pro');
assert(await evaluate(`document.querySelector('.endpoint-catalog-picker select').value==='kimicode_k3.1-pro'`),'Kimi Code k3.1-pro listed');
assert(await evaluate(`document.querySelector(${JSON.stringify(protocol)}).value==='openai-responses'`),'Kimi Code suggests Responses');
assert(await evaluate(`document.querySelector('input[placeholder="my-model-id"]').value==='k3.1-pro'`),'Kimi Code model id populated');
await setField('.endpoint-editor-grid input', 'Kimi Code E2E '+runId);
await setField('.endpoint-api-key input', 'kimi-e2e-placeholder');
assert(await evaluate(`!document.querySelector('.endpoint-editor button.primary').disabled`),'Kimi Code template is saveable after API key entry');
await evaluate(`document.querySelector('.endpoint-editor-topbar').scrollIntoView({block:'start'})`);
const kimiSave = await hitTest("document.querySelector('.endpoint-editor-buttons button.primary')",'Kimi Code save');
await realMouseClick(kimiSave.x,kimiSave.y);
await waitFor(()=>evaluate(`!document.querySelector('.endpoint-editor')`),'Kimi Code endpoint saved');
assert(await evaluate(`document.querySelector('.endpoint-settings-pane').textContent.includes('Kimi Code E2E '+${JSON.stringify(runId)})`),'Kimi Code endpoint appears in authoritative list');
await evaluate(`([...document.querySelectorAll('.endpoint-settings-edit')].find(b=>b.closest('.endpoint-settings-row')?.textContent.includes('Kimi Code E2E '+${JSON.stringify(runId)}))).click()`);
await waitFor(()=>evaluate(`!!document.querySelector('.endpoint-catalog-picker')`),'Kimi Code endpoint reopened');
assert(await evaluate(`document.querySelector('.endpoint-catalog-picker select').value==='kimicode_k3.1-pro' && document.querySelector('input[placeholder="my-model-id"]').value==='k3.1-pro' && document.querySelector(${JSON.stringify(protocol)}).value==='openai-responses'`),'Kimi Code template, model and protocol persisted');
console.log('PASS Kimi Code k3.1-pro API-key save and reopen through real Host');
await fresh();
if (process.env.TIMEM_CATALOG_TEST_PROTOCOL_TEMPLATE) {
  await choose(process.env.TIMEM_CATALOG_TEST_PROTOCOL_TEMPLATE);
  const url = () => evaluate(`document.querySelector(${JSON.stringify(baseUrl)}).value`);
  assert(await url() === 'https://responses.example.test/v1','protocol-specific initial URL');
  assert(await evaluate(`!document.querySelector('.endpoint-base-reset')`),'no restore at Responses default');
  await setField(protocol,'openai-compatible');
  assert(await url() === 'https://chat.example.test/v2','Chat changes template URL');
  assert(await evaluate(`!document.querySelector('.endpoint-base-reset')`),'no restore at Chat default');
  await setField(protocol,'openai-responses');
  assert(await url() === 'https://responses.example.test/v1','Responses changes template URL');
  await setField(baseUrl,'https://manual.example.test/v1');
  await setField(protocol,'openai-compatible');
  assert(await url() === 'https://manual.example.test/v1','manual URL retained');
  await evaluate(`document.querySelector('.endpoint-base-reset button').click()`);
  await waitFor(async()=>await url()==='https://chat.example.test/v2','reset current protocol URL');
  await setField(protocol,'openai-responses');
  assert(await url() === 'https://responses.example.test/v1','reset resumes following');
  // Save/reopen through Host to verify provenance survives persistence.
  await evaluate(`document.querySelector('.endpoint-editor button.primary').click()`);
  await waitFor(()=>evaluate(`!document.querySelector('.endpoint-editor')`),'protocol fixture saved');
  await evaluate(`document.querySelector('.endpoint-settings-edit').click()`);
  await waitFor(()=>evaluate(`!!document.querySelector('.endpoint-catalog-picker')`),'protocol fixture restored');
  await setField(protocol,'openai-compatible');
  assert(await url() === 'https://chat.example.test/v2','saved template provenance follows');
  await choose('');
  await setField(protocol,'openai-responses');
  assert(await url() === 'https://chat.example.test/v2','removed template stops following');
  await fresh();
  console.log('PASS protocol URL switch, manual override, reset, persistence, detach');
}
const compactPicker = async (maxHeight) => {
  const geometry = await evaluate(`(() => {const e=document.querySelector('.endpoint-catalog-picker'); const r=e.getBoundingClientRect(); const s=e.querySelector('select').getBoundingClientRect(); return {height:r.height,selectHeight:s.height,overflow:e.scrollWidth>e.clientWidth};})()`);
  assert(geometry.height <= maxHeight && geometry.selectHeight <= 36 && !geometry.overflow,'compact template picker: '+JSON.stringify(geometry));
};
await compactPicker(80);

const reasoningPanel = async (narrow = false) => {
  assert(await evaluate(`!document.querySelector('.endpoint-reasoning-config').open`),'reasoning configuration starts collapsed');
  for (const theme of ['dark', 'light']) {
    await evaluate(`document.documentElement.dataset.theme=${JSON.stringify(theme)}`);
    for(const open of [false,true]) {
      await evaluate(`document.querySelector('.endpoint-reasoning-config').open=${open}`);
      const geometry = await evaluate(`(() => {
        const p=document.querySelector('.endpoint-reasoning-panel');
        const r=p.getBoundingClientRect();
        const config=p.querySelector('.endpoint-reasoning-config').getBoundingClientRect();
        const daily=p.querySelector('.endpoint-reasoning-daily').getBoundingClientRect();
        const select=p.querySelector('.endpoint-reasoning-daily select').getBoundingClientRect();
        const adaptive=p.querySelector('.endpoint-reasoning-adaptive').getBoundingClientRect();
        const summary=p.querySelector('summary');const emphasis=summary.querySelector('strong');
        const chips=[...p.querySelectorAll('.endpoint-reasoning-chip span')].map(e=>e.getBoundingClientRect());
        // The committed policy row places daily and adaptive side by side on
        // wide screens and stacks them under the 600px media query; both must
        // stay below the collapsed configuration summary, in reading order.
        return {overflow:p.scrollWidth>p.clientWidth,ordered:daily.top>=config.bottom && adaptive.top>=daily.top,
          sameRow:select.top>=daily.top && select.bottom<=daily.bottom,
          contained:chips.every(c=>c.left>=r.left && c.right<=r.right),
          chipSize:chips.every(c=>c.height>=28 && c.width>=32),
          bold:Number(getComputedStyle(emphasis).fontWeight)>=700,
          emphasized:getComputedStyle(emphasis).color!==getComputedStyle(summary).color,
          checkboxWidth:p.querySelector('.endpoint-reasoning-adaptive input').getBoundingClientRect().width};
      })()`);
      assert(!geometry.overflow && geometry.ordered && geometry.sameRow && geometry.bold && geometry.emphasized && geometry.checkboxWidth===15,
        'reasoning '+theme+' narrow='+narrow+' open='+open+': '+JSON.stringify(geometry));
      if(open) { assert(geometry.contained && geometry.chipSize,'expanded chips fit'); await verifyReasoningChecks(); }
    }
  }
  await evaluate(`document.querySelector('.endpoint-reasoning-config').open=false;document.documentElement.dataset.theme='dark'`);
};
const verifyReasoningChecks = async () => {
  for(const theme of ['dark','light']) {
    await evaluate(`document.documentElement.dataset.theme=${JSON.stringify(theme)}`);
    assert(await evaluate(`([...document.querySelectorAll('.endpoint-reasoning-chip')]).every(label=>{
      const input=label.querySelector('input');const mark=label.querySelector('.endpoint-reasoning-check');
      if(!mark || (getComputedStyle(mark).visibility==='visible')!==input.checked)return false;
      const box=label.querySelector('span').getBoundingClientRect();const r=mark.getBoundingClientRect();
      const text=document.createRange();text.selectNodeContents(label.querySelector('span').firstChild);const t=text.getBoundingClientRect();
      return r.top>=box.top && r.top<box.top+6 && r.right<=box.right && r.right>box.right-6 && r.left>=t.right;
    })`),'selected check marks visible, top-right, no text overlap: '+theme);
  }
  await evaluate(`document.documentElement.dataset.theme='dark'`);
};
const reasoningInteractions = async () => {
  await evaluate(`document.querySelector('.endpoint-reasoning-config summary').scrollIntoView({block:'center'})`);
  const summaryHit=await hitTest("document.querySelector('.endpoint-reasoning-config summary')",'configure reasoning');
  await realMouseClick(summaryHit.x,summaryHit.y);
  await waitFor(()=>evaluate(`document.querySelector('.endpoint-reasoning-config').open`),'click expands levels');

  const first='.endpoint-reasoning-chip input';
  const original=await evaluate(`document.querySelector('${first}').checked`);
  await verifyReasoningChecks();
  await evaluate(`document.querySelector('.endpoint-reasoning-chip').scrollIntoView({block:'center'})`);
  const target=await hitTest("document.querySelector('.endpoint-reasoning-chip')",'reasoning chip');
  await realMouseClick(target.x,target.y);
  await waitFor(()=>evaluate(`document.querySelector('${first}').checked !== ${original}`),'chip pointer toggle');
  await verifyReasoningChecks();
  // The allow-list keeps at least one level: select a second chip first so the
  // keyboard toggle below removes a non-sole selection instead of the last one.
  const second='.endpoint-reasoning-chip:nth-of-type(2) input';
  const secondHit=await hitTest("document.querySelectorAll('.endpoint-reasoning-chip')[1]",'second reasoning chip');
  await realMouseClick(secondHit.x,secondHit.y);
  await waitFor(()=>evaluate(`document.querySelector('${second}').checked === true`),'second chip selected');
  await evaluate(`document.querySelector('${first}').focus()`);
  await call('Input.dispatchKeyEvent',{type:'keyDown',key:' ',code:'Space',windowsVirtualKeyCode:32});
  await call('Input.dispatchKeyEvent',{type:'keyUp',key:' ',code:'Space',windowsVirtualKeyCode:32});
  await waitFor(()=>evaluate(`document.querySelector('${first}').checked === ${original}`),'chip keyboard toggle');
  await verifyReasoningChecks();
  assert(await evaluate(`getComputedStyle(document.querySelector('.endpoint-reasoning-chip span')).outlineStyle==='solid'`),'chip keyboard focus visible');
  const adaptive='.endpoint-reasoning-adaptive input';
  const state=await evaluate(`document.querySelector('${adaptive}').checked`);
  for(const expected of [!state,state]) {
    await evaluate(`document.querySelector('.endpoint-reasoning-adaptive').scrollIntoView({block:'center'})`);
    const hit=await hitTest("document.querySelector('.endpoint-reasoning-adaptive')",'adaptive row');
    await realMouseClick(hit.x,hit.y);
    await waitFor(()=>evaluate(`document.querySelector('${adaptive}').checked === ${expected}`),'adaptive row toggles');
  }
  await evaluate(`document.querySelector('.endpoint-reasoning-default').click()`);
  await waitFor(()=>evaluate(`document.querySelector('.endpoint-reasoning-default').getAttribute('aria-pressed')==='true'`),'default range reset');
  await verifyReasoningChecks();
  assert(await evaluate(`!document.querySelector('.endpoint-reasoning-range-note')`),'no extra range explanation');
  await evaluate(`document.querySelector('.endpoint-reasoning-config summary').focus()`);
  await call('Input.dispatchKeyEvent',{type:'keyDown',key:' ',code:'Space',windowsVirtualKeyCode:32});
  await call('Input.dispatchKeyEvent',{type:'keyUp',key:' ',code:'Space',windowsVirtualKeyCode:32});
  await waitFor(()=>evaluate(`!document.querySelector('.endpoint-reasoning-config').open`),'keyboard collapses configuration');

};
await reasoningPanel();
await reasoningInteractions();
console.log('PASS reasoning chips pointer/keyboard/focus, adaptive row, clear selection, three rows, dark/light geometry');

await fresh();
assert(await evaluate(`!!document.querySelector('.endpoint-api-protocol option[value="anthropic"]:not(:disabled)')`),'custom service retains Anthropic choice');
await setField(protocol,'anthropic');
await choose('openai/gpt-6-astra');
assert(await evaluate(`document.querySelector('.endpoint-api-protocol select').value==='anthropic' && document.querySelector('.endpoint-api-protocol option[value="anthropic"]').disabled && document.querySelector('.endpoint-editor button.primary').disabled`),'invalid explicit protocol retained, labelled unavailable and blocks save');
await fresh();
await choose('openai/gpt-5.5-pro');
assert(await evaluate(`document.querySelector(${JSON.stringify(protocol)}).value==='openai-responses'`),'Pro suggests Responses');
await setField(baseUrl, 'https://open.bigmodel.cn/api/coding/paas/v4');
await choose('z-glm5.3');
assert(await evaluate(`document.querySelector(${JSON.stringify(baseUrl)}).value === 'https://open.bigmodel.cn/api/coding/paas/v4'`), 'template switch preserves the user-owned base URL (applyEndpointTemplate keeps user-tuned inputs)');
for (const id of ['z-glm5.2', 'z-glm5.3', 'z-glm5.3-flash']) {
  await choose(id);
  assert(await evaluate(`document.querySelector('.endpoint-catalog-picker select').value === ${JSON.stringify(id)}`), id + ' listed');
  const effortState = await evaluate(`(() => { const s = document.querySelector(${JSON.stringify(effort)}); return { n: s.options.length, marked: [...s.options].filter((o) => /\(default\)/.test(o.textContent) || o.textContent.includes('（默认）')).map((o) => o.value) }; })()`);
  assert(effortState.n === 3 && effortState.marked.length === 1, id + ' three effort levels with exactly one official default: ' + JSON.stringify(effortState));
  await evaluate(`document.querySelector('.endpoint-reasoning-config').open = true`);
  assert(await evaluate(`(() => {
    const states = Object.fromEntries([...document.querySelectorAll('.endpoint-reasoning-chip')]
      .map((label) => [label.textContent.trim(), label.querySelector('input').checked]));
    return ${JSON.stringify(id)} !== 'z-glm5.3'
      || states.low === true && states.high === true && states.max === true;
  })()`), id + ' renders every template reasoning level as selected');
  if (id === 'z-glm5.3') {
    assert(await evaluate(`document.querySelector(${JSON.stringify(effort)}).value === 'max'`), 'GLM-5.3 repairs an unsupported legacy daily level on template selection');
    assert(await evaluate(`!document.querySelector('.endpoint-validation-note')`), 'GLM-5.3 template selection is immediately valid');
    const protocolOptions = await evaluate(`[...document.querySelector('.endpoint-api-protocol select').options].map(o => o.value)`);
    assert(protocolOptions.length === 2 && protocolOptions.includes('openai-responses'), 'GLM-5.3 offers Chat and Responses');
    await setField(protocol, 'openai-responses');
    await waitFor(() => evaluate(`document.querySelector(${JSON.stringify(baseUrl)}).value === 'https://open.bigmodel.cn/api/v1'`), 'Responses switches to the official api/v1 entry');
    assert(await evaluate(`document.querySelector(${JSON.stringify(effort)}).options.length === 3`), 'GLM-5.3 keeps three effort levels under Responses');
    assert(await evaluate(`document.querySelector(${JSON.stringify(effort)}).value === 'max'`), 'GLM-5.3 daily effort stays max under Responses');
    await setField(protocol, 'openai-compatible');
    await waitFor(() => evaluate(`document.querySelector(${JSON.stringify(baseUrl)}).value === 'https://open.bigmodel.cn/api/paas/v4'`), 'switching back to Chat restores the paas/v4 entry');
    await setField(baseUrl, 'https://open.bigmodel.cn/api/paas/v4');
    await setField(protocol, 'openai-responses');
    await waitFor(() => evaluate(`document.querySelector(${JSON.stringify(baseUrl)}).value === 'https://open.bigmodel.cn/api/v1'`), 'manually entered template address follows the protocol switch');
    await setField(baseUrl, 'https://proxy.example.test/v1');
    await setField(protocol, 'openai-compatible');
    await waitFor(() => evaluate(`document.querySelector(${JSON.stringify(baseUrl)}).value === 'https://proxy.example.test/v1'`), 'custom proxy address survives the protocol switch');
    await evaluate(`document.querySelector('.endpoint-base-reset button').click()`);
    await waitFor(() => evaluate(`document.querySelector(${JSON.stringify(baseUrl)}).value === 'https://open.bigmodel.cn/api/paas/v4'`), 'restore returns the Chat template address');
    await evaluate(`(() => {
      const max = [...document.querySelectorAll('.endpoint-reasoning-chip')]
        .find((label) => label.textContent.trim() === 'max').querySelector('input');
      max.click();
      document.querySelector('.endpoint-reasoning-default').click();
    })()`);
    assert(await evaluate(`(() => {
      const states = Object.fromEntries([...document.querySelectorAll('.endpoint-reasoning-chip')]
        .map((label) => [label.textContent.trim(), label.querySelector('input').checked]));
      return states.low === true && states.high === true && states.max === true;
    })()`), 'GLM-5.3 restores low/high/max after a manual max removal');
  }
  assert(await evaluate(`document.querySelector(${JSON.stringify(protocol)}).value === 'openai-compatible'`), id + ' Chat protocol');
  if (id !== 'z-glm5.3') {
    assert(await evaluate(`document.querySelector('.endpoint-api-protocol select').options.length===1`),id+' only offers Chat');
  }
  await evaluate(`(() => { const b = document.querySelector('.endpoint-base-reset button'); if (b) b.click(); })()`);
  await waitFor(() => evaluate(`document.querySelector(${JSON.stringify(baseUrl)}).value === 'https://open.bigmodel.cn/api/paas/v4'`), id + ' provider default URL after restore');
}
assert(await evaluate(`!document.querySelector('.endpoint-base-reset')`),'Zhipu default has no restore hint');
await setField(baseUrl,'https://proxy.example.test/v1');
assert(await evaluate(`!!document.querySelector('.endpoint-base-reset button')`),'custom URL offers restore');
await setField(baseUrl,'https://open.bigmodel.cn/api/paas/v4');
assert(await evaluate(`!document.querySelector('.endpoint-base-reset')`),'manual entry of default hides restore');
await setField(baseUrl,'https://proxy.example.test/v1');
await evaluate(`document.querySelector('.endpoint-base-reset button').click()`);
await waitFor(()=>evaluate(`!document.querySelector('.endpoint-base-reset')`),'restore hides button and hint');
assert(await evaluate(`document.querySelector(${JSON.stringify(baseUrl)}).value==='https://open.bigmodel.cn/api/paas/v4'`),'restore fills current default');
await choose('openai/gpt-6-astra');
const budget = '.endpoint-token-budget';
assert(await evaluate(`[...document.querySelectorAll('.endpoint-token-budget')].every(e=>getComputedStyle(e).appearance === 'textfield')`), 'budget spinner hidden');
await setField(budget, '200000');
const budgetRect = await evaluate(`(() => {const e=document.querySelector('.endpoint-token-budget');e.scrollIntoView({block:'center'});const r=e.getBoundingClientRect();return {x:r.right-8,y:r.top+8};})()`);
await realMouseClick(budgetRect.x,budgetRect.y);
assert(await evaluate(`document.querySelector('.endpoint-token-budget').value === '200000'`), 'clicking former spinner area does not step');
await evaluate(`document.querySelector('.endpoint-token-budget').focus()`);
await call('Input.dispatchKeyEvent', {type:'keyDown',key:'ArrowUp',code:'ArrowUp',windowsVirtualKeyCode:38});
await call('Input.dispatchKeyEvent', {type:'keyUp',key:'ArrowUp',code:'ArrowUp',windowsVirtualKeyCode:38});
assert(await evaluate(`document.querySelector('.endpoint-token-budget').value === '200000'`), 'arrow does not step by one');
await call('Input.dispatchKeyEvent', {type:'keyDown',key:'a',code:'KeyA',modifiers:2,windowsVirtualKeyCode:65});
await call('Input.dispatchKeyEvent', {type:'keyUp',key:'a',code:'KeyA',modifiers:2,windowsVirtualKeyCode:65});
await call('Input.insertText', {text:'250000'});
assert(await evaluate(`document.querySelector('.endpoint-token-budget').value === '250000'`), 'budget directly editable');
await setField('.endpoint-editor input.endpoint-token-budget:not(.endpoint-context-window *)','0');
assert(await evaluate(`document.querySelector('.endpoint-editor button.primary').disabled`),'invalid budget blocked');
await setField('.endpoint-editor input.endpoint-token-budget:not(.endpoint-context-window *)','120000');
await setField('.endpoint-editor-grid input', 'Catalog UI '+runId);
await evaluate(`document.querySelector('.endpoint-editor button.primary').click()`);
await waitFor(()=>evaluate(`!document.querySelector('.endpoint-editor')`),'saved without key');
console.log(await evaluate(`document.querySelector('.endpoint-settings-list, .endpoint-settings-pane, .endpoint-settings')?.innerText || document.body.innerText`));
await evaluate(`([...document.querySelectorAll('.endpoint-settings-edit')].find(b=>b.closest('.endpoint-settings-row')?.textContent.includes('Catalog UI '+${JSON.stringify(runId)})) || [...document.querySelectorAll('.endpoint-settings-edit')].at(-1)).click()`);
await waitFor(()=>evaluate(`!!document.querySelector('.endpoint-catalog-picker')`),'edit restored');
assert(await evaluate(`document.querySelector('.endpoint-catalog-picker select').value==='openai/gpt-6-astra' && document.querySelector('.endpoint-editor input.endpoint-token-budget:not(.endpoint-context-window *)').value==='120000'`),'catalog and budget restored');
await call('Emulation.setDeviceMetricsOverride',{width:390,height:844,deviceScaleFactor:1,mobile:true});
await sleep(200);
assert(await evaluate(`document.documentElement.scrollWidth <= 390`),'no narrow viewport page overflow');
await compactPicker(110);
await reasoningPanel(true);
// Save stays disabled until the reopened draft actually differs from the
// persisted endpoint; touch the name so the sticky actions are interactive.
await setField('.endpoint-editor-grid input', 'Touched '+runId);
await topActions();
console.log("PASS edit endpoint sticky actions on narrow viewport");
console.log("PASS narrow reasoning panel in both themes");
await writeFile(fixtureRoot + '/mobile.png',Buffer.from((await call('Page.captureScreenshot',{format:'png'})).data,'base64'));
const beforeNone = await evaluate(`[...document.querySelectorAll('.endpoint-editor-grid input')].map(e=>e.value)`);
await choose('');
assert(await evaluate(`document.querySelector('.endpoint-catalog-picker select').value==='' && !document.querySelectorAll('.endpoint-editor-grid input')[1].readOnly`),'None retains editable model');
assert(JSON.stringify(beforeNone) === JSON.stringify(await evaluate(`[...document.querySelectorAll('.endpoint-editor-grid input')].map(e=>e.value)`)), 'removing template preserves values');
await setField('.endpoint-editor-grid input:nth-of-type(1)', 'Manual name');
await choose('z-glm5.3');
assert(await evaluate(`document.querySelector('.endpoint-editor-grid input').value==='Manual name'`),'user name survives template switch');
await setField('.endpoint-editor-grid input', 'UNSAVED '+runId);
await evaluate(`document.querySelector('.settings-center-content').scrollTop=0`);
const cancelHit=await hitTest("document.querySelector('.endpoint-editor-buttons button.secondary')",'cancel draft');
await realMouseClick(cancelHit.x,cancelHit.y);
await waitFor(()=>evaluate(`!document.querySelector('.endpoint-editor')`),'cancel closes draft');
assert(await evaluate(`!document.querySelector('.endpoint-settings-pane').textContent.includes('UNSAVED '+${JSON.stringify(runId)})`),'cancel does not persist draft');
// Reopen the endpoint saved by this run; the list may contain endpoints from earlier runs.
await evaluate(`([...document.querySelectorAll('.endpoint-settings-edit')].find(b=>b.closest('.endpoint-settings-row')?.textContent.includes('Catalog UI '+${JSON.stringify(runId)})) || [...document.querySelectorAll('.endpoint-settings-edit')].at(-1)).click()`);
await waitFor(()=>evaluate(`!!document.querySelector('.endpoint-editor-grid input')`),'reopen cancelled edit');
assert(await evaluate(`document.querySelector('.endpoint-editor-grid input').value==='Catalog UI '+${JSON.stringify(runId)}`),'cancel preserves saved endpoint');
console.log('PASS top cancel discards edits, persisted endpoint unchanged');
console.log('PASS catalog constraints, save, edit, custom, narrow viewport');
console.log('PASS three Zhipu templates, direct budget typing, no spinner/arrow stepping');
} finally {
  ws?.close();
  await terminateChrome(chrome);
  // Chrome may still be flushing its profile asynchronously after being
  // killed; never let cleanup errors mask the real assertion failure.
  await rm(fixtureRoot,{recursive:true,force:true}).catch(() => {});
  await rm(chromeProfile,{recursive:true,force:true}).catch(() => {});
}
process.exit(0);
