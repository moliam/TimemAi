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
if (!chrome) {
  throw new Error(
    `Chrome/Chromium not found; checked: ${chromeCandidates.join(", ")}`,
  );
}
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const assert = (condition, message) => { if (!condition) throw new Error(message); };
async function waitFor(check, message, timeout = 10000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    try { if (await check()) return; } catch {}
    await sleep(40);
  }
  throw new Error(message);
}

async function waitForSubtreeIdle(browser, selector, message, timeout = 10000) {
  // A fixed sleep races the CSS transition duration (the fold transition is
  // 460ms including its delay); wait briefly for the transition to register,
  // then poll until the element subtree reports no running animations so
  // geometry captures stay deterministic under CI runner load.
  await sleep(120);
  await waitFor(() => browser.evaluate(`(() => {
    const root = document.querySelector(${JSON.stringify(selector)});
    return !!root && root.getAnimations().length === 0;
  })()`), message, timeout);
}

const worker = (state) => ({
  worker_id: "worker-1", context_id: "context-1", display_name: "Primary",
  ordinal: 0, state, parent_worker_id: null,
});
const turn = (id, text = "Long task") => ({
  turn_id: id, state: "working", created_at_ms: Date.now(),
  user_entries: [{
    kind: "task", text,
    ...(id === "turn-1" ? { command_id: "submit-original" } : {}),
    created_at_ms: Date.now(),
  }],
  events: [], final_answer: null, completion: null,
});
const makeSession = (extra = {}) => ({
  session_id: "session-1", display_name: "Stop acceptance", ordinal: 0,
  state: "working", current_dir: "/work", max_llm_input_tokens: 100000,
  tools: [], mcp_server_ids: [],
  contexts: [{ context_id: "context-1", current_dir: "/work", worker_ids: ["worker-1"] }],
  workers: [worker("working")], active_context_id: "context-1", primary_worker_id: "worker-1",
  attachments: [], roles: [], messages: [], turns: [turn("turn-1")],
  history_before_cursor: null, history_has_more: false,
  active_turn_id: "turn-1", cancelling_turn_id: null, pending_turn_id: null,
  message_queue: {
    revision: 0, items: [], auto_send_enabled: true,
    continuation: { state: "awaiting_normal_completion" },
    dispatching_command_id: null,
  },
  ...extra,
});
const makeCancelledSession = (base = makeSession()) => ({
  ...base,
  state: "ready",
  workers: [worker("ready")],
  cancelling_turn_id: "turn-1",
  turns: base.turns.map((item) => item.turn_id === "turn-1"
    ? { ...item, state: "finished", completion: { stop_reason: "CancelledByUser" } }
    : item),
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
    mcp_servers: [],
    model_endpoints: [{
      id: "endpoint-1", name: "Acceptance endpoint", model: "test-model",
      api_protocol: "openai-compatible", response_protocol: "xml",
      base_url: "http://127.0.0.1/model", max_llm_input_tokens: 100000,
      max_llm_output_tokens: 4096, stream: true, api_key_configured: true,
      http_headers: {},
    }],
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
  let authoritativeSession = makeSession();
  // A real Host normally has emitted events before a browser connects. A
  // reconnect baseline must therefore work from a non-zero sequence.
  let eventSequence = 40;
  let connectionCount = 0;
  const peers = new Set();
  const commands = [];
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
    connectionCount += 1;
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
      if (command.type === "turn_history_page") {
        const records = authoritativeSession.historyRecords ?? [];
        const end = Math.min(command.offset + 16, records.length);
        peer.send({ type: "turn_history_page", session_id: command.session_id, turn_id: command.turn_id,
          offset: command.offset, records: records.slice(command.offset, end), next_offset: end < records.length ? end : null });
        return;
      }
      if (!command.command_id) return;
      peer.send({ type: "command_ack", command_id: command.command_id, status: "accepted" });
      if (command.type === "turn_submit" && authoritativeSession.state === "working") {
        authoritativeSession = {
          ...authoritativeSession,
          message_queue: {
            ...authoritativeSession.message_queue,
            revision: authoritativeSession.message_queue.revision + 1,
            items: [
              ...authoritativeSession.message_queue.items,
              {
                command_id: command.command_id,
                enqueue_seq: authoritativeSession.message_queue.items.length,
                payload: {
                  turn_id: `queued-${command.command_id}`,
                  text: command.text,
                  created_at_ms: Date.now(),
                  attachments: [],
                  worker_roles: [],
                },
              },
            ],
          },
        };
        eventSequence += 1;
        peer.send({
          type: "semantic_event",
          event_seq: eventSequence,
          event: {
            type: "message_queue_updated",
            session_id: authoritativeSession.session_id,
            message_queue: authoritativeSession.message_queue,
          },
        });
        // Transport acceptance is not business completion. The authoritative
        // Session queue projection nevertheless makes this future task visible
        // immediately, without browser-owned persistence or replay.
        return;
      }
      // Keep turn_cancel durable until authoritative TurnFinished. This
      // reproduces a reload that receives an older working snapshot while
      // cancellation is already accepted by Host.
      if (command.type !== "turn_cancel")
        peer.send({ type: "command_ack", command_id: command.command_id, status: "committed" });
    });
    peers.add(peer);
    // A headless browser may reset the loopback socket at any time (this
    // suite models reconnects explicitly), so peer resets are lifecycle
    // events, not crashes: drain the error and let the close handler clean up.
    socket.on("error", () => peers.delete(peer));
    socket.on("close", () => peers.delete(peer));
    peer.send({
      type: "hello", snapshot: makeSnapshot(authoritativeSession),
      event_cursor: eventSequence, event_replay_floor: 0,
    });
  });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  return {
    url: `http://127.0.0.1:${server.address().port}/`, commands,
    send(event) {
      eventSequence += 1;
      const envelope = { type: "semantic_event", event_seq: eventSequence, event };
      for (const peer of peers) peer.send(envelope);
    },
    getSession() { return authoritativeSession; },
    getConnectionCount() { return connectionCount; },
    setSession(session) { authoritativeSession = session; },
    close() { return new Promise((resolve) => server.close(resolve)); },
  };
}

async function waitForProcessExit(child, timeoutMs) {
  if (child.exitCode !== null) return true;
  return Promise.race([
    new Promise((resolve) => child.once("exit", () => resolve(true))),
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
      if (
        Date.now() >= deadline ||
        !["EBUSY", "ENOTEMPTY", "EPERM"].includes(error?.code)
      ) {
        throw error;
      }
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
    const [portLine] = (await readFile(join(profile, "DevToolsActivePort"), "utf8"))
      .trim()
      .split("\n");
    const port = Number(portLine);
    return Number.isInteger(port) && port > 0 ? port : null;
  } catch {
    return null;
  }
}

async function startBrowser(url) {
  const profile = await mkdtemp(join(tmpdir(), "timem-stop-ui-"));
  const child = spawn(chrome, [
    "--remote-debugging-port=0", `--user-data-dir=${profile}`,
    "--headless=new", "--no-sandbox", "--disable-dev-shm-usage",
      "--lang=zh-CN", "--accept-lang=zh-CN",
    "--no-first-run", "--no-default-browser-check",
    "--disable-background-networking", "--disable-component-update", "--disable-sync",
    "--window-size=1440,1000", "about:blank",
  ], { stdio: ["ignore", "ignore", "pipe"] });
  let chromeError = "";
  child.stderr.on("data", (chunk) => { chromeError += String(chunk); });

  try {
    let port = null;
    await waitFor(async () => {
      if (child.exitCode !== null) return false;
      port = await readDevToolsPort(profile);
      if (port === null) return false;
      try { return (await fetch(`http://127.0.0.1:${port}/json/version`)).ok; }
      catch { return false; }
    }, `Chrome DevTools did not start: ${chromeError}`, 12000);

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
    // Pin the media preference so assertions cannot inherit the host OS
    // accessibility setting (the macOS 26 runner image enables system
    // Reduce Motion, which silently disabled every entrance animation).
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


async function main() {
 const host=await startHost(); const browser=await startBrowser(host.url);
 const path='applications/timem/tests/unit/web_host_tests.rs';
 try {
  await waitFor(()=>browser.evaluate(`document.body.textContent.includes('Long task')`),'initial snapshot');
  for(const stream of [false,true]) {
   await browser.evaluate(`localStorage.setItem('timem-web-stream-ui-mode-v1',${JSON.stringify(String(stream))})`);
   const action={event_id:'file-read',source:'core_topic',created_at_ms:Date.now(),payload:{session_id:'session-1',state:{name:'running'},topic:{name:'core.action',attributes:{}},payload:{action:'readfile',action_id:'file-read',status:'completed',input:{ender:{line_nr:15244},path,starter:{line_nr:15190}}}}};
   host.setSession(makeSession({turns:[{...turn('turn-1'),events:[action,{...action,event_id:'memory-search',payload:{...action.payload,payload:{action:'memmgr',action_id:'memory-search',status:'completed',input:{type:'raw_chat',op:'search',search_text:'project'}}}},{...action,event_id:'memory-delete',payload:{...action.payload,payload:{action:'memmgr',action_id:'memory-delete',status:'completed',input:{type:'scratch',op:'delete',id:'scratch-id'}}}},{...action,event_id:'self-inspect',payload:{...action.payload,payload:{action:'self_tool',action_id:'self-inspect',status:'completed',input:{type:'params'}}}},{...action,event_id:'file-failed',payload:{...action.payload,payload:{action:'readfile',action_id:'file-failed',status:'failed',input:{path:'missing.txt'}}}}]}]}));
   await browser.call('Page.reload');
   try { await waitFor(()=>browser.evaluate(`!!document.querySelector('.file-tool-preview')`),'file preview mode '+stream); } catch(error) { console.log(await browser.evaluate(`document.body.innerText`)); throw error; }
   assert(await browser.evaluate(`document.querySelectorAll('.memory-search-icon .lucide-database-search').length===1`),'memory search icon only for search');
   assert(await browser.evaluate(`document.querySelectorAll('.memory-tool-icon .lucide-database').length===1`),'other memory operations use Database');
   assert(await browser.evaluate(`document.querySelectorAll('.self-tool-icon .lucide-eye').length===1 && !document.querySelector('.self-tool-icon .lucide-info')`),'self_tool uses Eye');
   assert(await browser.evaluate(`document.querySelectorAll('.tool-failure-icon .lucide-circle-x').length===1 && !document.querySelector('.stream-tool-status,.tool-activity-status')`),'failed tool replaces its native icon with one CircleX and no extra verdict');
   assert(await browser.evaluate(`(() => {
     const rows=[...document.querySelectorAll(${stream ? "'.stream-tool-head'" : "'.tool-activity > summary, .tool-activity-static'"})];
     const success=rows.find(row=>row.querySelector('.file-tool-icon'));
     const failure=rows.find(row=>row.querySelector('.tool-failure-icon'));
     if(!success||!failure) return false;
     const successIcon=success.querySelector(':scope > b')?.getBoundingClientRect();
     const failureIcon=failure.querySelector(':scope > b')?.getBoundingClientRect();
     const successContent=success.querySelector('.file-tool-preview')?.getBoundingClientRect();
     const failureContent=failure.querySelector('.file-tool-preview')?.getBoundingClientRect();
     return success.firstElementChild?.tagName==='B' && failure.firstElementChild?.tagName==='B' &&
       !!successIcon && !!failureIcon && !!successContent && !!failureContent &&
       Math.abs(successIcon.left-failureIcon.left)<1 && Math.abs(successContent.left-failureContent.left)<1;
   })()`),'successful and failed tools share the same icon/content columns');
   assert(await browser.evaluate(`!document.querySelector('.stream-tool-toggle > svg, .tool-activity > summary > .tool-activity-chevron')`),'no persistent disclosure glyph');
   if(!stream) await browser.evaluate(`document.querySelectorAll('.tool-activity-group').forEach(e=>e.open=true)`);
   for(const width of [1280,390]) {
    await browser.call('Emulation.setDeviceMetricsOverride',{width,height:1000,deviceScaleFactor:1,mobile:false});
    for(const theme of ['dark','light']) {
     await browser.evaluate(`document.documentElement.dataset.theme=${JSON.stringify(theme)}`);
     const info=await browser.evaluate(`(()=>{const p=document.querySelector('.file-tool-preview');p.scrollIntoView({block:'center'});const r=p.getBoundingClientRect();const n=p.querySelector('.file-tool-name').getBoundingClientRect();const range=p.querySelector('.file-tool-range');const row=p.closest('.stream-tool-run,.tool-activity-group');const parent=row?.parentElement;const rowWidth=row?.getBoundingClientRect().width??0;const parentWidth=parent?.getBoundingClientRect().width??0;const style=getComputedStyle(p);return {width:r.width,inside:n.left>=r.left&&n.right<=r.right+1,overflow:p.scrollWidth>p.clientWidth+1,range:range.textContent,name:p.querySelector('.file-tool-name').textContent,title:p.title,icon:!!document.querySelector('.file-tool-icon .lucide-square-text'),font:style.fontFamily,fontSize:style.fontSize,rowRatio:parentWidth?rowWidth/parentWidth:0};})()`);
     const expectedRatio=width<=720?1:.9;
     assert(info.width>0 && info.inside && !info.overflow && info.icon && info.font.includes('IBM Plex Mono') && info.fontSize==='11.5px' && Math.abs(info.rowRatio-expectedRatio)<.015,'geometry/font/width '+stream+' '+width+' '+theme+' '+JSON.stringify(info));
     assert(info.range.includes('15190–15244') && info.name==='web_host_tests.rs' && info.title.includes(path),'semantic content');
    }
   }
   const selector=stream?'.stream-tool-toggle':'.tool-activity > summary';
   assert(await browser.evaluate(`(() => {const row=document.querySelector(${JSON.stringify(selector)});const children=[...row.children];const tool=children.findIndex(e=>e.tagName==='B');const preview=children.findIndex(e=>e.matches('.file-tool-preview,.stream-tool-command-preview,.tool-activity-command'));const visibleStatus=children.find(e=>e.matches('.stream-tool-status-slot,.tool-activity-running-marker,.stream-tool-status,.tool-activity-status'));return !visibleStatus&&tool===0&&preview>tool&&!row.textContent.includes('✓');})()`),'completed tool removes status marker and shifts left');
   await browser.evaluate(`document.querySelector(${JSON.stringify(selector)}).scrollIntoView({block:'center'})`);
   await browser.evaluate(`document.querySelector(${JSON.stringify(selector)}).dispatchEvent(new MouseEvent('mouseover',{bubbles:true}))`);
   assert(await browser.evaluate(`getComputedStyle(document.querySelector(${JSON.stringify(selector)})).cursor==='pointer'`),'whole row has pointer affordance');
   const hit=await browser.evaluate(`(()=>{const e=document.querySelector(${JSON.stringify(selector)}),r=e.getBoundingClientRect();return {x:r.x+r.width/2,y:r.y+r.height/2};})()`);
   for(const type of ['mousePressed','mouseReleased']) await browser.call('Input.dispatchMouseEvent',{type,x:hit.x,y:hit.y,button:'left',clickCount:1});
   await waitFor(()=>browser.evaluate(stream?`document.querySelector('.stream-tool-toggle').getAttribute('aria-expanded')==='true'`:`document.querySelector('.tool-activity').open`),'pointer expands details');
   assert(await browser.evaluate(`document.querySelector(${JSON.stringify(stream?'.stream-tool-detail':'.tool-activity-body')}).textContent.includes('line_nr')`),'original parameters preserved');
   await browser.evaluate(`document.querySelector(${JSON.stringify(selector)}).focus()`);
   for(const type of ['keyDown','keyUp']) await browser.call('Input.dispatchKeyEvent',{type,key:' ',code:'Space',windowsVirtualKeyCode:32});
   await waitFor(()=>browser.evaluate(stream?`document.querySelector('.stream-tool-toggle').getAttribute('aria-expanded')==='false'`:`!document.querySelector('.tool-activity').open`),'keyboard collapses details');
  }
  console.log('PASS tool presentation: ordinary/stream alignment, Eye self_tool, single CircleX failure icon, semantic summaries, pointer/keyboard, light/dark, 390px');
 } finally {await browser.close();await host.close();}
}
await main();
