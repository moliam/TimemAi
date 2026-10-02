// Run only against an isolated Timem host, never the development instance.
import { spawn } from "node:child_process";
import { mkdtemp, writeFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
const URL = process.env.TIMEM_SHARE_TEST_URL;
if (!URL) throw new Error("TIMEM_SHARE_TEST_URL must identify an isolated test host");
const sleep = (ms) => new Promise(r => setTimeout(r, ms));
const fixtureRoot = await mkdtemp(tmpdir() + "/timem-share-e2e-");
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
chrome = spawn("/usr/bin/google-chrome", [
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
// LAN HTTP is not a secure context in Chromium. Reproduce it even though this
// acceptance host uses localhost, where randomUUID is normally available.
assert(await evaluate(`(() => {
  try { Object.defineProperty(globalThis.crypto, 'randomUUID', { configurable: true, value: undefined }); }
  catch { try { Object.defineProperty(Crypto.prototype, 'randomUUID', { configurable: true, value: undefined }); } catch {} }
  return typeof globalThis.crypto?.randomUUID === 'undefined';
})()`), 'randomUUID disabled for insecure-context regression');

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

const click = async (selector) => {
  await evaluate(`(${selector}).scrollIntoView({block:'center'})`);
  const hit = await hitTest(selector, 'share control');
  await realMouseClick(hit.x, hit.y);
  await sleep(100);
};
const setData = async data => {
  await evaluate(`(() => {const e=document.querySelector('.endpoint-share-data textarea');Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,'value').set.call(e,${JSON.stringify(data)});e.dispatchEvent(new Event('input',{bubbles:true}));})()`);
  await sleep(100);
};
// Second authenticated connection observes broadcasts and seeds one fixture.
await evaluate(`new Promise((resolve,reject)=>{
 window.shareEvents=[];window.shareSocket=new WebSocket(location.origin.replace(/^http/,'ws')+'/ws'+location.search);
 shareSocket.onmessage=e=>shareEvents.push(JSON.parse(e.data));shareSocket.onerror=reject;
 shareSocket.onopen=()=>{shareSocket.send(JSON.stringify({type:'model_endpoint_upsert',command_id:'fixture-${runId}',endpoint:{name:'mygpt',model:'custom-model',api_protocol:'openai-compatible',response_protocol:'xml',base_url:'https://example.test/v1',max_llm_input_tokens:100000,max_llm_output_tokens:10000,stream:true,api_key:'e2e-private-key',http_headers:{'X-Tenant':'e2e-private-header'},request_fields:{vendor_options:{custom:'中文'}}}}));resolve(true);};
})`);
await waitFor(()=>evaluate(`!!document.querySelector('.endpoint-share-export')`),'fixture added');
const openExport = async () => {
 await click("[...document.querySelectorAll('.endpoint-settings-row')].find(e=>e.querySelector('strong').textContent==='mygpt').querySelector('.endpoint-share-export')");
 await waitFor(()=>evaluate(`!!document.querySelector('.endpoint-share-backdrop .endpoint-share-dialog[role=dialog][aria-modal=true]')`),'export modal opened');
 assert(await evaluate(`(() => {const b=document.querySelector('.endpoint-share-backdrop');const d=document.querySelector('.endpoint-share-dialog');const settings=document.querySelector('.settings-center');return b?.parentElement===document.body && !!settings && Number(getComputedStyle(b).zIndex)>Number(getComputedStyle(document.querySelector('.settings-center-backdrop')).zIndex) && d.getBoundingClientRect().top>=0;})()`),'share is a top-level modal above settings');
};
const close = ()=>click("document.querySelector('.endpoint-share-heading button')");
const generate = async () => {
 await evaluate(`(() => {
   window.shareGeneratingCounts=[];
   window.shareGeneratingObserver?.disconnect();
   const sample=()=>window.shareGeneratingCounts.push((document.querySelector('.endpoint-share-panel')?.textContent.match(/正在生成|Generating/g)||[]).length);
   window.shareGeneratingObserver=new MutationObserver(sample);
   window.shareGeneratingObserver.observe(document.body,{subtree:true,childList:true,characterData:true,attributes:true});
   sample();
 })()`);
 const started=Date.now();
 const button=await hitTest("document.querySelector('.endpoint-share-actions .primary')",'share submit');
 await realMouseClick(button.x,button.y);
 await waitFor(()=>evaluate(`document.querySelector('.endpoint-share-dialog')?.getAttribute('aria-busy')==='true' || !!document.querySelector('.endpoint-share-data textarea')?.value`),'visible export progress or result');
 await waitFor(()=>evaluate(`!!document.querySelector('.endpoint-share-data textarea')?.value`),'export data');
 const elapsed=Date.now()-started;
 const progress=await evaluate(`(() => {window.shareGeneratingObserver?.disconnect();return {counts:window.shareGeneratingCounts,progressNodes:document.querySelectorAll('.endpoint-share-progress').length};})()`);
 assert(Math.max(0,...progress.counts)<=1,'generating label rendered more than once: '+JSON.stringify(progress.counts));
 assert(progress.progressNodes===0,'duplicate standalone progress row rendered');
 assert(elapsed<3000,'local share export should settle quickly, took '+elapsed+'ms');
 return evaluate(`document.querySelector('.endpoint-share-data textarea').value`);
};
await openExport();
assert(await evaluate(`JSON.stringify([...document.querySelectorAll('.endpoint-share-options input')].map(e=>e.checked))==='[true,false,false]'`),'safe defaults');
assert(await evaluate(`document.querySelectorAll('.endpoint-share-panel svg.lucide-triangle-alert').length===2 && !document.querySelector('.endpoint-share-panel').textContent.includes('⚠')`),'Lucide warnings not emoji');
const basic = await generate();
const decode = data=>JSON.parse(Buffer.from(data,'base64').toString('utf8'));
assert(decode(basic).basic && !decode(basic).advanced && !decode(basic).personal,'basic only default');
await click("document.querySelectorAll('.endpoint-share-options label')[1]");
assert(await evaluate(`!document.querySelector('.endpoint-share-data textarea')`),'selection clears previous export');
const advanced = await generate();
assert(decode(advanced).advanced.request_fields.vendor_options.custom==='中文' && !decode(advanced).personal,'advanced custom fields');
await click("document.querySelectorAll('.endpoint-share-options label')[2]");
const full = await generate();
assert(decode(full).personal.api_key==='e2e-private-key' && decode(full).personal.http_headers['X-Tenant']==='e2e-private-header','explicit personal export');
assert(await evaluate(`!JSON.stringify(shareEvents).includes('e2e-private-key') && !JSON.stringify(shareEvents).includes('e2e-private-header') && !JSON.stringify(shareEvents).includes('model_endpoint_share_exported')`),'no secret broadcast to second connection');
for (const width of [1280,390]) {
 await call('Emulation.setDeviceMetricsOverride',{width,height:1000,deviceScaleFactor:1,mobile:false});
 for(const theme of ['dark','light']) {
  await evaluate(`document.documentElement.dataset.theme=${JSON.stringify(theme)}`);
  assert(await evaluate(`(() => {const p=document.querySelector('.endpoint-share-dialog');const r=p.getBoundingClientRect();return r.top>=0 && r.bottom<=innerHeight+1 && p.scrollWidth<=p.clientWidth+1 && [...p.querySelectorAll('input,textarea,button')].every(e=>{const b=e.getBoundingClientRect();return b.left>=r.left && b.right<=r.right;});})()`),'share modal fits '+width+' '+theme);
 }
}
await close();
assert(await evaluate(`!document.querySelector('.endpoint-share-backdrop') && !!document.querySelector('.settings-center')`),'closing share keeps settings open');
await openExport();
assert(await evaluate(`!document.querySelector('.endpoint-share-data textarea') && JSON.stringify([...document.querySelectorAll('.endpoint-share-options input')].map(e=>e.checked))==='[true,false,false]'`),'close clears secrets and resets selections');
// All unchecked disallows export, basic-less fragment cannot create a new endpoint.
await click("document.querySelectorAll('.endpoint-share-options label')[0]");
assert(await evaluate(`document.querySelector('.endpoint-share-actions .primary').disabled`),'empty selection blocked');
await click("document.querySelectorAll('.endpoint-share-options label')[1]");
const fragment=await generate();
await close();
await click("[...document.querySelectorAll('.endpoint-settings-toolbar button')].find(b=>/导入分享|import shared endpoint/i.test(b.textContent))");
for(const invalid of ['invalid!!!',fragment]) {
 await setData(invalid);await click("document.querySelector('.endpoint-share-actions .primary')");
 await waitFor(()=>evaluate(`!!document.querySelector('.endpoint-share-panel [role=alert]')`),'invalid import shows error');
 assert(await evaluate(`document.querySelectorAll('.endpoint-settings-row').length===1`),'invalid import does not mutate');
}
for(const [data,name] of [[basic,'mygpt1'],[full,'mygpt2'],[full,'mygpt3']]) {
 await setData(data);await click("document.querySelector('.endpoint-share-actions .primary')");
 await waitFor(()=>evaluate(`document.querySelector('.endpoint-share-panel [role=status]')?.textContent.includes(${JSON.stringify(name)})`),'import success '+name);
 assert(await evaluate(`document.querySelector('.endpoint-share-data textarea').value===''`),'import clears sensitive input');
}
assert(await evaluate(`JSON.stringify([...document.querySelectorAll('.endpoint-settings-row strong')].map(e=>e.textContent))==='["mygpt","mygpt1","mygpt2","mygpt3"]'`),'collision suffixes no overwrite');
await close();
// Reload projections, then export the imported full configuration again.
await evaluate(`shareSocket.close()`);
await call('Page.reload');await sleep(2200);
await evaluate(`[...document.querySelectorAll('button')].find(b=>/打开设置|open settings/i.test(b.getAttribute('aria-label')||b.title||'')).click()`);
await waitFor(()=>evaluate(`(()=>{const b=[...document.querySelectorAll('button')].find(b=>/模型接入点|model endpoints/i.test(b.textContent||''));if(!b)return false;b.click();return true;})()`),'reopened endpoints');
await waitFor(()=>evaluate(`document.querySelectorAll('.endpoint-settings-row').length===4`),'endpoints survive reload');
await click("[...document.querySelectorAll('.endpoint-settings-row')].find(e=>e.querySelector('strong').textContent==='mygpt2').querySelector('.endpoint-share-export')");
await click("document.querySelectorAll('.endpoint-share-options label')[1]");
await click("document.querySelectorAll('.endpoint-share-options label')[2]");
const roundtrip=decode(await generate());
assert(roundtrip.name==='mygpt2' && roundtrip.personal.api_key==='e2e-private-key' && roundtrip.advanced.request_fields.vendor_options.custom==='中文','full import roundtrip after reload');
await close();
console.log('PASS endpoint sharing modal: insecure-context IDs, single progress state, fast settlement, polished responsive layout, defaults, export categories, Unicode, secret isolation, cleanup, invalid import, collisions, reload roundtrip');
} finally {
  ws?.close();
  if (chrome && chrome.exitCode === null) {
    const exited = new Promise(resolve => chrome.once('exit',resolve));
    chrome.kill();
    await exited;
  }
  await rm(fixtureRoot,{recursive:true,force:true});
  await rm(chromeProfile,{recursive:true,force:true});
}
process.exit(0);
