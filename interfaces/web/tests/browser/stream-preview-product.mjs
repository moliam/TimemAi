import { createHash } from "node:crypto";
import { createServer } from "node:http";
import { existsSync } from "node:fs";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
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
  events: [], sub_answers: [], final_answer: null, completion: null,
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
  const mem = await mkdtemp(join(tmpdir(), "timem-stream-product-"));
  await writeFile(join(mem, "Cargo.toml"), "stream readfile acceptance fixture");
  const scenario = process.env.STREAM_PREVIEW_SCENARIO ?? "normal";
  const protocol = process.env.STREAM_PREVIEW_PROTOCOL ?? "xml";
  assert(["xml", "json", "native"].includes(protocol), "unsupported preview protocol");
  assert(["normal", "invalid", "network", "stop", "supplement", "interaction", "tools"].includes(scenario), "unsupported preview scenario");
  assert(protocol === "xml" || scenario === "normal", "non-XML scenarios currently support normal only");
  let release;
  let releaseFinal;
  let appendStreaming;
  let requests = 0;
  const modelInputs = [];
  const model = createServer(async (req, res) => {
    let body = ""; for await (const chunk of req) body += chunk; modelInputs.push(body);
    if (body.includes("timem_capability_probe")) {
      res.writeHead(200, {"Content-Type":"application/json"});
      res.end(JSON.stringify({choices:[{message:{content:"probe unsupported"}}]})); return;
    }
    assert(JSON.parse(body).stream === true, "streaming request must not require TIMEM_STREAM environment configuration");
    requests++;
    res.writeHead(200, {"Content-Type":"text/event-stream"});
    const write = (content) => res.write(`data: ${JSON.stringify({choices:[{delta:{content}}]})}\n\n`);
    appendStreaming = write;
    if (protocol === "native") {
      if (requests === 1) {
        write("HTTP early response");
        res.write(`data: ${JSON.stringify({choices:[{delta:{tool_calls:[{index:0,id:"chat-call",type:"function",function:{name:"sub_answer",arguments:'{"task":"HTTP interim","answer":"HTTP early chat'}}]}}]})}\n\n`);
        await new Promise(resolve => { release = resolve; });
        res.write(`data: ${JSON.stringify({choices:[{delta:{tool_calls:[{index:0,function:{arguments:'"}'}}]}}]})}\n\n`);
      } else {
        if (scenario === "normal" || scenario === "tools") await new Promise(resolve => { releaseFinal = resolve; });
        write("HTTP final");
      }
      res.end("data: [DONE]\n\n"); return;
    }
    if (requests === 1) {
      write(protocol === "json" ? '{"status":"working","free_talk":"HTTP early response","working_still_action":[{"sub_answer":{"task":"HTTP interim","answer":"HTTP early chat' : '<ASSISTANT><free_talk>HTTP early response</free_talk><actions><sub_answer><task>HTTP interim</task><answer>HTTP early chat');
      await new Promise(resolve => { release = resolve; });
      if (scenario === 'network') { res.destroy(); return; }
      if (scenario === 'invalid') write('</answer></sub_answer></actions><invalid></ASSISTANT>');
      else if (scenario === "tools") write('</answer></sub_answer><readfile><path>Cargo.toml</path><max_bytes>200</max_bytes></readfile></actions></ASSISTANT>');
      else write(protocol === "json" ? '"}}]}' : '</answer></sub_answer></actions></ASSISTANT>');
    } else {
      if (scenario === "normal" || scenario === "tools") await new Promise(resolve => { releaseFinal = resolve; });
      write(protocol === "json" ? JSON.stringify({status:"all_finished",final_answer:"HTTP final"}) : "<ASSISTANT><finish_confirm>Now let me think seriously twice before I announce stop. Review user's task list. Is my delivery consistent with user's demand?</finish_confirm><final_answer>HTTP final</final_answer></ASSISTANT>");
    }
    res.end("data: [DONE]\n\n");
  });
  await new Promise(resolve => model.listen(0,"127.0.0.1",resolve));
  const child = spawn(resolve(root,"../../target/debug/timem"), ["--no-open","--space",mem,"--port","18987"], {
    env:{PATH:process.env.PATH,HOME:mem,TIMEM_API_KEY:"dummy",TIMEM_API_PROTOCOL:"openai-compatible",TIMEM_RESPONSE_PROTOCOL:protocol === "native" ? "xml" : protocol,TIMEM_TOOL_CALL_MODE:protocol === "native" ? "native" : "inline",TIMEM_BASE_URL:`http://127.0.0.1:${model.address().port}/v1`,TIMEM_MODEL:"preview-test",TIMEM_WORK_INSTRUCTIONS:"off"}, stdio:["ignore","pipe","pipe"]
  });
  let logs = ""; child.stdout.on("data",x=>logs+=x); child.stderr.on("data",x=>logs+=x);
  let browser, socket;
  const received = [];
  try {
    await waitFor(async()=>{try{return (await fetch("http://127.0.0.1:18987/")).ok;}catch{return false;}},"product did not start",30000);
    socket = new WebSocket("ws://127.0.0.1:18987/ws");
    let sessionId;
    socket.addEventListener("message",({data})=>{let e=JSON.parse(String(data)); received.push(e); if(e.type==="semantic_event")e=e.event; if(e.type==="session_created")sessionId=e.session.session_id;});
    await new Promise((resolve,reject)=>{socket.addEventListener("open",resolve,{once:true});socket.addEventListener("error",reject,{once:true});});
    socket.send(JSON.stringify({type:"session_create",display_name:"HTTP streaming acceptance",workspace_dir:mem}));
    await waitFor(()=>sessionId,"session creation failed");
    browser = await startBrowser("http://127.0.0.1:18987/");
    await browser.evaluate(`localStorage.setItem("timem-web-stream-ui-mode-v1","true")`);
    await browser.call("Page.reload",{ignoreCache:true});
    await waitFor(()=>browser.evaluate(`!!document.querySelector('textarea[aria-label="Message Timem"]')`),"composer missing");
    await waitFor(() => browser.evaluate(`!!document.querySelector('button.session[title="HTTP streaming acceptance"]')`), "session button missing");
    await browser.evaluate(`document.querySelector('button.session[title="HTTP streaming acceptance"]')?.click()`);
    socket.send(JSON.stringify({type:"turn_submit",session_id:sessionId,text:"HTTP streaming acceptance"}));
    await waitFor(()=>browser.evaluate(`document.querySelector('.response-preview')?.textContent.includes('HTTP early response')`),"real HTTP response preview absent");
    await waitFor(()=>browser.evaluate(`document.querySelector('.provisional-chat')?.textContent.includes('HTTP early chat')`),"real HTTP Chat preview absent");
    assert(requests===1,"response finished before browser observed preview");
    if (scenario === "interaction") {
      appendStreaming("\n\n" + Array.from({length:180}, (_,i) => `Paragraph ${i} streaming text.`).join("\n\n"));
      await waitFor(() => browser.evaluate(`document.querySelector('.provisional-chat')?.textContent.includes('Paragraph 179')`), "long streamed Chat missing");
      await browser.call("Browser.grantPermissions", {origin:"http://127.0.0.1:18987", permissions:["clipboardReadWrite", "clipboardSanitizedWrite"]});
      await browser.evaluate(`(() => { const node=document.querySelector('.provisional-chat .message-content p');const r=document.createRange();r.selectNodeContents(node);const sel=window.getSelection();sel.removeAllRanges();sel.addRange(r); })()`);
      await browser.call("Input.dispatchKeyEvent", {type:"keyDown",key:"c",code:"KeyC",modifiers:4,commands:["copy"]});
      await browser.call("Input.dispatchKeyEvent", {type:"keyUp",key:"c",code:"KeyC",modifiers:4});
      assert((await browser.evaluate(`navigator.clipboard.readText()`)).includes("HTTP early chat"), "streamed Chat clipboard copy failed");
      const before = await browser.evaluate(`(() => { const sc=document.querySelector('[data-session-timeline-active="true"] .provisional-chat').closest('.chat-scroll');sc.scrollTop=100;return sc.scrollTop; })()`);
      appendStreaming("\n\nCONTINUED_AFTER_COPY");
      await waitFor(() => browser.evaluate(`document.querySelector('.provisional-chat')?.textContent.includes('CONTINUED_AFTER_COPY')`), "stream did not continue after copy");
      const after = await browser.evaluate(`document.querySelector('.provisional-chat').closest('.chat-scroll').scrollTop`);
      assert(Math.abs(after-before)<8, `stream moved reading position: ${before} -> ${after}`);
      assert((await browser.evaluate(`navigator.clipboard.readText()`)).includes("HTTP early chat"), "stream update changed clipboard");
    }
    if (scenario === "normal") {
      await browser.call("Page.reload", {ignoreCache:true});
      await waitFor(() => browser.evaluate(`!!document.querySelector('button.session[title="HTTP streaming acceptance"]')`), "session missing after reload");
      await browser.evaluate(`document.querySelector('button.session[title="HTTP streaming acceptance"]')?.click()`);
      await waitFor(()=>browser.evaluate(`document.querySelector('.provisional-chat')?.textContent.includes('HTTP early chat')`), "HTTP streaming reload lost Chat");
      await browser.evaluate(`document.querySelector('button.session[title="Untitled 1"]')?.click()`);
      await waitFor(()=>browser.evaluate(`!document.querySelector('[data-session-timeline-active="true"] .provisional-chat')`), "preview leaked into other Session");
      await browser.evaluate(`document.querySelector('button.session[title="HTTP streaming acceptance"]')?.click()`);
      await waitFor(()=>browser.evaluate(`document.querySelector('.provisional-chat')?.textContent.includes('HTTP early chat')`), "switch back lost Chat");
      assert(await browser.evaluate(`(() => { const node=document.querySelector('.provisional-chat .message-content');const range=document.createRange();range.selectNodeContents(node);const selection=window.getSelection();selection.removeAllRanges();selection.addRange(range);return selection.toString()==='HTTP early chat'; })()`), "partial Chat text cannot be selected");
    }

    if (scenario === "stop") {
      await browser.evaluate(`document.querySelector('button[aria-label="Cancel current turn"]')?.click()`);
      await waitFor(()=>browser.evaluate(`document.querySelector('.response-preview-interruption')?.textContent.includes('Stopped')`), "Stop lost partial output");
      await waitFor(()=>browser.evaluate(`!document.querySelector('button[aria-label="Cancel current turn"]')`), "Stop did not restore Send");
      socket.send(JSON.stringify({type:"turn_submit",command_id:"stream-stop-next-send",session_id:sessionId,text:"Immediately after Stop"}));
      await waitFor(()=>browser.evaluate(`document.querySelector('.turn-final-delivery')?.textContent.includes('HTTP final')`), "next Send after Stop did not complete");
      assert(requests === 2, "next Send was retried or queued incorrectly");
      console.log("PASS actual Host + HTTP SSE + Chrome: Stop retains partial output; immediate next Send completes");
      return;
    }
    if (scenario === "supplement") {
      socket.send(JSON.stringify({type:"turn_supplement",session_id:sessionId,text:"SUPPLEMENT_STREAM_CHECK"}));
      await waitFor(() => received.some(raw => JSON.stringify(raw).includes("SUPPLEMENT_STREAM_CHECK")), "supplement admission missing");
      assert(!modelInputs[0].includes("SUPPLEMENT_STREAM_CHECK"), "supplement attributed to already-sent request");
    }
    await browser.evaluate(`window.__previewNode = document.querySelector(".response-preview"); window.__chatNode = document.querySelector(".provisional-chat"); true`);
    release();
    if (scenario === "normal" || scenario === "tools") {
      await waitFor(() => !!releaseFinal, "next model request missing");
      await waitFor(() => browser.evaluate(`window.__chatNode?.isConnected && !window.__chatNode.classList.contains('provisional-chat')`), "Chat confirmation replaced its DOM node");
      assert(await browser.evaluate(`document.querySelectorAll('.turn-interim-item').length === 1`), "Chat confirmation duplicated the answer");
      if (scenario === "tools") {
        await waitFor(() => browser.evaluate(`document.querySelector('.turn-work-content')?.textContent.includes('readfile')`), "executed readfile missing while preview enabled");
        await waitFor(() => received.some(raw => { const e = raw.type === "semantic_event" ? raw.event : raw; return e.event?.topic?.name === "core.action" && e.event.payload.action === "readfile" && e.event.payload.event === "finish"; }), "readfile execution evidence missing");
      }
      releaseFinal();
    }
    if (scenario === "network") {
      await waitFor(()=>browser.evaluate(`document.querySelector('.response-preview-interruption')?.textContent.includes('Network error')`), "network lost partial output");
      assert(await browser.evaluate(`document.querySelector('.provisional-chat')?.textContent.includes('HTTP early chat')`), "network lost partial Chat");
      console.log("PASS actual Host + HTTP SSE + Chrome: broken HTTP retains partial response and Chat");
      return;
    }
    await waitFor(()=>browser.evaluate(`document.querySelector('.turn-final-delivery')?.textContent.includes('HTTP final')`),"final answer absent",20000);
    if (scenario === "tools") {
      await waitFor(() => browser.evaluate(`!document.querySelector('button[aria-label="Cancel current turn"]')`), "terminal projection missing");
      assert(await browser.evaluate(`Array.from(document.querySelectorAll('.turn-assistant-heading')).some(e=>e.textContent.includes('Thought/Action'))`), "Thought/Action disappeared after final delivery");
      assert(await browser.evaluate(`document.querySelector('.turn-work-content')?.textContent.includes('readfile')`), "tool details disappeared after final delivery");
    }
    if (scenario !== "invalid") assert(await browser.evaluate(`window.__previewNode === document.querySelector('.turn-final-delivery')`), "final confirmation replaced the answer DOM node");
    if (scenario === "supplement") {
      assert(modelInputs.slice(1).some(body => body.includes("SUPPLEMENT_STREAM_CHECK")), "next request did not consume supplement");
    }
    if (scenario === "invalid") {
      assert(!received.some(raw => { const e = raw.type === "semantic_event" ? raw.event : raw; return e.event?.topic?.name === "core.sub_answer"; }), "invalid response executed sub_answer");
      assert(await browser.evaluate(`document.querySelectorAll('.provisional-chat').length === 0`), "invalid chat survived repair");
      assert(received.some(raw => { const e = raw.type === "semantic_event" ? raw.event : raw; return e.event?.topic?.name === "core.model.preview" && e.event.payload.attempt === 1 && e.event.payload.response === null && e.event.payload.chat.length === 0; }), "invalid attempt did not retract all previews");
    }
    console.log(`PASS actual Host + HTTP SSE + Chrome: protocol=${protocol} scenario=${scenario}; response and Chat visible before HTTP completion, final delivered`);
  } catch(error) { console.error(logs); console.error("requests",requests); console.error("action evidence", JSON.stringify(received.flatMap(raw => { const e=raw.type === "semantic_event" ? raw.event : raw; return e.event?.topic?.name === "core.action" ? [e.event.payload] : []; }))); console.error("host errors", JSON.stringify(received.filter(e => JSON.stringify(e).includes("host_error")))); if(browser)console.error(await browser.evaluate("document.body.innerText")); throw error; }
  finally {
    release?.(); releaseFinal?.(); socket?.close(); if(browser)await browser.close();
    child.kill("SIGTERM"); await waitForProcessExit(child,5000);
    model.closeAllConnections(); await new Promise(resolve=>model.close(resolve));
    await rm(mem,{recursive:true,force:true});
  }
}
await main();
