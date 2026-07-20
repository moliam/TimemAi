import { describe, expect, it } from "vitest";
import { ChatHistoryRecord, ChatMessage, CoreTopicEvent, Session, WebTurn, WebTurnEvent } from "../src/protocol";
import { activityFromTopic, appendTurnEvent, applyCoreTopicToSession, attachTurnCompletion, boundSessionHistory, clearDecisionsForSession, clearDecisionsForWorker, coalesceActionLifecycle, composerSendDecision, decisionKey, draftForSession, enqueueDecision, finishDraftSubmission, finishSessionDraftSubmission, finishTurn, manualToolGenCommand, MAX_CLIENT_TURN_EVENTS, MAX_CLIENT_TURNS, MAX_RENDERED_MESSAGES, prependHistoryRecords, pruneSessionDrafts, pruneSessionSubmissionLocks, releaseSessionDraftSubmission, removePendingAttachment, requestDecision, reserveDraftSubmission, reserveSessionDraftSubmission, resolveActiveSessionId, sessionContextUsage, sessionCreateDecision, sessionRenameDecision, setSessionDraft, tailPath, trimMessages, turnLiveUsage, turnsFromHistoryRecords, updateSessionWorkerState, upsertSession, upsertTurn } from "../src/view_model";

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
  tools: [],
  contexts: [{ context_id: `context_${sessionId}`, current_dir: "/work", worker_ids: [`worker_${sessionId}`] }],
  workers: [{ worker_id: `worker_${sessionId}`, context_id: `context_${sessionId}`, display_name: sessionId, ordinal: 0, state: "ready", parent_worker_id: null }],
  active_context_id: `context_${sessionId}`,
  primary_worker_id: `worker_${sessionId}`,
  attachments: [],
  messages: [],
  turns: [],
  history_before_cursor: null,
  history_has_more: false,
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
  actionId?: string,
): WebTurnEvent => ({
  event_id: id,
  source: "core_topic",
  created_at_ms: Number(id.replace(/\D/g, "")) || 1,
  payload: {
    session_id: "session_1",
    topic: { name: "core.action", attributes: { event: lifecycle, ...(actionId ? { action_id: actionId } : {}) } },
    state: { name: "running" },
    payload: { action: "run_bash", input, event: lifecycle, status, ...(actionId ? { action_id: actionId } : {}) },
  },
});

describe("web topic view model", () => {
  it("renders ToolGen lifecycle as one compact system activity", () => {
    const started = activityFromTopic(topic("core.toolgen", { phase: "started", tool_count: 2 }));
    expect(started).toMatchObject({ tone: "notice", kind: "toolgen", title: "ToolGen: 正在评估…" });
    const published = activityFromTopic(topic("core.toolgen", { phase: "published", tool_count: 3, tool: { name: "trace-summarizer" }, retrospect: "Created and validated." }, "ready"));
    expect(published).toMatchObject({ tone: "notice", kind: "toolgen", title: "ToolGen: 已生成并验证 trace-summarizer", detail: "Created and validated." });
    const failed = activityFromTopic(topic("core.toolgen", { phase: "failed", error: "self-test failed" }, "ready"));
    expect(failed).toMatchObject({ tone: "warning", kind: "toolgen", title: "ToolGen: 生成失败", detail: "self-test failed" });
    expect(activityFromTopic(topic("core.model.response", { runtime_phase: "toolgen", free_talk: "Building a reusable parser.", final_answer: "internal completion" }))).toMatchObject({
      tone: "thinking",
      detail: "Building a reusable parser.",
    });
    expect(activityFromTopic(topic("core.action", { runtime_phase: "toolgen", action: "run_bash", status: "running", input: { cmd: "bash tool.sh --self-test" } }))).toMatchObject({
      tone: "action",
      code: "bash tool.sh --self-test",
      code_language: "bash",
    });
    expect(activityFromTopic(topic("core.model.repair", { runtime_phase: "toolgen", attempt: 1, max_attempts: 5, issue: "invalid_xml" }))).toMatchObject({
      tone: "warning",
      title: "⚠️ 模型回复偏离协议，重试 (1/5)",
    });
  });

  it("submits a new user turn when the active session is ready", () => {
    const current = session("session_1");
    expect(composerSendDecision(current, "  start task  ", false)).toEqual({
      kind: "send",
      text: "start task",
      clearDraftOnSuccess: true,
      command: { type: "turn_submit", session_id: "session_1", text: "start task" },
    });
  });

  it("sends working-session text as a supplement instead of disabling input", () => {
    const current = { ...session("session_1"), state: "working" };
    expect(composerSendDecision(current, "  add this constraint  ", false)).toEqual({
      kind: "send",
      text: "add this constraint",
      clearDraftOnSuccess: true,
      command: { type: "turn_supplement", session_id: "session_1", text: "add this constraint" },
    });
  });

  it("keeps rapid repeated sends during a working turn as separate supplements", () => {
    const current = { ...session("session_1"), state: "working" };
    const decisions = ["first correction", "second correction", "third correction"].map((text) => composerSendDecision(current, text, false));
    expect(decisions.map((decision) => decision.kind)).toEqual(["send", "send", "send"]);
    expect(decisions.map((decision) => decision.kind === "send" ? decision.command : undefined)).toEqual([
      { type: "turn_supplement", session_id: "session_1", text: "first correction" },
      { type: "turn_supplement", session_id: "session_1", text: "second correction" },
      { type: "turn_supplement", session_id: "session_1", text: "third correction" },
    ]);
  });

  it("guards one browser draft submission while preserving text typed during the pending send", () => {
    const lock = { current: false };
    const submitted = reserveDraftSubmission(lock, "  first message  ");
    expect(submitted).toBe("first message");
    expect(lock.current).toBe(true);
    expect(reserveDraftSubmission(lock, "double click")).toBeNull();

    const draftAfterTypingDuringSend = finishDraftSubmission(lock, "second message typed while sending", submitted, true);
    expect(draftAfterTypingDuringSend).toBe("second message typed while sending");
    expect(lock.current).toBe(false);

    const retried = reserveDraftSubmission(lock, draftAfterTypingDuringSend);
    expect(retried).toBe("second message typed while sending");
  });

  it("keeps the original draft when the transport send fails", () => {
    const lock = { current: false };
    const submitted = reserveDraftSubmission(lock, "retry me");
    expect(finishDraftSubmission(lock, "retry me", submitted, false)).toBe("retry me");
    expect(lock.current).toBe(false);
  });

  it("releases a session send guard when the authoritative turn-finished event arrives", () => {
    const locks = { current: new Set(["session_1", "session_2"]) };
    expect(releaseSessionDraftSubmission(locks, "session_1")).toBe(true);
    expect(locks.current).toEqual(new Set(["session_2"]));
    expect(releaseSessionDraftSubmission(locks, "session_1")).toBe(false);
  });

  it("keeps drafts and pending send guards isolated by session", () => {
    let drafts: Record<string, string> = {};
    drafts = setSessionDraft(drafts, "session_a", "draft for A");
    drafts = setSessionDraft(drafts, "session_b", "draft for B");
    expect(draftForSession(drafts, "session_a")).toBe("draft for A");
    expect(draftForSession(drafts, "session_b")).toBe("draft for B");

    const locks = { current: new Set<string>() };
    const submittedA = reserveSessionDraftSubmission(locks, "session_a", drafts);
    expect(submittedA).toEqual({ sessionId: "session_a", text: "draft for A" });
    expect(reserveSessionDraftSubmission(locks, "session_a", drafts)).toBeNull();
    expect(reserveSessionDraftSubmission(locks, "session_b", drafts)).toEqual({ sessionId: "session_b", text: "draft for B" });
  });

  it("does not erase another session draft or text typed after a pending send", () => {
    let drafts = {
      session_a: "first A",
      session_b: "keep B",
    };
    const locks = { current: new Set<string>() };
    const submittedA = reserveSessionDraftSubmission(locks, "session_a", drafts);
    expect(submittedA?.text).toBe("first A");

    drafts = setSessionDraft(drafts, "session_a", "second A typed while first A sends");
    drafts = finishSessionDraftSubmission(locks, drafts, "session_a", submittedA!.text, true);
    expect(draftForSession(drafts, "session_a")).toBe("second A typed while first A sends");
    expect(draftForSession(drafts, "session_b")).toBe("keep B");

    const submittedB = reserveSessionDraftSubmission(locks, "session_b", drafts);
    drafts = finishSessionDraftSubmission(locks, drafts, "session_b", submittedB!.text, true);
    expect(draftForSession(drafts, "session_b")).toBe("");
  });

  it("prunes stale drafts and pending send locks when a snapshot swaps out sessions", () => {
    const drafts = {
      session_a: "old mem draft",
      session_b: "live draft",
      session_c: "removed session draft",
    };
    const locks = { current: new Set(["session_a", "session_b", "session_c"]) };

    const liveSessions = ["session_b", "session_d"];
    expect(pruneSessionDrafts(drafts, liveSessions)).toEqual({ session_b: "live draft" });
    expect(pruneSessionSubmissionLocks(locks, liveSessions)).toBe(true);
    expect(Array.from(locks.current)).toEqual(["session_b"]);
    expect(pruneSessionSubmissionLocks(locks, liveSessions)).toBe(false);
  });

  it("recovers from an in-flight old-mem send after a mem snapshot swaps sessions", () => {
    let drafts = { old_session: "old mem pending text" };
    const locks = { current: new Set<string>() };
    const submitted = reserveSessionDraftSubmission(locks, "old_session", drafts);
    expect(submitted).toEqual({ sessionId: "old_session", text: "old mem pending text" });

    const liveSessions = ["new_session"];
    drafts = pruneSessionDrafts(drafts, liveSessions);
    expect(pruneSessionSubmissionLocks(locks, liveSessions)).toBe(true);
    expect(drafts).toEqual({});
    expect(Array.from(locks.current)).toEqual([]);

    const activeSessionId = resolveActiveSessionId("old_session", [session("new_session")]);
    drafts = setSessionDraft(drafts, activeSessionId, "fresh task in new mem");
    const reserved = reserveSessionDraftSubmission(locks, activeSessionId, drafts);
    expect(reserved).toEqual({ sessionId: "new_session", text: "fresh task in new mem" });

    const decision = composerSendDecision(session(activeSessionId), reserved!.text, false);
    expect(decision).toEqual({
      kind: "send",
      text: "fresh task in new mem",
      clearDraftOnSuccess: true,
      command: { type: "turn_submit", session_id: "new_session", text: "fresh task in new mem" },
    });
  });

  it("keeps draft state identity stable when every draft belongs to a live session", () => {
    const drafts = { session_a: "draft A", session_b: "draft B" };
    expect(pruneSessionDrafts(drafts, ["session_a", "session_b"])).toBe(drafts);
  });

  it("moves the active session to a live session when a snapshot swaps out the old one", () => {
    expect(resolveActiveSessionId("session_a", [session("session_a"), session("session_b")])).toBe("session_a");
    expect(resolveActiveSessionId("session_old", [session("session_new"), session("session_other")])).toBe("session_new");
    expect(resolveActiveSessionId("session_old", [])).toBe("");
  });

  it("does not send while cancellation is still in flight", () => {
    const current = { ...session("session_1"), state: "working" };
    expect(composerSendDecision(current, "do not race stop", true)).toEqual({ kind: "skip", reason: "cancelling" });
  });

  it("keeps draft text and releases the pending guard when cancellation blocks a reserved send", () => {
    let drafts = { session_1: "human clicked send while stop is pending" };
    const locks = { current: new Set<string>() };
    const reserved = reserveSessionDraftSubmission(locks, "session_1", drafts);
    expect(reserved).toEqual({ sessionId: "session_1", text: "human clicked send while stop is pending" });

    const decision = composerSendDecision({ ...session("session_1"), state: "working" }, reserved!.text, true);
    expect(decision).toEqual({ kind: "skip", reason: "cancelling" });

    drafts = finishSessionDraftSubmission(locks, drafts, reserved!.sessionId, reserved!.text, false);
    expect(draftForSession(drafts, "session_1")).toBe("human clicked send while stop is pending");
    expect(Array.from(locks.current)).toEqual([]);

    const retryAfterCancelSettles = reserveSessionDraftSubmission(locks, "session_1", drafts);
    expect(retryAfterCancelSettles).toEqual({ sessionId: "session_1", text: "human clicked send while stop is pending" });
  });

  it("sends a new task after a cancelled active turn is marked finished", () => {
    const active = upsertTurn(session("session_1"), turn("turn_cancelled"));
    const working = updateSessionWorkerState(active, active.primary_worker_id, "working");
    const finished = finishTurn(working, "turn_cancelled", {
      elapsed_ms: 42_000,
      stop_reason: "CancelledByUser",
    });

    expect(composerSendDecision(finished, "resume as a fresh task", false)).toEqual({
      kind: "send",
      text: "resume as a fresh task",
      clearDraftOnSuccess: true,
      command: { type: "turn_submit", session_id: "session_1", text: "resume as a fresh task" },
    });
  });

  it("does not send new tasks or supplements while a mem switch is pending", () => {
    expect(composerSendDecision(session("session_1"), "new task", false, true)).toEqual({ kind: "skip", reason: "mem_switching" });
    expect(composerSendDecision({ ...session("session_1"), state: "working" }, "late supplement", false, true)).toEqual({ kind: "skip", reason: "mem_switching" });
  });

  it("does not rename a session while mem switching or another rename is pending", () => {
    expect(sessionRenameDecision("session_1", "Renamed", new Set(), true)).toEqual({ kind: "skip", reason: "mem_switching" });
    expect(sessionRenameDecision("session_1", "Renamed", new Set(["session_1"]))).toEqual({ kind: "skip", reason: "already_pending" });
    expect(sessionRenameDecision("session_1", "   ", new Set())).toEqual({ kind: "skip", reason: "empty_name" });
    expect(sessionRenameDecision(undefined, "Renamed", new Set())).toEqual({ kind: "skip", reason: "no_session" });
  });

  it("builds a single session rename command from the trimmed display name", () => {
    expect(sessionRenameDecision("session_1", "  Research Agent  ", new Set())).toEqual({
      kind: "send",
      displayName: "Research Agent",
      command: { type: "session_rename", session_id: "session_1", display_name: "Research Agent" },
    });
  });

  it("builds a session create command from cleaned form input", () => {
    expect(sessionCreateDecision("  Research  ", "  /work/project  ", {
      TIMEM_MODEL: " qwen-plus ",
      TIMEM_API_KEY: "   ",
      TIMEM_STREAM: " true ",
    }, false)).toEqual({
      kind: "send",
      displayName: "Research",
      workspaceDir: "/work/project",
      env: { TIMEM_MODEL: "qwen-plus", TIMEM_STREAM: "true" },
      command: {
        type: "session_create",
        display_name: "Research",
        workspace_dir: "/work/project",
        env: { TIMEM_MODEL: "qwen-plus", TIMEM_STREAM: "true" },
      },
    });
    expect(sessionCreateDecision("   ", "/work/project", {}, false)).toMatchObject({
      kind: "send",
      command: { type: "session_create", workspace_dir: "/work/project", env: {} },
    });
  });

  it("blocks session creation while creating, mem switching, or missing a workspace", () => {
    expect(sessionCreateDecision("name", "   ", {}, false)).toEqual({ kind: "skip", reason: "empty_workspace" });
    expect(sessionCreateDecision("name", "/work", {}, true)).toEqual({ kind: "skip", reason: "creating" });
    expect(sessionCreateDecision("name", "/work", {}, false, true)).toEqual({ kind: "skip", reason: "mem_switching" });
  });

  it("skips empty text and missing sessions before touching the socket", () => {
    expect(composerSendDecision(session("session_1"), "   \n\t", false)).toEqual({ kind: "skip", reason: "empty_text" });
    expect(composerSendDecision(undefined, "hello", false)).toEqual({ kind: "skip", reason: "no_session" });
  });

  it("treats stopped or error sessions as explicit new submit attempts for the host to validate", () => {
    expect(composerSendDecision({ ...session("session_1"), state: "error" }, "recover", false)).toMatchObject({
      kind: "send",
      command: { type: "turn_submit", session_id: "session_1", text: "recover" },
    });
  });

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

  it("does not append a topic event to another session even if turn ids collide", () => {
    const target = { ...session("session_1"), turns: [turn("turn_shared")] };
    const other = { ...session("session_2"), turns: [turn("turn_shared")] };
    const event = actionEvent("event_1", "start", "running");
    expect(appendTurnEvent(target, "turn_shared", event).turns[0].events).toHaveLength(1);
    expect(appendTurnEvent(other, "turn_shared", event).turns[0].events).toHaveLength(0);
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

  it("uses structured action ids to keep out-of-order parallel action completion aligned", () => {
    const events = coalesceActionLifecycle([
      actionEvent("event_1", "start", "running", { cmd: "same command" }, "action_a"),
      actionEvent("event_2", "start", "running", { cmd: "same command" }, "action_b"),
      actionEvent("event_3", "finish", "timeout", { cmd: "same command" }, "action_b"),
      actionEvent("event_4", "finish", "completed", { cmd: "same command" }, "action_a"),
    ]);
    expect(events).toHaveLength(2);
    expect(events.map((event) => (event.payload.payload as Record<string, unknown>).action_id)).toEqual(["action_a", "action_b"]);
    expect(events.map((event) => (event.payload.payload as Record<string, unknown>).status)).toEqual(["completed", "timeout"]);
  });

  it("pairs action lifecycle events even when input object key order changes", () => {
    const events = coalesceActionLifecycle([
      actionEvent("event_1", "start", "running", { timeout_ms: 5000, cmd: "git status" }),
      actionEvent("event_2", "finish", "completed", { cmd: "git status", timeout_ms: 5000 }),
    ]);
    expect(events).toHaveLength(1);
    expect((events[0].payload.payload as Record<string, unknown>).status).toBe("completed");
  });

  it("pairs action lifecycle events when nested input object key order changes", () => {
    const events = coalesceActionLifecycle([
      actionEvent("event_1", "start", "running", {
        cmd: "python3 analyze.py",
        options: { output: "summary.json", filters: { warning: true, error: true } },
      }),
      actionEvent("event_2", "finish", "completed", {
        options: { filters: { error: true, warning: true }, output: "summary.json" },
        cmd: "python3 analyze.py",
      }),
    ]);
    expect(events).toHaveLength(1);
    expect((events[0].payload.payload as Record<string, unknown>).status).toBe("completed");
  });

  it("keeps a background action visibly active after its launch event finishes", () => {
    const events = coalesceActionLifecycle([
      actionEvent("event_1", "start", "running", { cmd: "cargo test", background: true }),
      actionEvent("event_2", "finish", "background_running", { cmd: "cargo test", background: true }),
    ]);
    expect(events).toHaveLength(1);
    expect((events[0].payload.payload as Record<string, unknown>).status).toBe("background_running");
  });

  it("replaces the ToolGen start row with one terminal failure row", () => {
    const toolgenEvent = (id: string, phase: string): WebTurnEvent => ({
      event_id: id,
      source: "core_topic",
      created_at_ms: 1,
      payload: {
        session_id: "session_1",
        context_id: "context_1",
        topic: { name: "core.toolgen" },
        state: { name: "running" },
        payload: { phase, error: phase === "failed" ? "toolgen_no_verified_tool" : null },
      },
    });
    const events = coalesceActionLifecycle([
      toolgenEvent("toolgen_started", "started"),
      toolgenEvent("toolgen_failed", "failed"),
    ]);
    expect(events).toHaveLength(1);
    expect((events[0].payload.payload as Record<string, unknown>).phase).toBe("failed");
    expect(coalesceActionLifecycle([toolgenEvent("toolgen_started", "started")])).toHaveLength(0);
  });

  it("reconstructs turns from stored chat history records", () => {
    const records: ChatHistoryRecord[] = [
      { type: "message", role: "user", turn_id: "turn_1", created_at_ms: 1, content: "old task" },
      { type: "event", role: "system", turn_id: "turn_1", created_at_ms: 2, kind: "action", content: "ran bash", source: "core_topic", payload: { topic: { name: "core.action" }, payload: { action: "run_bash" } } },
      { type: "message", role: "assistant", turn_id: "turn_1", created_at_ms: 3, content: "old answer" },
    ];
    const turns = turnsFromHistoryRecords(records);
    expect(turns).toHaveLength(1);
    expect(turns[0].user_entries[0].text).toBe("old task");
    expect(turns[0].events[0].source).toBe("core_topic");
    expect(turns[0].final_answer).toBe("old answer");
  });

  it("preserves the ToolGen topic marker when restoring historical work events", () => {
    const turns = turnsFromHistoryRecords([
      { type: "event", role: "system", turn_id: "toolgen_turn_1", created_at_ms: 1, kind: "toolgen", content: "published", payload: { topic: { name: "core.toolgen" }, payload: { phase: "published" } } },
      { type: "message", role: "assistant", turn_id: "toolgen_turn_1", created_at_ms: 2, content: "tool generated" },
    ]);
    expect(turns[0].events[0].source).toBe("history");
    expect((turns[0].events[0].payload.topic as { name: string }).name).toBe("core.toolgen");
  });

  it("restores task, supplement, and approval user entries inside one turn", () => {
    const records: ChatHistoryRecord[] = [
      { type: "message", role: "user", turn_id: "turn_1", created_at_ms: 1, kind: "task", content: "original task" },
      { type: "message", role: "user", turn_id: "turn_1", created_at_ms: 2, kind: "supplement", content: "mid-turn correction" },
      { type: "message", role: "user", turn_id: "turn_1", created_at_ms: 3, kind: "approval", content: "approved run_bash" },
      { type: "message", role: "assistant", turn_id: "turn_1", created_at_ms: 4, content: "done" },
    ];

    const turns = turnsFromHistoryRecords(records);
    expect(turns).toHaveLength(1);
    expect(turns[0].user_entries).toEqual([
      { kind: "task", text: "original task", attachments: [], created_at_ms: 1 },
      { kind: "supplement", text: "mid-turn correction", attachments: [], created_at_ms: 2 },
      { kind: "approval", text: "approved run_bash", attachments: [], created_at_ms: 3 },
    ]);
    expect(turns[0].final_answer).toBe("done");
  });

  it("restores the last assistant message as the turn final answer while preserving chat order", () => {
    const records: ChatHistoryRecord[] = [
      { type: "message", role: "user", turn_id: "turn_1", created_at_ms: 1, kind: "task", content: "analyze this" },
      { type: "message", role: "assistant", turn_id: "turn_1", created_at_ms: 2, content: "partial answer" },
      { type: "message", role: "assistant", turn_id: "turn_1", created_at_ms: 3, content: "final answer" },
    ];
    const turns = turnsFromHistoryRecords(records);
    expect(turns).toHaveLength(1);
    expect(turns[0].final_answer).toBe("final answer");

    const restored = prependHistoryRecords(session("session_1"), records);
    expect(restored.messages.map((message) => `${message.role}:${message.text}`)).toEqual([
      "user:analyze this",
      "assistant:partial answer",
      "assistant:final answer",
    ]);
  });

  it("sorts restored entries and events within one turn by creation time", () => {
    const records: ChatHistoryRecord[] = [
      { type: "message", role: "user", turn_id: "turn_1", created_at_ms: 30, kind: "approval", content: "approved late" },
      { type: "event", role: "system", turn_id: "turn_1", created_at_ms: 20, kind: "action_result", content: "second event", source: "history", payload: { marker: "event-2" } },
      { type: "message", role: "user", turn_id: "turn_1", created_at_ms: 10, kind: "task", content: "first task" },
      { type: "event", role: "system", turn_id: "turn_1", created_at_ms: 15, kind: "action", content: "first event", source: "history", payload: { marker: "event-1" } },
      { type: "message", role: "user", turn_id: "turn_1", created_at_ms: 25, kind: "supplement", content: "middle supplement" },
    ];

    const turns = turnsFromHistoryRecords(records);
    expect(turns[0].user_entries.map((entry) => entry.text)).toEqual([
      "first task",
      "middle supplement",
      "approved late",
    ]);
    expect(turns[0].events.map((event) => event.payload.marker)).toEqual(["event-1", "event-2"]);
  });

  it("falls back to task for unknown historical user entry kinds", () => {
    const turns = turnsFromHistoryRecords([
      { type: "message", role: "user", turn_id: "turn_1", created_at_ms: 1, kind: "legacy_custom", content: "legacy text" },
    ]);
    expect(turns[0].user_entries[0]).toMatchObject({ kind: "task", text: "legacy text" });
  });

  it("prepends older history without duplicating existing turns", () => {
    const current = {
      ...session("session_1"),
      turns: [turn("turn_2", "finished")],
      messages: [assistantMessage("current answer")],
    };
    const records: ChatHistoryRecord[] = [
      { type: "message", role: "assistant", turn_id: "turn_1", created_at_ms: 2, content: "older answer" },
      { type: "message", role: "user", turn_id: "turn_1", created_at_ms: 1, content: "older" },
      { type: "message", role: "user", turn_id: "turn_2", created_at_ms: 3, content: "duplicate current" },
    ];
    const updated = prependHistoryRecords(current, records);
    expect(updated.turns.map((item) => item.turn_id)).toEqual(["turn_1", "turn_2"]);
    expect(updated.turns[0].final_answer).toBe("older answer");
    expect(updated.messages.map((message) => message.text)).toEqual([
      "older",
      "older answer",
      "current answer",
    ]);
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

  it("rejects core topics scoped to an unknown context before mutating a session", () => {
    const current = session("session_1");
    const unknownContextResponse: CoreTopicEvent = {
      ...topic("core.model.response", { continue_work: false, final_answer: "wrong context answer" }),
      context_id: "context_missing",
    };
    const afterResponse = applyCoreTopicToSession(current, unknownContextResponse, assistantMessage);
    expect(afterResponse).toBe(current);
    expect(afterResponse.messages).toEqual([]);

    const unknownContextCwd: CoreTopicEvent = {
      ...topic("core.action", { context_state: { cwd: "/wrong/context" } }),
      context_id: "context_missing",
    };
    const afterCwd = applyCoreTopicToSession(current, unknownContextCwd, assistantMessage);
    expect(afterCwd).toBe(current);
    expect(afterCwd.current_dir).toBe("/work");
  });

  it("accepts lifecycle topics that introduce a new scoped worker and context", () => {
    const current = { ...session("session_1"), display_name: "Session0" };
    const lifecycle: CoreTopicEvent = {
      ...topic("core.lifecycle", {
        worker: {
          display_name: "ID1",
          ordinal: 1,
          parent_worker_id: current.primary_worker_id,
        },
        context_state: { cwd: "/work/subtask" },
        max_llm_input_tokens: 128_000,
      }),
      context_id: "context_subtask",
      worker_id: "worker_subtask",
    };

    const updated = applyCoreTopicToSession(current, lifecycle, assistantMessage);
    expect(updated.display_name).toBe("Session0");
    expect(updated.contexts.find((context) => context.context_id === "context_subtask")).toEqual({
      context_id: "context_subtask",
      current_dir: "/work/subtask",
      worker_ids: ["worker_subtask"],
    });
    expect(updated.workers.find((worker) => worker.worker_id === "worker_subtask")).toEqual({
      worker_id: "worker_subtask",
      context_id: "context_subtask",
      display_name: "ID1",
      ordinal: 1,
      state: "ready",
      parent_worker_id: current.primary_worker_id,
    });
    expect(updated.max_llm_input_tokens).toBe(128_000);
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

  it("shows ToolGen model usage as the latest usage in its active work frame", () => {
    const activeTurn = turn("turn_toolgen_usage");
    activeTurn.events = [
      { event_id: "main", source: "worker_activity", created_at_ms: 2, payload: { kind: "model_response", usage: { prompt_tokens: 8_200, completion_tokens: 120 } } },
      { event_id: "toolgen", source: "worker_activity", created_at_ms: 3, payload: { kind: "model_response", runtime_phase: "toolgen", usage: { prompt_tokens: 3_100, completion_tokens: 80 } } },
    ];

    expect(turnLiveUsage(activeTurn)).toEqual({
      total: { prompt_tokens: 11_300, completion_tokens: 200 },
      latest: { prompt_tokens: 3_100, completion_tokens: 80 },
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

  it("does not treat restored history telemetry as current context usage", () => {
    const restored = session("session_restored");
    const historicalTurn = turn("old", "restored");
    historicalTurn.events = [
      { event_id: "history_usage", source: "worker_activity", created_at_ms: 2, payload: { kind: "model_response", usage: { prompt_tokens: 26_000, completion_tokens: 500 } } },
    ];
    historicalTurn.completion = { latest_usage: { prompt_tokens: 26_000 } };
    restored.turns = [historicalTurn];

    expect(sessionContextUsage(restored)).toBeUndefined();

    const liveTurn = turn("new", "working");
    liveTurn.events = [
      { event_id: "new_usage", source: "worker_activity", created_at_ms: 3, payload: { kind: "model_response", usage: { prompt_tokens: 4_200, completion_tokens: 20 } } },
    ];
    restored.turns.push(liveTurn);

    expect(sessionContextUsage(restored)?.prompt_tokens).toBe(4_200);
  });

  it("renders response repair as a visible warning", () => {
    const activity = activityFromTopic(topic("core.model.repair", { attempt: 2, max_attempts: 5, issue: "missing_response_root" }));
    expect(activity).toMatchObject({ tone: "warning", title: "⚠️ 模型回复偏离协议，重试 (2/5)", detail: "missing_response_root" });
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

  it("renders model progress even when free talk is omitted", () => {
    const activity = activityFromTopic(topic("core.model.response", {
      status: "working",
      progress: "正在检查日志并提取关键错误。",
    }));
    expect(activity).toMatchObject({
      tone: "thinking",
      title: "",
      detail: "正在检查日志并提取关键错误。",
    });
  });

  it("keeps free talk before progress for one model response topic", () => {
    const activity = activityFromTopic(topic("core.model.response", {
      free_talk: "先判断需要哪些证据。",
      progress: "正在读取本地文件。",
    }));
    expect(activity?.detail).toBe("先判断需要哪些证据。\n\n正在读取本地文件。");
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
    expect(activity).toMatchObject({ tone: "action", title: "Bash · running", tool_name: "run_bash", detail: "", code: "git status", code_language: "bash" });
  });

  it("shows human-readable action statuses while preserving structured tool status", () => {
    const background = activityFromTopic(topic("core.action", { action: "run_bash", status: "background_running", input: { cmd: "cargo test" } }));
    expect(background).toMatchObject({ title: "Bash · background running", tool_status: "background_running" });

    const timeout = activityFromTopic(topic("core.action", { action: "run_bash", status: "timeout", input: { cmd: "sleep 30" } }));
    expect(timeout).toMatchObject({ title: "Bash · timed out", tool_status: "timeout" });
  });

  it("renders builtin tool usage as a readable invocation", () => {
    const activity = activityFromTopic(topic("core.action", {
      action: "memmgr",
      status: "running",
      input: { type: "durable", op: "sql", sql: "SELECT id, content FROM memories" },
    }));
    expect(activity).toMatchObject({
      tone: "action",
      title: "MemMgr · running",
      tool_name: "memmgr",
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

  it("queues concurrent decisions from different workers in the same session", () => {
    const primary = requestDecision({ ...topic("core.request", { request_id: "req_shared" }, "waiting_user"), context_id: "context_primary", worker_id: "worker_primary" })!;
    const child = requestDecision({ ...topic("core.request", { request_id: "req_shared" }, "waiting_user"), context_id: "context_child", worker_id: "worker_child" })!;
    const queued = enqueueDecision(enqueueDecision(enqueueDecision([], primary), child), primary);
    expect(queued).toHaveLength(2);
    expect(queued.map((decision) => decision.event.worker_id)).toEqual(["worker_primary", "worker_child"]);
    expect(decisionKey(primary)).not.toBe(decisionKey(child));
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

  it("keeps a human click storm bounded and session scoped", () => {
    let sessions = Array.from({ length: 5 }, (_, index) => {
      const active = upsertTurn(session(`storm_${index}`), turn(`turn_${index}`));
      return updateSessionWorkerState(active, active.primary_worker_id, "working");
    });
    const acceptedSupplements = new Map<string, string[]>();

    for (let index = 0; index < 600; index += 1) {
      const targetIndex = index % sessions.length;
      const target = sessions[targetIndex];
      const isCancelling = index % 17 === 0;
      const text = `rapid user input ${index}`;
      const decision = composerSendDecision(target, text, isCancelling);
      if (isCancelling) {
        expect(decision).toEqual({ kind: "skip", reason: "cancelling" });
      } else {
        expect(decision).toMatchObject({
          kind: "send",
          command: { type: "turn_supplement", session_id: target.session_id, text },
        });
        acceptedSupplements.set(target.session_id, [
          ...(acceptedSupplements.get(target.session_id) ?? []),
          text,
        ]);
      }
      sessions = sessions.map((current, sessionIndex) => sessionIndex === targetIndex ? appendTurnEvent(current, current.active_turn_id, {
        event_id: `storm_event_${index}`,
        source: "worker_activity",
        payload: { kind: "progress", owner: current.session_id, index },
        created_at_ms: index,
      }) : current);
    }

    for (const current of sessions) {
      const events = current.turns[0]?.events ?? [];
      expect(events.length).toBeLessThanOrEqual(MAX_CLIENT_TURN_EVENTS);
      expect(events.every((event) => event.payload.owner === current.session_id)).toBe(true);
      expect(current.state).toBe("working");
      expect(current.workers.every((worker) => worker.state === "working")).toBe(true);
      expect(acceptedSupplements.get(current.session_id)?.length).toBeGreaterThan(80);
      const finished = finishTurn(current, current.active_turn_id, { elapsed_ms: 42_000, stop_reason: "CancelledByUser" });
      expect(finished.state).toBe("ready");
      expect(finished.active_turn_id).toBeNull();
      expect(finished.workers.every((worker) => worker.state === "ready")).toBe(true);
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

  it("does not append a core topic event to another session with the same turn id", () => {
    const sharedTurnId = "turn_shared";
    const event: WebTurnEvent = {
      event_id: "event_session_1",
      source: "core_topic",
      payload: topic("core.model.response", { final_answer: "only session one", continue_work: false }) as unknown as Record<string, unknown>,
      created_at_ms: 2,
    };

    const sessionOne = appendTurnEvent(upsertTurn(session("session_1"), turn(sharedTurnId)), sharedTurnId, event);
    const sessionTwo = appendTurnEvent(upsertTurn(session("session_2"), turn(sharedTurnId)), sharedTurnId, event);

    expect(sessionOne.turns[0]?.events).toHaveLength(1);
    expect(sessionOne.turns[0]?.final_answer).toBe("only session one");
    expect(sessionTwo.turns[0]?.events).toHaveLength(0);
    expect(sessionTwo.turns[0]?.final_answer).toBeNull();
  });

  it("does not append scoped core topics for unknown workers or contexts", () => {
    const current = upsertTurn(session("session_1"), turn("turn_1"));
    const unknownWorkerEvent: WebTurnEvent = {
      event_id: "event_unknown_worker",
      source: "core_topic",
      payload: {
        ...topic("core.action", { action: "run_bash", event: "start", input: { cmd: "pwd" } }),
        worker_id: "worker_missing",
      } as unknown as Record<string, unknown>,
      created_at_ms: 2,
    };
    const unknownContextEvent: WebTurnEvent = {
      event_id: "event_unknown_context",
      source: "core_topic",
      payload: {
        ...topic("core.action", { action: "run_bash", event: "start", input: { cmd: "pwd" } }),
        context_id: "context_missing",
      } as unknown as Record<string, unknown>,
      created_at_ms: 3,
    };

    expect(appendTurnEvent(current, "turn_1", unknownWorkerEvent).turns[0]?.events).toHaveLength(0);
    expect(appendTurnEvent(current, "turn_1", unknownContextEvent).turns[0]?.events).toHaveLength(0);
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

  it("never lets a ToolGen child response replace the primary final answer", () => {
    const primary = topic("core.model.response", {
      status: "finished",
      final_answer: "Primary answer",
      continue_work: false,
    });
    const toolgen = {
      ...topic("core.model.response", {
        status: "finished",
        final_answer: "Tool preservation skipped.",
        free_talk: "Decision details",
        runtime_phase: "toolgen",
        continue_work: false,
      }),
      context_id: "context_session_1",
      worker_id: "worker_session_1",
    };
    let current = upsertTurn(session("session_1"), turn("turn_1"));
    current = appendTurnEvent(current, "turn_1", { event_id: "main", source: "core_topic", payload: primary as unknown as Record<string, unknown>, created_at_ms: 2 });
    current = appendTurnEvent(current, "turn_1", { event_id: "toolgen", source: "core_topic", payload: toolgen as unknown as Record<string, unknown>, created_at_ms: 3 });
    const afterTopicReducer = applyCoreTopicToSession(current, toolgen, assistantMessage);

    expect(current.turns[0].final_answer).toBe("Primary answer");
    expect(afterTopicReducer.turns[0].final_answer).toBe("Primary answer");
    expect(afterTopicReducer.messages).toEqual([]);
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

  it("clears all worker working states when a cancelled session turn finishes", () => {
    let current = upsertTurn(session("session_1"), turn("turn_cancelled"));
    current.contexts.push({ context_id: "context_child", current_dir: "/work/child", worker_ids: ["worker_child"] });
    current.workers.push({
      worker_id: "worker_child",
      context_id: "context_child",
      display_name: "ID1",
      ordinal: 1,
      state: "working",
      parent_worker_id: current.primary_worker_id,
    });
    current = updateSessionWorkerState(current, current.primary_worker_id, "working");

    const finished = finishTurn(current, "turn_cancelled", {
      elapsed_ms: 42_000,
      stop_reason: "CancelledByUser",
    });

    expect(finished.active_turn_id).toBeNull();
    expect(finished.state).toBe("ready");
    expect(finished.workers.map((worker) => worker.state)).toEqual(["ready", "ready"]);
  });

  it("deduplicates replayed turn events by the host event id", () => {
    const active = upsertTurn(session("session_1"), turn("turn_1"));
    const event = { event_id: "stable_event", source: "worker_activity", payload: { kind: "model_retry" }, created_at_ms: 2 };
    const once = appendTurnEvent(active, "turn_1", event);
    const replayed = appendTurnEvent(once, "turn_1", event);
    expect(replayed.turns[0].events).toEqual([event]);
  });

  it("builds a source-turn-bound manual ToolGen command without inventing user text", () => {
    expect(manualToolGenCommand("session_1", "turn_7", "   ")).toEqual({
      type: "turn_submit",
      session_id: "session_1",
      input_kind: "toolgen",
      source_turn_id: "turn_7",
      text: "",
    });
    expect(manualToolGenCommand("session_1", "turn_7", "  Prefer Python.  ").text)
      .toBe("Prefer Python.");
  });

  it("does not apply a turn event to another session or another turn", () => {
    const first = upsertTurn(session("session_1"), turn("turn_1"));
    const event = { event_id: "event_x", source: "worker_activity", payload: { kind: "model_retry" }, created_at_ms: 2 };
    expect(appendTurnEvent(first, "turn_2", event)).toEqual(first);
    expect(appendTurnEvent(session("session_2"), "turn_1", event).turns).toEqual([]);
  });
});
