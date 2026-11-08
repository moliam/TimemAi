// End-to-end acceptance for the "thinking chain in context" system setting:
// a real Host process talks to a mock OpenAI-compatible SSE provider, and the
// second model request body must carry the first response's reasoning only
// while the setting is on (default on, explicitly off afterwards).
import { spawn } from "node:child_process";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { createServer } from "node:http";

const root = resolve(fileURLToPath(new URL("../..", import.meta.url)));
const sleep = (ms) => new Promise((settle) => setTimeout(settle, ms));
const assert = (condition, message) => { if (!condition) throw new Error(message); };
async function waitFor(check, message, timeout = 20000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    try { if (await check()) return; } catch {}
    await sleep(50);
  }
  throw new Error("TIMEOUT: " + message);
}

const MARKER_ON = "REASONING_CONTEXT_MARKER_ON";
const MARKER_OFF = "REASONING_CONTEXT_MARKER_OFF";
const PORT = 18989;

// Mock provider: every turn streams one reasoning delta and one XML body that
// asks for a real readfile action, so the turn continues to a second request.
const modelInputs = [];
let reasoningForTurn = MARKER_ON;
const model = createServer(async (req, res) => {
  let body = "";
  for await (const chunk of req) body += chunk;
  if (body.includes("timem_capability_probe")) {
    res.writeHead(200, { "Content-Type": "application/json" });
    res.end(JSON.stringify({ choices: [{ message: { content: "probe unsupported" } }] }));
    return;
  }
  modelInputs.push(body);
  res.writeHead(200, { "Content-Type": "text/event-stream" });
  const write = (payload) => res.write(`data: ${JSON.stringify(payload)}\n\n`);
  write({ choices: [{ delta: { reasoning_content: reasoningForTurn } }] });
  const turn = modelInputs.length;
  if (turn <= 2) {
    // Alternate between the two verification turns: each turn's second request
    // (even indexes) must reflect that turn's marker.
    write({ choices: [{ delta: { content: '<ASSISTANT><free_talk>verifying reasoning context</free_talk><actions><readfile><path>Cargo.toml</path><ender><byte_nr>199</byte_nr></ender></readfile></actions></ASSISTANT>' } }] });
  } else {
    write({ choices: [{ delta: { content: "<ASSISTANT><finish_confirm>Reviewed the fixture read.</finish_confirm><final_answer>reasoning context check done</final_answer></ASSISTANT>" } }] });
  }
  res.write("data: [DONE]\n\n");
  res.end();
});

const mem = await mkdtemp(join(tmpdir(), "timem-reasoning-context-"));
let child;
let socket;
const received = [];
let logs = "";
try {
  await new Promise((done) => model.listen(0, "127.0.0.1", done));
  child = spawn(resolve(root, "../../target/debug/timem"), ["--no-open", "--space", mem, "--port", String(PORT)], {
    env: { PATH: process.env.PATH, HOME: mem, TIMEM_API_KEY: "dummy", TIMEM_API_PROTOCOL: "openai-compatible", TIMEM_RESPONSE_PROTOCOL: "xml", TIMEM_TOOL_CALL_MODE: "inline", TIMEM_BASE_URL: `http://127.0.0.1:${model.address().port}/v1`, TIMEM_MODEL: "reasoning-test", TIMEM_WORK_INSTRUCTIONS: "off" },
    stdio: ["ignore", "pipe", "pipe"],
  });
  child.stdout.on("data", (x) => { logs += x; });
  child.stderr.on("data", (x) => { logs += x; });
  await waitFor(async () => { try { return (await fetch(`http://127.0.0.1:${PORT}/`)).ok; } catch { return false; } }, "host did not start", 40000);

  socket = new WebSocket(`ws://127.0.0.1:${PORT}/ws`);
  let sessionId;
  socket.addEventListener("message", ({ data }) => {
    let event = JSON.parse(String(data));
    if (event.type === "semantic_event") event = event.event;
    received.push(event);
    if (event.type === "session_created") sessionId = event.session.session_id;
  });
  await new Promise((done, fail) => {
    socket.addEventListener("open", done, { once: true });
    socket.addEventListener("error", fail, { once: true });
  });
  const send = (command) => socket.send(JSON.stringify(command));
  const ack = (commandId) => received.some((e) => e.type === "command_ack" && e.command_id === commandId && e.status === "committed");
  const finishedTurns = () => received.filter((e) => e.type === "turn_finished").length;
  const runTurn = async (commandId, text, label) => {
    const before = finishedTurns();
    send({ type: "turn_submit", command_id: commandId, session_id: sessionId, text });
    await waitFor(() => finishedTurns() > before, `${label} turn never finished`, 60000);
  };

  send({ type: "session_create", command_id: "rc-session", display_name: "Reasoning context", workspace_dir: mem });
  await waitFor(() => sessionId, "session creation failed");
  send({ type: "model_endpoint_upsert", command_id: "rc-endpoint", endpoint: { id: "rc-endpoint", name: "Reasoning mock", model: "reasoning-test", api_protocol: "openai-compatible", response_protocol: "xml", base_url: `http://127.0.0.1:${model.address().port}/v1`, api_key: "dummy", max_llm_input_tokens: 100000, max_llm_output_tokens: 10000, stream: true } });
  await waitFor(() => ack("rc-endpoint"), "endpoint creation failed");
  send({ type: "model_endpoint_apply", command_id: "rc-apply", session_id: sessionId, endpoint_id: "rc-endpoint" });
  await waitFor(() => ack("rc-apply"), "endpoint apply failed");

  // Turn 1 (default setting on): the follow-up request must replay reasoning.
  reasoningForTurn = MARKER_ON;
  await runTurn("rc-turn-on", "verify reasoning context default", "first");
  await waitFor(() => modelInputs.some((body) => body.includes(MARKER_ON)), "first turn never replayed its reasoning");
  const replayRequest = modelInputs.find((body) => body.includes(MARKER_ON));
  assert(replayRequest.includes("verify reasoning context default"), "reasoning request lost the user input");

  // Turn 2 (explicitly off via the system command): the replay must stop.
  send({ type: "system_include_reasoning_in_context_update", command_id: "rc-off", enabled: false });
  await waitFor(() => received.some((e) => e.type === "mem_settings_updated" && e.include_reasoning_in_context === false), "system setting did not persist off");
  reasoningForTurn = MARKER_OFF;
  await runTurn("rc-turn-off", "verify reasoning context disabled", "second");
  const offRequest = modelInputs.find((body) => body.includes("verify reasoning context disabled"));
  assert(!offRequest.includes(MARKER_OFF), "disabled setting must not replay the new reasoning");
  // Turn 1's reasoning already entered the context while the setting was on;
  // turning the setting off stops future replay without rewriting history.
  assert(offRequest.includes(MARKER_ON), "history from the enabled turn must stay in context");

  console.log("PASS reasoning context e2e: default on replays the thinking chain, system off stops it");
} catch (error) {
  console.error(logs);
  console.error("model requests", modelInputs.length);
  console.error("event types", received.map((e) => e.type));
  console.error("acked", received.filter((e) => e.command_id).map((e) => `${e.type}:${e.command_id}:${e.status ?? ""}`));
  throw error;
} finally {
  try { socket?.close(); } catch {}
  if (child) { child.kill("SIGTERM"); await sleep(500); try { child.kill("SIGKILL"); } catch {} }
  model.closeAllConnections();
  await new Promise((done) => model.close(done));
  await rm(mem, { recursive: true, force: true }).catch(() => {});
}
