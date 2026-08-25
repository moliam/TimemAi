import { describe, expect, it } from "vitest";
import { ChatHistoryRecord, ChatMessage, CoreTopicEvent, Session, WebTurn, WebTurnEvent } from "../src/protocol";
import { activeModelRetryStatus, activityFromTopic, applySessionRuntimeProfile, appendActivityToCurrentTurn, appendTurnEvent, applyChatMessageDeleted, applyCoreTopicToSession, attachTurnCompletion, boundSessionHistory, clearDecisionsForSession, clearDecisionsForWorker, coalesceActionLifecycle, compareTurnTimelineItems, composerPrimaryAction, composerSendDecision, decisionKey, decisionsFromSessions, draftForSession, enqueueDecision, finishDraftSubmission, finishSessionDraftSubmission, finishTurn, groupDecisionsBySessionTurn, hasOnlyFreeTalkActivity, manualToolGenCommand, MAX_CLIENT_TURNS, MAX_RENDERED_MESSAGES, normalizeCopiedUserMessageText, prependHistoryRecords, pruneSessionDrafts, pruneSessionSubmissionLocks, redactSensitiveDisplayText, releaseSessionDraftSubmission, removePendingAttachment, requestDecision, reserveDraftSubmission, reserveSessionDraftSubmission, resolveActiveSessionId, runtimeConnectionLabel, sessionContextUsage, sessionCreateDecision, sessionInteractionLockReason, sessionRenameDecision, sessionTurnKey, setSessionDraft, tailPath, trimMessages, turnLiveUsage, turnTimelinePlacement, turnsFromHistoryRecords, visibleRuntimeRestartMarkers, updateSessionWorkerState, upsertSession, upsertTurn, workspacePathLabel } from "../src/view_model";

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

describe("user message clipboard normalization", () => {
  it("removes only trailing line breaks added by DOM selection serialization", () => {
    expect(normalizeCopiedUserMessageText("hello\n\n\n")).toBe("hello");
    expect(normalizeCopiedUserMessageText("第一行\n\n第二行\n\n")).toBe("第一行\n\n第二行");
    expect(normalizeCopiedUserMessageText("first\r\nsecond\r\n\r\n")).toBe("first\r\nsecond");
  });

  it("preserves internal line breaks, trailing spaces, and text without trailing line breaks", () => {
    expect(normalizeCopiedUserMessageText("hello   ")).toBe("hello   ");
    expect(normalizeCopiedUserMessageText("first\n\nsecond")).toBe("first\n\nsecond");
    expect(normalizeCopiedUserMessageText("")).toBe("");
    expect(normalizeCopiedUserMessageText("\n\r\n")).toBe("");
  });
});

describe("web topic view model", () => {
  it("reconstructs unresolved decisions from a working snapshot turn", () => {
    const waiting = topic("core.request", { request_id: "req-1", request: { command: "git status" } }, "waiting_user");
    const active = session("session_1");
    active.state = "working";
    active.turns = [{
      ...turn("turn_1"),
      state: "working",
      events: [
        { event_id: "wait-1", source: "core_topic", payload: waiting as unknown as Record<string, unknown>, created_at_ms: 2 },
        { event_id: "wait-duplicate", source: "core_topic", payload: waiting as unknown as Record<string, unknown>, created_at_ms: 3 },
      ],
    }];
    const decisions = decisionsFromSessions([active]);
    expect(decisions).toHaveLength(1);
    expect(decisions[0].turnId).toBe("turn_1");
    expect(decisions[0].event.payload.request_id).toBe("req-1");
  });

  it("does not resurrect decisions from completed turns", () => {
    const waiting = topic("core.request", { request_id: "req-old" }, "waiting_user");
    const completed = session("session_1");
    completed.turns = [{ ...turn("turn_1", "finished"), events: [{ event_id: "wait", source: "core_topic", payload: waiting as unknown as Record<string, unknown>, created_at_ms: 2 }] }];
    expect(decisionsFromSessions([completed])).toEqual([]);
  });

  it("routes local runtime errors into the current task work stream", () => {
    const active = session("session_1");
    active.turns = [turn("turn_1")];
    active.active_turn_id = "turn_1";

    const updated = appendActivityToCurrentTurn(active, {
      id: "runtime-warning-1",
      sessionId: "session_1",
      tone: "warning",
      title: "Runtime persistence warning",
      detail: "semantic_event_persist_failed",
      createdAt: 2,
    });

    expect(updated.turns[0].events).toHaveLength(1);
    expect(updated.turns[0].events[0]).toMatchObject({
      event_id: "runtime-warning-1",
      source: "ui_activity",
      created_at_ms: 2,
    });
  });

  it("defaults completed work to collapsed only when its process contains free talk alone", () => {
    const freeTalk = activityFromTopic(topic("core.model.response", { free_talk: "Simple reasoning." }));
    const action = activityFromTopic(topic("core.action", { action: "run_bash", status: "completed", input: { cmd: "pwd" } }));

    expect(freeTalk).toMatchObject({ tone: "thinking", kind: "free_talk", detail: "Simple reasoning." });
    expect(hasOnlyFreeTalkActivity(freeTalk ? [freeTalk] : [], 0)).toBe(true);
    expect(hasOnlyFreeTalkActivity([], 0)).toBe(false);
    expect(hasOnlyFreeTalkActivity([freeTalk, action].filter((activity): activity is NonNullable<typeof activity> => activity !== null), 0)).toBe(false);
    expect(hasOnlyFreeTalkActivity(freeTalk ? [freeTalk] : [], 1)).toBe(false);
  });

  it("renders ToolGen lifecycle as one compact system activity", () => {
    const started = activityFromTopic(topic("core.toolgen", { phase: "started", tool_count: 2 }));
    expect(started).toMatchObject({ tone: "notice", kind: "toolgen", title: "ToolGen: 正在评估…" });
    const published = activityFromTopic(topic("core.toolgen", { phase: "published", tool_count: 3, tool: { name: "trace-summarizer" }, retrospect: "Created and validated." }, "ready"));
    expect(published).toMatchObject({ tone: "notice", kind: "toolgen", title: "ToolGen: 已生成并验证 trace-summarizer", detail: "Created and validated." });
    const failed = activityFromTopic(topic("core.toolgen", { phase: "failed", error: "self-test failed" }, "ready"));
    expect(failed).toMatchObject({ tone: "warning", kind: "toolgen", title: "ToolGen: 生成失败", detail: "self-test failed" });
    expect(activityFromTopic(topic("core.model.response", { runtime_phase: "toolgen", free_talk: "Building a reusable parser.", final_answer: "internal completion" }))).toMatchObject({
      tone: "thinking",
      kind: "free_talk",
      detail: "Building a reusable parser.",
    });
    expect(activityFromTopic(topic("core.action", { runtime_phase: "toolgen", action: "run_bash", status: "running", input: { cmd: "bash tool.sh --self-test" } }))).toMatchObject({
      tone: "action",
      code: "bash tool.sh --self-test",
      code_language: "bash",
    });
    expect(activityFromTopic(topic("core.model.repair", {
      runtime_phase: "toolgen",
      attempt: 1,
      max_attempts: 5,
      issue: "invalid_xml",
    }))).toBeNull();
  });

  it("switches one composer action between stop and send across typing and turn-state races", () => {
    expect(composerPrimaryAction("working", "", false)).toBe("stop");
    expect(composerPrimaryAction("working", "  \n\t", false)).toBe("stop");
    expect(composerPrimaryAction("working", "follow-up", false)).toBe("send");
    expect(composerPrimaryAction("working", "", false)).toBe("stop");

    // Once cancellation has started, do not make newly typed text look sendable.
    expect(composerPrimaryAction("working", "typed during cancellation", true)).toBe("stop");
    expect(composerPrimaryAction("ready", "typed during cancellation completion", true)).toBe("stop");

    // Authoritative completion can race with local typing; after cancellation clears, a ready Session uses Send.
    expect(composerPrimaryAction("ready", "follow-up", false)).toBe("send");
    expect(composerPrimaryAction("ready", "", false)).toBe("send");
    expect(composerPrimaryAction(undefined, "draft without a Session", false)).toBe("send");
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

  it("keeps ordinary working-session text as a separate next turn", () => {
    const current = { ...session("session_1"), state: "working" };
    expect(composerSendDecision(current, "  ask this next  ", false)).toEqual({
      kind: "send",
      text: "ask this next",
      clearDraftOnSuccess: true,
      command: { type: "turn_submit", session_id: "session_1", text: "ask this next" },
    });
  });

  it("forces ready-session text to send immediately as a supplement with attachments", () => {
 const current = session("session_1");
 expect(composerSendDecision(
 current,
 " urgent context ",
 false,
 false,
 ["attachment_1", "attachment_2"],
 true,
 )).toEqual({
 kind: "send",
 text: "urgent context",
 clearDraftOnSuccess: true,
 command: {
 type: "turn_supplement",
 session_id: "session_1",
 text: "urgent context",
 attachment_ids: ["attachment_1", "attachment_2"],
 },
 });
 });

 it("forces a paused queued message to start a new turn even while the session is working", () => {
 const current = { ...session("session_1"), state: "working" };
 expect(composerSendDecision(
 current,
 " start this as a separate task ",
 false,
 false,
 ["attachment_1"],
 false,
 true,
 )).toEqual({
 kind: "send",
 text: "start this as a separate task",
 clearDraftOnSuccess: true,
 command: {
 type: "turn_submit",
 session_id: "session_1",
 text: "start this as a separate task",
 attachment_ids: ["attachment_1"],
 },
 });
 });

 it("keeps rapid ordinary sends during a working turn as separate next-turn submissions", () => {
    const current = { ...session("session_1"), state: "working" };
    const decisions = ["second question", "third question", "fourth question"].map((text) => composerSendDecision(current, text, false));
    expect(decisions.map((decision) => decision.kind)).toEqual(["send", "send", "send"]);
    expect(decisions.map((decision) => decision.kind === "send" ? decision.command : undefined)).toEqual([
      { type: "turn_submit", session_id: "session_1", text: "second question" },
      { type: "turn_submit", session_id: "session_1", text: "third question" },
      { type: "turn_submit", session_id: "session_1", text: "fourth question" },
    ]);
  });

  it("retains several loaded history pages instead of discarding the page just requested", () => {
    const current = session("session_1");
    current.turns = Array.from({ length: 200 }, (_, index) => turn(`current_${index}`, "finished"));
    const olderRecords: ChatHistoryRecord[] = Array.from({ length: 200 }, (_, index) => ({
      type: "message",
      role: "user",
      turn_id: `older_${index}`,
      created_at_ms: index,
      content: `older task ${index}`,
    }));

    const restored = prependHistoryRecords(current, olderRecords);

    expect(restored.turns).toHaveLength(400);
    expect(restored.turns[0]?.turn_id).toBe("older_0");
    expect(restored.turns.at(-1)?.turn_id).toBe("current_199");
  });

  it("retains complete restored and live action history", () => {
    const eventCount = 530;
    const restored = {
      ...turn("restored_turn", "restored"),
      events: Array.from({ length: eventCount }, (_, index) => ({
        event_id: `restored_event_${index}`,
        source: "worker_activity",
        payload: { kind: "action", index },
        created_at_ms: index,
      })),
    };
    const live = {
      ...turn("live_turn", "finished"),
      events: Array.from({ length: eventCount }, (_, index) => ({
        event_id: `live_event_${index}`,
        source: "worker_activity",
        payload: { kind: "action", index },
        created_at_ms: index,
      })),
    };

    const retained = boundSessionHistory({ ...session("session_1"), turns: [restored, live] });

    expect(retained.turns[0]?.events).toHaveLength(eventCount);
    expect(retained.turns[0]?.events[0]?.event_id).toBe("restored_event_0");
    expect(retained.turns[1]?.events).toHaveLength(eventCount);
    expect(retained.turns[1]?.events[0]?.event_id).toBe("live_event_0");
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

  it("uses explicit runtime-exit wording for disconnected interaction locks", () => {
    expect(sessionInteractionLockReason(false, false, true)).toBe("Connection lost. Reconnecting…");
    expect(sessionInteractionLockReason(false, false, true, 3)).toBe("Runtime unavailable. Restart timem-web.");
    expect(sessionInteractionLockReason(false, false, false)).toBe("Waiting for runtime snapshot…");
    expect(sessionInteractionLockReason(true, false, true)).toBe("Mem switch is in progress");
    expect(sessionInteractionLockReason(true, true, true)).toBe("Mem switch is in progress");
  });

  it("reports connection state without hiding a runtime exit behind reconnect text", () => {
    expect(runtimeConnectionLabel(false, false, false)).toBe("Connecting to runtime…");
    expect(runtimeConnectionLabel(false, false, true)).toBe("Connection lost. Reconnecting…");
    expect(runtimeConnectionLabel(false, false, true, 3)).toBe("Runtime unavailable. Restart timem-web.");
    expect(runtimeConnectionLabel(true, false, true)).toBe("Syncing runtime…");
    expect(runtimeConnectionLabel(true, true, true)).toBe("Runtime connected");
  });

  it("shows the tail of a long cwd while retaining short paths verbatim", () => {
    expect(tailPath("/short/workspace")).toBe("/short/workspace");
    const rendered = tailPath("/Users/example/very/long/company/project/packages/web-ui", 24);
    expect(rendered.startsWith("…")).toBe(true);
    expect(rendered.endsWith("project/packages/web-ui")).toBe(true);
    expect(rendered).toHaveLength(24);
  });

  it("keeps the complete workspace directory name in compact session labels", () => {
    expect(workspacePathLabel("/Users/limo3/my_code/timem_shell")).toBe("…/timem_shell");
    expect(workspacePathLabel("/Users/limo3/my_code/timem_shell/")).toBe("…/timem_shell");
    expect(workspacePathLabel("timem_shell")).toBe("timem_shell");
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

  it("replaces a background-running action with its later process exit", () => {
    const events = coalesceActionLifecycle([
      actionEvent("event_1", "start", "running", { cmd: "cargo test", background: true }, "call_bg"),
      actionEvent("event_2", "finish", "background_running", { cmd: "cargo test", background: true }, "call_bg"),
      actionEvent("event_3", "finish", "completed", { cmd: "cargo test", background: true }, "call_bg"),
    ]);
    expect(events).toHaveLength(1);
    expect((events[0].payload.payload as Record<string, unknown>).status).toBe("completed");
  });

  it("settles a restored background action when its start event was trimmed", () => {
    const events = coalesceActionLifecycle([
      actionEvent("turn_event_1787626168440_155", "finish", "background_running", { cmd: "cargo test -p timem_web", tail_out: true, timeout_ms: 300000 }, "call_TNTIyS9Gv3eSjDujSijCLyUD"),
      actionEvent("turn_event_1787626188091_162", "finish", "failed", { cmd: "cargo test -p timem_web" }, "call_TNTIyS9Gv3eSjDujSijCLyUD"),
    ]);
    expect(events).toHaveLength(1);
    expect(events[0].event_id).toBe("turn_event_1787626188091_162");
    expect((events[0].payload.payload as Record<string, unknown>).status).toBe("failed");
  });

  it("does not guess that legacy background events without action ids belong together", () => {
    const events = coalesceActionLifecycle([
      actionEvent("event_legacy_background", "finish", "background_running", { cmd: "cargo test" }),
      actionEvent("event_legacy_terminal", "finish", "failed", { cmd: "cargo test" }),
    ]);
    expect(events).toHaveLength(2);
    expect(events.map((event) => (event.payload.payload as Record<string, unknown>).status)).toEqual(["background_running", "failed"]);
  });

  it("does not settle one trimmed background action with another action id", () => {
    const events = coalesceActionLifecycle([
      actionEvent("event_background_a", "finish", "background_running", { cmd: "cargo test" }, "call_a"),
      actionEvent("event_terminal_b", "finish", "failed", { cmd: "cargo test" }, "call_b"),
    ]);
    expect(events).toHaveLength(2);
    expect(events.map((event) => (event.payload.payload as Record<string, unknown>).action_id)).toEqual(["call_a", "call_b"]);
    expect(events.map((event) => (event.payload.payload as Record<string, unknown>).status)).toEqual(["background_running", "failed"]);
  });

  it("settles a trimmed background action after compacted history reconstruction", () => {
    const actionId = "call_compacted_background";
    const actionTopic = (status: string, input: Record<string, unknown>) => ({
      session_id: "session_1",
      context_id: "context_1",
      worker_id: "worker_session_1",
      topic: { name: "core.action", attributes: { event: "finish", action_id: actionId } },
      state: { name: "running" },
      payload: { action: "run_bash", action_id: actionId, event: "finish", status, input },
    });
    const records: ChatHistoryRecord[] = [
      { type: "event", role: "system", turn_id: "turn_compacted", created_at_ms: 10, kind: "action", content: "background", source: "core_topic", payload: actionTopic("background_running", { cmd: "cargo test", timeout_ms: 300000 }) },
      { type: "event", role: "system", turn_id: "turn_compacted", created_at_ms: 20, kind: "context_compact", content: "compacted", source: "core_topic", payload: topic("core.context.compact", { estimated_before_tokens: 180000, estimated_after_tokens: 20000 }) },
      { type: "event", role: "system", turn_id: "turn_compacted", created_at_ms: 30, kind: "action", content: "failed", source: "core_topic", payload: actionTopic("failed", { cmd: "cargo test" }) },
    ];

    const [restored] = turnsFromHistoryRecords(records);
    const visible = coalesceActionLifecycle(restored.events);

    expect(visible).toHaveLength(2);
    expect((visible[0].payload.payload as Record<string, unknown>).status).toBe("failed");
    expect((visible[1].payload.topic as Record<string, unknown>).name).toBe("core.context.compact");
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

  it("restores runtime restart markers as system timeline messages without creating turns", () => {
    const current = {
      ...session("session_1"),
      turns: [turn("turn_2", "finished")],
      messages: [assistantMessage("current answer")],
    };
    const records: ChatHistoryRecord[] = [
      { type: "message", role: "user", turn_id: "turn_1", created_at_ms: 10, kind: "task", content: "older task" },
      { type: "message", role: "assistant", turn_id: "turn_1", created_at_ms: 20, content: "older answer" },
      { type: "message", role: "system", turn_id: "runtime_restart_30", created_at_ms: 30, kind: "runtime_restart", content: "Timem Web 已重新启动，以下内容来自新的运行实例" },
      { type: "message", role: "system", turn_id: "other_system_notice", created_at_ms: 31, kind: "other_notice", content: "not a chat divider" },
    ];

    const updated = prependHistoryRecords(current, records);
    expect(updated.turns.map((item) => item.turn_id)).toEqual(["turn_1", "turn_2"]);
    expect(updated.messages.map((message) => `${message.role}:${message.kind ?? ""}:${message.text}`)).toEqual([
      "user:task:older task",
      "assistant::older answer",
      "system:runtime_restart:Timem Web 已重新启动，以下内容来自新的运行实例",
      "assistant::current answer",
    ]);

    const markerOnly = prependHistoryRecords(session("session_2"), [records[2]]);
    expect(markerOnly.turns).toEqual([]);
    expect(markerOnly.messages).toHaveLength(1);
    expect(markerOnly.messages[0]).toMatchObject({
      role: "system",
      kind: "runtime_restart",
      created_at_ms: 30,
    });
  });

  it("shows only the latest restart marker when repeated restarts contain no work", () => {
    const markers: ChatMessage[] = [
      { id: "restart_10", role: "system", kind: "runtime_restart", text: "restart 1", created_at_ms: 10 },
      { id: "restart_20", role: "system", kind: "runtime_restart", text: "restart 2", created_at_ms: 20 },
      { id: "restart_30", role: "system", kind: "runtime_restart", text: "restart 3", created_at_ms: 30 },
    ];

    expect(visibleRuntimeRestartMarkers([], markers).map((marker) => marker.id)).toEqual(["restart_30"]);
  });

  it("orders turn work before a restart marker at the same timestamp", () => {
    const marker: ChatMessage = {
      id: "restart_same_time",
      role: "system",
      kind: "runtime_restart",
      text: "restart",
      created_at_ms: 30,
    };
    const completedWork = { ...turn("zz_turn", "finished"), created_at_ms: 30 };

    expect(visibleRuntimeRestartMarkers([completedWork], [marker])).toEqual([marker]);
  });

  it("places a restarted unfinished turn after the runtime restart divider as soon as it resumes", () => {
    const marker: ChatMessage = {
      id: "restart_100",
      role: "system",
      kind: "runtime_restart",
      text: "restart",
      created_at_ms: 100,
    };
    const restored = {
      ...turn("turn_before_restart", "restored"),
      created_at_ms: 10,
      events: [{
        event_id: "historical_thinking",
        source: "core_topic",
        payload: {},
        created_at_ms: 20,
      }],
    };

    expect(turnTimelinePlacement(restored, [marker])).toEqual({
      createdAtMs: 10,
      resumedAfterRestart: false,
    });
    expect(turnTimelinePlacement({ ...restored, state: "working" }, [marker])).toEqual({
      createdAtMs: 100,
      resumedAfterRestart: true,
    });
  });

  it("sorts the restart divider before a resumed pre-restart turn", () => {
    const timeline = [
      {
        type: "turn" as const,
        createdAtMs: 100,
        resumedAfterRestart: true,
        id: "old_turn",
      },
      {
        type: "restart" as const,
        createdAtMs: 100,
        resumedAfterRestart: false,
        id: "restart_100",
      },
    ].sort(compareTurnTimelineItems);

    expect(timeline.map((item) => item.type)).toEqual(["restart", "turn"]);
  });

  it("keeps ordinary turn work before a same-time restart marker", () => {
    const timeline = [
      {
        type: "restart" as const,
        createdAtMs: 100,
        resumedAfterRestart: false,
        id: "restart_100",
      },
      {
        type: "turn" as const,
        createdAtMs: 100,
        resumedAfterRestart: false,
        id: "new_turn",
      },
    ].sort(compareTurnTimelineItems);

    expect(timeline.map((item) => item.type)).toEqual(["turn", "restart"]);
  });

  it("keeps a resumed turn below the latest restart divider after new activity arrives", () => {
    const markers: ChatMessage[] = [
      { id: "restart_100", role: "system", kind: "runtime_restart", text: "restart 1", created_at_ms: 100 },
      { id: "restart_200", role: "system", kind: "runtime_restart", text: "restart 2", created_at_ms: 200 },
    ];
    const resumed = {
      ...turn("turn_before_restarts", "finished"),
      created_at_ms: 10,
      events: [
        { event_id: "old_event", source: "core_topic", payload: {}, created_at_ms: 20 },
        { event_id: "new_event", source: "core_topic", payload: {}, created_at_ms: 220 },
      ],
    };

    expect(turnTimelinePlacement(resumed, markers)).toEqual({
      createdAtMs: 200,
      resumedAfterRestart: true,
    });
  });

  it("does not move completed historical turns across a later restart divider", () => {
    const marker: ChatMessage = {
      id: "restart_100",
      role: "system",
      kind: "runtime_restart",
      text: "restart",
      created_at_ms: 100,
    };
    const completed = {
      ...turn("old_completed_turn", "finished"),
      created_at_ms: 10,
      events: [{
        event_id: "old_completion",
        source: "core_topic",
        payload: {},
        created_at_ms: 50,
      }],
    };

    expect(turnTimelinePlacement(completed, [marker])).toEqual({
      createdAtMs: 10,
      resumedAfterRestart: false,
    });
  });

  it("keeps restart markers separated by actual turn work", () => {
    const markers: ChatMessage[] = [
      { id: "restart_10", role: "system", kind: "runtime_restart", text: "restart 1", created_at_ms: 10 },
      { id: "restart_20", role: "system", kind: "runtime_restart", text: "restart 2", created_at_ms: 20 },
      { id: "restart_40", role: "system", kind: "runtime_restart", text: "restart 3", created_at_ms: 40 },
      { id: "restart_50", role: "system", kind: "runtime_restart", text: "restart 4", created_at_ms: 50 },
    ];
    const completedWork = { ...turn("turn_30", "finished"), created_at_ms: 30 };

    expect(visibleRuntimeRestartMarkers([completedWork], markers).map((marker) => marker.id)).toEqual([
      "restart_20",
      "restart_50",
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

  it("synchronizes the context meter denominator when a runtime profile is updated", () => {
    const current = session("session_1");
    const runtimeProfile: NonNullable<Session["runtime_profile"]> = {
      model: "gpt-4.1",
      api_protocol: "openai-compatible",
      response_protocol: "xml",
      base_url: "https://api.example.test/v1",
      timeout_secs: 60,
      max_llm_input_tokens: 1_000_000,
      max_llm_output_tokens: 50_000,
      max_rounds: "50",
      bash_approval: "ask",
      work_instructions: "silent",
      api_key_configured: true,
    };

    const updated = applySessionRuntimeProfile(current, runtimeProfile);

    expect(updated.runtime_profile).toBe(runtimeProfile);
    expect(updated.max_llm_input_tokens).toBe(1_000_000);
    expect(current.max_llm_input_tokens).toBe(100_000);
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

  it("uses only the selected session's latest real model usage for context", () => {
    const current = session("session_1");
    const oldTurn = turn("old", "finished");
    oldTurn.completion = { latest_usage: { prompt_tokens: 2_000 } };
    const activeTurn = turn("active");
    activeTurn.events = [{ event_id: "latest", source: "worker_activity", created_at_ms: 3, payload: { kind: "model_response", usage: { prompt_tokens: 8_200, completion_tokens: 40 } } }];
    current.turns = [oldTurn, activeTurn];

    expect(sessionContextUsage(current)?.prompt_tokens).toBe(8_200);
    expect(sessionContextUsage(session("session_2"))).toBeUndefined();
  });

  it("resets session context usage at the latest runtime restart boundary", () => {
    const restarted = session("session_restarted");
    const oldTurn = turn("old", "finished");
    oldTurn.created_at_ms = 100;
    oldTurn.events = [
      { event_id: "old_usage", source: "worker_activity", created_at_ms: 120, payload: { kind: "model_response", usage: { prompt_tokens: 24_000, completion_tokens: 500 } } },
    ];
    oldTurn.completion = { latest_usage: { prompt_tokens: 24_000, completion_tokens: 500 } };
    restarted.turns = [oldTurn];
    restarted.messages = [{
      id: "restart",
      role: "system",
      kind: "runtime_restart",
      text: "Timem Web 已重新启动，以下内容来自新的运行实例",
      created_at_ms: 200,
    }];

    expect(sessionContextUsage(restarted)).toBeUndefined();

    const resumedTurn = turn("resumed", "working");
    resumedTurn.created_at_ms = 150;
    resumedTurn.events = [
      { event_id: "restored_old_usage", source: "worker_activity", created_at_ms: 180, payload: { kind: "model_response", usage: { prompt_tokens: 30_000 } } },
      { event_id: "new_runtime_usage", source: "worker_activity", created_at_ms: 220, payload: { kind: "model_response", usage: { prompt_tokens: 4_600, completion_tokens: 30 } } },
    ];
    restarted.turns.push(resumedTurn);
    expect(sessionContextUsage(restarted)).toEqual({ prompt_tokens: 4_600, completion_tokens: 30 });

    const newCompletedTurn = turn("new_completed", "finished");
    newCompletedTurn.created_at_ms = 240;
    newCompletedTurn.completion = { latest_usage: { prompt_tokens: 6_200, completion_tokens: 45 } };
    restarted.turns.push(newCompletedTurn);
    expect(sessionContextUsage(restarted)).toEqual({ prompt_tokens: 6_200, completion_tokens: 45 });
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

  it("keeps model recovery out of the thought stream and exposes the active header status", () => {
    expect(activityFromTopic(topic("core.model.repair", {
      attempt: 1,
      issue: "missing_response_root",
    }))).toBeNull();

    const retrying = turn("repairing", "working");
    retrying.events = [{
      event_id: "repair",
      source: "core_topic",
      created_at_ms: 2,
      payload: topic("core.model.repair", {
        attempt: 2,
        max_attempts: 20,
        issue: "missing_response_root",
      }) as unknown as Record<string, unknown>,
    }];

    expect(activeModelRetryStatus(retrying)).toMatchObject({
      kind: "retrying",
      label: "retrying",
      progress: "2/20",
    });
    expect(activeModelRetryStatus(retrying)?.detail)
      .toContain("回复缺少必需的 response 根节点");

    const reconnecting = turn("reconnecting", "working");
    reconnecting.events = [{
      event_id: "retry",
      source: "worker_activity",
      created_at_ms: 2,
      payload: {
        kind: "model_retry",
        attempt: 3,
        max_attempts: 100,
        delay_ms: 10_000,
        error: "model_http_503: upstream unavailable",
      },
    }];

    expect(activeModelRetryStatus(reconnecting)).toMatchObject({
      kind: "reconnecting",
      label: "reconnecting",
      progress: "3/100",
    });
    expect(activeModelRetryStatus(reconnecting)?.detail)
      .toContain("model_http_503");

    const changingFailure = turn("changing-failure", "working");
    changingFailure.events = [
      retrying.events[0],
      reconnecting.events[0],
    ];
    expect(activeModelRetryStatus(changingFailure)?.kind).toBe("reconnecting");

    changingFailure.events.push({
      ...retrying.events[0],
      event_id: "later-repair",
      created_at_ms: 4,
    });
    expect(activeModelRetryStatus(changingFailure)?.kind).toBe("retrying");
  });

  it("clears the temporary recovery status after recovery or turn completion", () => {
    const recovered = turn("recovered", "working");
    recovered.events = [
      {
        event_id: "retry",
        source: "worker_activity",
        created_at_ms: 2,
        payload: {
          kind: "model_retry",
          attempt: 1,
          max_attempts: 100,
          error: "model_timeout",
        },
      },
      {
        event_id: "response",
        source: "worker_activity",
        created_at_ms: 3,
        payload: { kind: "model_response" },
      },
    ];
    expect(activeModelRetryStatus(recovered)).toBeNull();

    const completed = {
      ...recovered,
      state: "finished",
      events: [recovered.events[0]],
    };
    expect(activeModelRetryStatus(completed)).toBeNull();
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

  it("keeps a completed final answer out of the thought and action timeline", () => {
    expect(activityFromTopic(topic("core.model.response", {
      status: "finished",
      free_talk: "这是完成前的总结。",
      progress: "已经完成。",
      final_answer: "这是只应出现在最终答案区域的内容。",
    }))).toBeNull();
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
      estimated_text_before_tokens: 12_000,
      estimated_text_after_tokens: 4_000,
      estimated_native_before_tokens: 70_000,
      estimated_native_after_tokens: 10_000,
    }));
    expect(activity).toMatchObject({
      kind: "context_compact",
      tone: "notice",
      title: "Dynamic context compacted",
      before_tokens: 82_000,
      after_tokens: 14_000,
      text_before_tokens: 12_000,
      text_after_tokens: 4_000,
      native_before_tokens: 70_000,
      native_after_tokens: 10_000,
    });
  });

  it("renders run_bash commands as Bash code and keeps the structured status", () => {
    const activity = activityFromTopic(topic("core.action", { action: "run_bash", status: "running", input: { cmd: "git status" } }));
    expect(activity).toMatchObject({ tone: "action", title: "Bash · running", tool_name: "run_bash", detail: "", code: "git status", code_language: "bash" });
  });

  it("redacts credentials from displayed Bash commands without hiding command structure", () => {
    const raw = "curl -H 'Authorization: Bearer top-secret' -H 'X-Example-GWToken: token-123' --api-key other-secret https://example.test?token=query-secret";
    const activity = activityFromTopic(topic("core.action", { action: "run_bash", status: "running", input: { cmd: raw } }));
    expect(activity?.code).toBe("curl -H 'Authorization: ****' -H 'X-Example-GWToken: ****' --api-key **** https://example.test?token=****");
    expect(activity?.code).not.toContain("top-secret");
    expect(activity?.code).not.toContain("token-123");
    expect(activity?.code).not.toContain("other-secret");
    expect(activity?.code).not.toContain("query-secret");
    expect(redactSensitiveDisplayText("echo ordinary=value token_count=123")).toBe("echo ordinary=value token_count=123");
  });

  it("keeps the Bash command visible when a finish topic only carries its action kind", () => {
    const activity = activityFromTopic(topic("core.action", {
      action: "run_bash",
      status: "completed",
      kind: { kind: "bash", command: "gh run list --limit 5", mode: "normal" },
    }, "ready"));
    expect(activity).toMatchObject({
      title: "Bash · completed",
      code: "gh run list --limit 5",
      code_language: "bash",
    });
  });

  it("shows human-readable action statuses while preserving structured tool status", () => {
    const background = activityFromTopic(topic("core.action", { action: "run_bash", status: "background_running", input: { cmd: "cargo test" } }));
    expect(background).toMatchObject({ title: "Bash · background running", tool_status: "background_running" });

    const timeout = activityFromTopic(topic("core.action", { action: "run_bash", status: "timeout", input: { cmd: "sleep 30" } }));
    expect(timeout).toMatchObject({ title: "Bash · timed out", tool_status: "timeout" });
  });

  it("derives completed action duration from its start and finish topics", () => {
    const start = actionEvent("1000", "start", "running", { cmd: "scripts/ci.sh" }, "ci");
    const finish = actionEvent("84250", "finish", "completed", { cmd: "scripts/ci.sh" }, "ci");
    const [completed] = coalesceActionLifecycle([start, finish]);
    const completedTopic = completed.payload as unknown as CoreTopicEvent;

    expect(completedTopic.payload.elapsed_ms).toBe(83250);
    expect(activityFromTopic(completedTopic)).toMatchObject({
      tool_status: "completed",
      elapsed_ms: 83250,
    });
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

  it("redacts nested sensitive builtin-tool arguments while retaining ordinary options", () => {
    const activity = activityFromTopic(topic("core.action", {
      action: "remote_tool",
      status: "running",
      input: {
        endpoint: "https://example.test",
        headers: { Authorization: "Bearer top-secret", Accept: "application/json" },
        api_key: "other-secret",
      },
    }));
    expect(activity?.detail).toContain('endpoint="https://example.test"');
    expect(activity?.detail).toContain('"Authorization":"****"');
    expect(activity?.detail).toContain('"Accept":"application/json"');
    expect(activity?.detail).toContain('api_key="****"');
    expect(activity?.detail).not.toContain("top-secret");
    expect(activity?.detail).not.toContain("other-secret");
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

  it("explains elapsed time and timeout for a long-running command decision", () => {
    const decision = requestDecision(topic("core.user.long_running_command.request", {
      kind: "long_running_command_continue",
      request: {
        command: "scripts/ci.sh 2>&1 | tee /tmp/ci_output.log; echo EXIT_CODE=$?",
        elapsed_ms: 60_250,
        timeout_ms: 600_000,
      },
    }, "waiting_user"));

    expect(decision).toMatchObject({
      title: "Long-running command",
      detail: "The command has been running for 1m00s; timeout is 10m00s.\nCommand: scripts/ci.sh 2>&1 | tee /tmp/ci_output.log; echo EXIT_CODE=$?",
    });
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

  it("keeps same-named turns in separate sessions isolated in the decision queue", () => {
    const first = requestDecision(topic("core.request", { request_id: "req_1" }, "waiting_user"))!;
    const second = requestDecision({
      ...topic("core.request", { request_id: "req_2" }, "waiting_user"),
      session_id: "session_2",
    })!;
    const decisions = [
      { ...first, turnId: "turn_shared" },
      { ...second, turnId: "turn_shared" },
    ];

    const grouped = groupDecisionsBySessionTurn(decisions);

    expect(grouped.get(sessionTurnKey("session_1", "turn_shared"))).toEqual([decisions[0]]);
    expect(grouped.get(sessionTurnKey("session_2", "turn_shared"))).toEqual([decisions[1]]);
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
    const eventCount = 550;
    const current = session("session_pressure");
    current.turns = Array.from({ length: MAX_CLIENT_TURNS + 40 }, (_, turnIndex) => ({
      ...turn(`turn_${turnIndex}`, "finished"),
      events: Array.from({ length: eventCount }, (_, eventIndex) => ({
        event_id: `event_${turnIndex}_${eventIndex}`,
        source: "worker_activity",
        payload: { kind: "progress", marker: `${turnIndex}:${eventIndex}` },
        created_at_ms: eventIndex,
      })),
    }));

    const bounded = boundSessionHistory(current);
    expect(bounded.turns).toHaveLength(MAX_CLIENT_TURNS);
    expect(bounded.turns[0]?.turn_id).toBe("turn_40");
    expect(bounded.turns.every((item) => item.events.length === eventCount)).toBe(true);
    expect(bounded.turns.at(-1)?.events[0]?.payload.marker).toBe(`${MAX_CLIENT_TURNS + 39}:0`);
  });

  it("keeps repeated live event bursts bounded and isolated across sessions", () => {
    const totalEvents = 1500;
    let sessions = Array.from({ length: 5 }, (_, index) => upsertTurn(session(`pressure_${index}`), turn(`turn_${index}`)));
    for (let eventIndex = 0; eventIndex < totalEvents; eventIndex += 1) {
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
      expect(events.length).toBe(totalEvents / sessions.length);
      expect(events.every((event) => event.payload.owner === current.session_id)).toBe(true);
    }
  });

  it("keeps a human click storm bounded and session scoped", () => {
    let sessions = Array.from({ length: 5 }, (_, index) => {
      const active = upsertTurn(session(`storm_${index}`), turn(`turn_${index}`));
      return updateSessionWorkerState(active, active.primary_worker_id, "working");
    });
    const acceptedNextTurns = new Map<string, string[]>();

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
          command: { type: "turn_submit", session_id: target.session_id, text },
        });
        acceptedNextTurns.set(target.session_id, [
          ...(acceptedNextTurns.get(target.session_id) ?? []),
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
      expect(events.length).toBe(120);
      expect(events.every((event) => event.payload.owner === current.session_id)).toBe(true);
      expect(current.state).toBe("working");
      expect(current.workers.every((worker) => worker.state === "working")).toBe(true);
      expect(acceptedNextTurns.get(current.session_id)?.length).toBeGreaterThan(80);
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
    expect(replayed).toBe(once);
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
    expect(appendTurnEvent(first, "turn_2", event)).toBe(first);
    expect(appendTurnEvent(session("session_2"), "turn_1", event).turns).toEqual([]);
  });

  it("does not recreate a session when a worker repeats its current state", () => {
    const current = updateSessionWorkerState(session("session_1"), "worker_session_1", "working");
    expect(updateSessionWorkerState(current, "worker_session_1", "working")).toBe(current);
  });

  it("applies user and assistant message deletion to both turn and chat projections", () => {
    const current = session("session_1");
    current.turns = [{
      ...turn("turn_1", "finished"),
      user_entries: [
        { kind: "task", text: "same", created_at_ms: 10 },
        { kind: "supplement", text: "same", created_at_ms: 20 },
      ],
      final_answer: "answer",
    }];
    current.messages = [
      { id: "u1", role: "user", text: "same", created_at_ms: 10 },
      { id: "u2", role: "user", text: "same", created_at_ms: 20 },
      { id: "a1", role: "assistant", text: "answer", created_at_ms: 30 },
    ];

    const withoutSupplement = applyChatMessageDeleted(current, "turn_1", "user", 1);
    expect(withoutSupplement.turns[0].user_entries.map((entry) => entry.created_at_ms)).toEqual([10]);
    expect(withoutSupplement.messages.map((message) => message.id)).toEqual(["u1", "a1"]);

    const withoutAnswer = applyChatMessageDeleted(withoutSupplement, "turn_1", "assistant", 0);
    expect(withoutAnswer.turns[0].final_answer).toBeNull();
    expect(withoutAnswer.messages.map((message) => message.id)).toEqual(["u1"]);
  });
});
