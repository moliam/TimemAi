import { describe, expect, it } from "vitest";
import { ChatMessage, CoreTopicEvent, Session, WebTurn, WebTurnEvent } from "../src/protocol";
import { activityFromTopic, appendTurnEvent, applyCoreTopicToSession, attachTurnCompletion, boundSessionHistory, clearDecisionsForSession, clearDecisionsForWorker, coalesceActionLifecycle, enqueueDecision, finishTurn, MAX_CLIENT_TURN_EVENTS, MAX_CLIENT_TURNS, MAX_RENDERED_MESSAGES, removePendingAttachment, requestDecision, sessionContextUsage, tailPath, trimMessages, turnLiveUsage, updateSessionWorkerState, upsertSession, upsertTurn } from "../src/view_model";

const topic = (name: string, payload: Record<string, unknown>, state = "running"): CoreTopicEvent => ({
  session_id: "session_1",
  topic: { name, attributes: {} },
  state: { name: state },
  payload,
});

const session = (sessionId: string): Session => ({
  session_id: sessionId,
  display_name: sessionId,
  ordinal: 0,
  state: "ready",
  current_dir: "/work",
  max_llm_input_tokens: 100_000,
  contexts: [{ context_id: `context_${sessionId}`, current_dir: "/work", worker_ids: [`worker_${sessionId}`] }],
  workers: [{ worker_id: `worker_${sessionId}`, context_id: `context_${sessionId}`, display_name: sessionId, ordinal: 0, state: "ready", parent_worker_id: null }],
  active_context_id: `context_${sessionId}`,
  primary_worker_id: `worker_${sessionId}`,
  attachments: [],
  messages: [],
  turns: [],
  active_turn_id: null,
});

const turn = (turnId: string, state = "working"): WebTurn => ({
  turn_id: turnId,
  state,
  created_at_ms: 1,
  user_entries: [{ kind: "task", text: "do the work", created_at_ms: 1 }],
  events: [],
  final_answer: null,
  completion: null,
});

const assistantMessage = (text: string): ChatMessage => ({
  id: `assistant-${text}`,
  role: "assistant",
  text,
  created_at_ms: 1,
});

const actionEvent = (
  id: string,
  lifecycle: "start" | "finish",
  status: string,
  input: Record<string, unknown> = { cmd: "git status" },
): WebTurnEvent => ({
  event_id: id,
  source: "core_topic",
  created_at_ms: Number(id.replace(/\D/g, "")) || 1,
  payload: {
    session_id: "session_1",
    topic: { name: "core.action", attributes: { event: lifecycle } },
    state: { name: "running" },
    payload: { action: "run_bash", input, event: lifecycle, status },
  },
});

describe("web topic view model", () => {
  it("shows the tail of a long cwd while retaining short paths verbatim", () => {
    expect(tailPath("/short/workspace")).toBe("/short/workspace");
    const rendered = tailPath("/Users/example/very/long/company/project/packages/web-ui", 24);
    expect(rendered.startsWith("…")).toBe(true);
    expect(rendered.endsWith("project/packages/web-ui")).toBe(true);
    expect(rendered).toHaveLength(24);
  });

  it("replaces an action start with its terminal lifecycle event", () => {
    const events = coalesceActionLifecycle([
      actionEvent("event_1", "start", "running"),
      actionEvent("event_2", "finish", "completed"),
    ]);
    expect(events).toHaveLength(1);
    expect((events[0].payload.payload as Record<string, unknown>).status).toBe("completed");
  });

  it("pairs duplicate concurrent actions in order without collapsing either invocation", () => {
    const events = coalesceActionLifecycle([
      actionEvent("event_1", "start", "running"),
      actionEvent("event_2", "start", "running"),
      actionEvent("event_3", "finish", "completed"),
      actionEvent("event_4", "finish", "timeout"),
    ]);
    expect(events).toHaveLength(2);
    expect(events.map((event) => (event.payload.payload as Record<string, unknown>).status)).toEqual(["completed", "timeout"]);
  });

  it("keeps a background action visibly active after its launch event finishes", () => {
    const events = coalesceActionLifecycle([
      actionEvent("event_1", "start", "running", { cmd: "cargo test", background: true }),
      actionEvent("event_2", "finish", "background_running", { cmd: "cargo test", background: true }),
    ]);
    expect(events).toHaveLength(1);
    expect((events[0].payload.payload as Record<string, unknown>).status).toBe("background_running");
  });

  it("keeps one session working when a subworker finishes and hides its final answer", () => {
    let current = session("session_1");
    current.contexts.push({ context_id: "context_sub", current_dir: "/work/sub", worker_ids: ["worker_sub"] });
    current.workers.push({ worker_id: "worker_sub", context_id: "context_sub", display_name: "Subtask", ordinal: 1, state: "ready", parent_worker_id: current.primary_worker_id });
    current = updateSessionWorkerState(current, current.primary_worker_id, "working");
    current = updateSessionWorkerState(current, "worker_sub", "working");
    const subResponse: CoreTopicEvent = {
      ...topic("core.model.response", { status: "finished", continue_work: false, final_answer: "subtask-only answer" }),
      context_id: "context_sub",
      worker_id: "worker_sub",
    };

    const updated = applyCoreTopicToSession(current, subResponse, assistantMessage);
    expect(updated.state).toBe("working");
    expect(updated.messages).toEqual([]);
    expect(updated.workers.find((worker) => worker.worker_id === "worker_sub")?.state).toBe("ready");
  });

  it("routes scoped cwd updates to the matching context and rejects unknown workers", () => {
    const current = session("session_1");
    current.contexts.push({ context_id: "context_sub", current_dir: "/work/sub", worker_ids: ["worker_sub"] });
    current.workers.push({ worker_id: "worker_sub", context_id: "context_sub", display_name: "Subtask", ordinal: 1, state: "ready" });
    const update: CoreTopicEvent = {
      ...topic("core.action", { context_state: { cwd: "/work/sub/new" } }),
      context_id: "context_sub",
      worker_id: "worker_sub",
    };
    const updated = applyCoreTopicToSession(current, update, assistantMessage);
    expect(updated.current_dir).toBe("/work");
    expect(updated.contexts.find((context) => context.context_id === "context_sub")?.current_dir).toBe("/work/sub/new");

    const unknown = applyCoreTopicToSession(current, { ...update, worker_id: "worker_unknown" }, assistantMessage);
    expect(unknown).toBe(current);
  });

  it("updates worker lifecycle metadata without replacing the session display name", () => {
    const current = { ...session("session_1"), display_name: "Session0" };
    const lifecycle: CoreTopicEvent = {
      ...topic("core.lifecycle", {
        worker: { display_name: "ID0" },
        max_llm_input_tokens: 128_000,
      }),
      context_id: current.active_context_id,
      worker_id: current.primary_worker_id,
    };
    const updated = applyCoreTopicToSession(current, lifecycle, assistantMessage);
    expect(updated.display_name).toBe("Session0");
    expect(updated.workers[0].display_name).toBe("ID0");
    expect(updated.max_llm_input_tokens).toBe(128_000);
  });

  it("aggregates live task usage across model rounds and preserves the latest call", () => {
    const activeTurn = turn("turn_usage");
    activeTurn.events = [
      { event_id: "usage_1", source: "worker_activity", created_at_ms: 2, payload: { kind: "model_response", usage: { prompt_tokens: 4_000, completion_tokens: 200, cached_tokens: 3_000 } } },
      { event_id: "other", source: "worker_activity", created_at_ms: 3, payload: { kind: "model_request", round: 2 } },
      { event_id: "usage_2", source: "worker_activity", created_at_ms: 4, payload: { kind: "model_response", usage: { prompt_tokens: 5_500, completion_tokens: 350, cached_tokens: 4_500 } } },
    ];

    expect(turnLiveUsage(activeTurn)).toEqual({
      total: { prompt_tokens: 9_500, completion_tokens: 550, cached_tokens: 7_500 },
      latest: { prompt_tokens: 5_500, completion_tokens: 350, cached_tokens: 4_500 },
    });
  });

  it("uses only the selected session's latest real provider usage for context", () => {
    const current = session("session_1");
    const oldTurn = turn("old", "finished");
    oldTurn.completion = { latest_usage: { prompt_tokens: 2_000 } };
    const activeTurn = turn("active");
    activeTurn.events = [{ event_id: "latest", source: "worker_activity", created_at_ms: 3, payload: { kind: "model_response", usage: { prompt_tokens: 8_200, completion_tokens: 40 } } }];
    current.turns = [oldTurn, activeTurn];

    expect(sessionContextUsage(current)?.prompt_tokens).toBe(8_200);
    expect(sessionContextUsage(session("session_2"))).toBeUndefined();
  });

  it("renders response repair as a visible warning", () => {
    const activity = activityFromTopic(topic("core.model.repair", { attempt: 2, max_attempts: 5, issue: "missing_response_root" }));
    expect(activity).toMatchObject({ tone: "warning", title: "Response format repair (2/5)", detail: "missing_response_root" });
  });

  it("renders model free talk verbatim without an invented completion label", () => {
    const activity = activityFromTopic(topic("core.model.response", {
      status: "finished",
      free_talk: "User sent a simple greeting. No tools needed.",
    }));
    expect(activity).toMatchObject({
      tone: "thinking",
      title: "",
      detail: "User sent a simple greeting. No tools needed.",
    });
  });

  it("does not turn work-instruction bookkeeping into user-visible activity", () => {
    expect(activityFromTopic(topic("core.work_instruction_load", {
      status: "loaded",
      file_names: ["AGENTS.md"],
    }))).toBeNull();
  });

  it("keeps context compaction as a typed system activity with token metrics", () => {
    const activity = activityFromTopic(topic("core.context.compact", {
      estimated_before_tokens: 82_000,
      estimated_after_tokens: 14_000,
    }));
    expect(activity).toMatchObject({
      kind: "context_compact",
      tone: "notice",
      before_tokens: 82_000,
      after_tokens: 14_000,
    });
  });

  it("renders run_bash commands as Bash code and keeps the structured status", () => {
    const activity = activityFromTopic(topic("core.action", { action: "run_bash", status: "running", input: { cmd: "git status" } }));
    expect(activity).toMatchObject({ tone: "action", title: "run_bash · running", detail: "", code: "git status", code_language: "bash" });
  });

  it("renders builtin tool usage as a readable invocation", () => {
    const activity = activityFromTopic(topic("core.action", {
      action: "memmgr",
      status: "running",
      input: { type: "durable", op: "sql", sql: "SELECT id, content FROM memories" },
    }));
    expect(activity).toMatchObject({
      tone: "action",
      title: "memmgr · running",
      detail: 'type="durable" op="sql" sql="SELECT id, content FROM memories"',
    });
  });

  it("applies a structured cwd update only to the matching session", () => {
    const cwdUpdate = topic("core.action", {
      action: "self_tool",
      status: "completed",
      context_state: { cwd: "/work/new-root" },
    });
    const matching = applyCoreTopicToSession(session("session_1"), cwdUpdate, assistantMessage);
    const unrelated = applyCoreTopicToSession(session("session_2"), cwdUpdate, assistantMessage);

    expect(matching.current_dir).toBe("/work/new-root");
    expect(unrelated.current_dir).toBe("/work");
  });

  it("turns a waiting request topic into a decision dialog", () => {
    const decision = requestDecision(topic("core.request", { request: { command: "git status" } }, "waiting_user_with_timeout"));
    expect(decision?.detail).toBe("git status");
    expect(requestDecision(topic("core.request", {}, "running"))).toBeNull();
  });

  it("queues concurrent decisions by session and request id without cross-session replacement", () => {
    const first = requestDecision(topic("core.request", { request_id: "req_a", request: { command: "git status" } }, "waiting_user"))!;
    const secondEvent = { ...topic("core.request", { request_id: "req_b", request: { command: "cargo test" } }, "waiting_user"), session_id: "session_2" };
    const second = requestDecision(secondEvent)!;
    const queued = enqueueDecision(enqueueDecision(enqueueDecision([], first), second), first);
    expect(queued).toHaveLength(2);
    expect(queued.map((decision) => decision.event.session_id)).toEqual(["session_1", "session_2"]);
    expect(clearDecisionsForSession(queued, "session_1")).toEqual([second]);
  });

  it("clears only the resumed workers decision within a shared session", () => {
    const primary = requestDecision({ ...topic("core.request", { request_id: "req_primary" }, "waiting_user"), worker_id: "worker_primary" })!;
    const child = requestDecision({ ...topic("core.request", { request_id: "req_child" }, "waiting_user"), worker_id: "worker_child" })!;
    expect(clearDecisionsForWorker([primary, child], "session_1", "worker_primary")).toEqual([child]);
  });

  it("renders a work-instruction decision using its shared structured fields", () => {
    const decision = requestDecision(topic("core.work_instruction_load", {
      request_id: "work_1",
      request: { directory: "/workspace", file_names: ["AGENTS.md", "CLAUDE.md"] },
    }, "waiting_user_with_timeout"));
    expect(decision?.detail).toBe("Load AGENTS.md, CLAUDE.md from /workspace?");
  });

  it("upserts newly created sessions without duplicating lifecycle replays", () => {
    const original = session("session_1");
    const created = { ...session("session_2"), display_name: "Review" };
    expect(upsertSession([original], created)).toEqual([original, created]);
    expect(upsertSession([original, created], { ...created, display_name: "Renamed" })).toEqual([
      original,
      { ...created, display_name: "Renamed" },
    ]);
  });

  it("removes only the selected pending attachment from one session", () => {
    const original = {
      ...session("session_1"),
      attachments: [
        { id: "upload_1", name: "first.md", path: "/tmp/first.md", bytes: 1 },
        { id: "upload_2", name: "second.md", path: "/tmp/second.md", bytes: 2 },
      ],
    };
    expect(removePendingAttachment(original, "upload_1").attachments).toEqual([
      { id: "upload_2", name: "second.md", path: "/tmp/second.md", bytes: 2 },
    ]);
    expect(removePendingAttachment(original, "missing")).toEqual(original);
  });

  it("bounds the browser message window without changing order", () => {
    const input = Array.from({ length: MAX_RENDERED_MESSAGES + 2 }, (_, index) => index);
    const visible = trimMessages(input);
    expect(visible).toHaveLength(MAX_RENDERED_MESSAGES);
    expect(visible[0]).toBe(2);
    expect(visible.at(-1)).toBe(MAX_RENDERED_MESSAGES + 1);
  });

  it("trims a sudden very large snapshot to the fixed render window", () => {
    const input = Array.from({ length: 100_000 }, (_, index) => index);
    const visible = trimMessages(input);
    expect(visible).toHaveLength(MAX_RENDERED_MESSAGES);
    expect(visible[0]).toBe(99_000);
    expect(visible.at(-1)).toBe(99_999);
  });

  it("bounds a reconnect snapshot with many turns and high-frequency events", () => {
    const current = session("session_pressure");
    current.turns = Array.from({ length: MAX_CLIENT_TURNS + 40 }, (_, turnIndex) => ({
      ...turn(`turn_${turnIndex}`, "finished"),
      events: Array.from({ length: MAX_CLIENT_TURN_EVENTS + 50 }, (_, eventIndex) => ({
        event_id: `event_${turnIndex}_${eventIndex}`,
        source: "worker_activity",
        payload: { kind: "progress", marker: `${turnIndex}:${eventIndex}` },
        created_at_ms: eventIndex,
      })),
    }));

    const bounded = boundSessionHistory(current);
    expect(bounded.turns).toHaveLength(MAX_CLIENT_TURNS);
    expect(bounded.turns[0]?.turn_id).toBe("turn_40");
    expect(bounded.turns.every((item) => item.events.length === MAX_CLIENT_TURN_EVENTS)).toBe(true);
    expect(bounded.turns.at(-1)?.events[0]?.payload.marker).toBe(`${MAX_CLIENT_TURNS + 39}:50`);
  });

  it("keeps repeated live event bursts bounded and isolated across sessions", () => {
    let sessions = Array.from({ length: 5 }, (_, index) => upsertTurn(session(`pressure_${index}`), turn(`turn_${index}`)));
    for (let eventIndex = 0; eventIndex < MAX_CLIENT_TURN_EVENTS * 3; eventIndex += 1) {
      const target = eventIndex % sessions.length;
      sessions = sessions.map((current, index) => index === target ? appendTurnEvent(current, `turn_${index}`, {
        event_id: `event_${index}_${eventIndex}`,
        source: "worker_activity",
        payload: { kind: "progress", owner: current.session_id, eventIndex },
        created_at_ms: eventIndex,
      }) : current);
    }

    for (const current of sessions) {
      const events = current.turns[0]?.events ?? [];
      expect(events.length).toBeLessThanOrEqual(MAX_CLIENT_TURN_EVENTS);
      expect(events.every((event) => event.payload.owner === current.session_id)).toBe(true);
    }
  });

  it("bounds newly appended turns without changing chronological order", () => {
    let current = session("turn_pressure");
    for (let index = 0; index < MAX_CLIENT_TURNS + 25; index += 1) current = upsertTurn(current, turn(`turn_${index}`, "finished"));
    expect(current.turns).toHaveLength(MAX_CLIENT_TURNS);
    expect(current.turns[0]?.turn_id).toBe("turn_25");
    expect(current.turns.at(-1)?.turn_id).toBe(`turn_${MAX_CLIENT_TURNS + 24}`);
  });

  it("applies a model response only to the session named by the core topic", () => {
    const response = topic("core.model.response", { final_answer: "agent one result", continue_work: false });
    const sessionOne = applyCoreTopicToSession(session("session_1"), response, assistantMessage);
    const sessionTwo = applyCoreTopicToSession(session("session_2"), response, assistantMessage);

    expect(sessionOne.messages.map((message) => message.text)).toEqual(["agent one result"]);
    expect(sessionOne.state).toBe("ready");
    expect(sessionTwo.messages).toEqual([]);
    expect(sessionTwo.state).toBe("ready");
  });

  it("keeps a matched agent working without changing unrelated sessions", () => {
    const response = { ...topic("core.model.response", { final_answer: "progress", continue_work: true }), session_id: "session_b" };
    const agentA = applyCoreTopicToSession(session("session_a"), response, assistantMessage);
    const agentB = applyCoreTopicToSession(session("session_b"), response, assistantMessage);

    expect(agentA).toEqual(session("session_a"));
    expect(agentB.state).toBe("working");
    expect(agentB.messages[0]?.text).toBe("progress");
  });

  it("attaches completion telemetry only to the matching final answer", () => {
    const response = topic("core.model.response", { final_answer: "done", ui_message_id: "core-msg-1", continue_work: false });
    const matching = applyCoreTopicToSession(session("session_1"), response, (text, id) => ({ ...assistantMessage(text), id: id ?? "missing" }));
    const completed = attachTurnCompletion(matching, "core-msg-1", { elapsed_ms: 1800, stats: { prompt_tokens: 1200, completion_tokens: 34 } });
    const unrelated = attachTurnCompletion(session("session_2"), "core-msg-1", { elapsed_ms: 1 });

    expect(completed.messages[0]?.completion).toMatchObject({ elapsed_ms: 1800, stats: { prompt_tokens: 1200 } });
    expect(unrelated.messages).toEqual([]);
  });

  it("keeps one turn envelope for task, supplement, process, and final telemetry", () => {
    const active = upsertTurn(session("session_1"), turn("turn_1"));
    const response = topic("core.model.response", {
      status: "finished",
      free_talk: "checked the workspace",
      final_answer: "## Delivered\nDone.",
      continue_work: false,
    });
    const withResponse = appendTurnEvent(active, "turn_1", {
      event_id: "event_1",
      source: "core_topic",
      payload: response as unknown as Record<string, unknown>,
      created_at_ms: 2,
    });
    const finished = finishTurn(withResponse, "turn_1", {
      elapsed_ms: 2300,
      stats: { prompt_tokens: 4200, completion_tokens: 180 },
    });

    expect(finished.turns).toHaveLength(1);
    expect(finished.turns[0]).toMatchObject({
      turn_id: "turn_1",
      state: "finished",
      final_answer: "## Delivered\nDone.",
      completion: { elapsed_ms: 2300, stats: { prompt_tokens: 4200 } },
    });
    expect(finished.active_turn_id).toBeNull();
    expect(finished.state).toBe("ready");
    expect(finished.workers[0]?.state).toBe("ready");
  });

  it("clears stale primary working state when a cancelled turn finishes without a model response", () => {
    const active = upsertTurn(session("session_1"), turn("turn_cancelled"));
    const working = updateSessionWorkerState(active, active.primary_worker_id, "working");

    const finished = finishTurn(working, "turn_cancelled", {
      elapsed_ms: 519_000,
      stop_reason: "CancelledByUser",
    });

    expect(finished.active_turn_id).toBeNull();
    expect(finished.state).toBe("ready");
    expect(finished.workers.find((worker) => worker.worker_id === finished.primary_worker_id)?.state).toBe("ready");
  });

  it("deduplicates replayed turn events by the host event id", () => {
    const active = upsertTurn(session("session_1"), turn("turn_1"));
    const event = { event_id: "stable_event", source: "worker_activity", payload: { kind: "model_retry" }, created_at_ms: 2 };
    const once = appendTurnEvent(active, "turn_1", event);
    const replayed = appendTurnEvent(once, "turn_1", event);
    expect(replayed.turns[0].events).toEqual([event]);
  });

  it("does not apply a turn event to another session or another turn", () => {
    const first = upsertTurn(session("session_1"), turn("turn_1"));
    const event = { event_id: "event_x", source: "worker_activity", payload: { kind: "model_retry" }, created_at_ms: 2 };
    expect(appendTurnEvent(first, "turn_2", event)).toEqual(first);
    expect(appendTurnEvent(session("session_2"), "turn_1", event).turns).toEqual([]);
  });
});
