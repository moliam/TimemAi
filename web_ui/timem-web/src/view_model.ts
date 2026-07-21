import { Activity, ChatHistoryRecord, ChatMessage, ClientCommand, CoreTopicEvent, Decision, Session, TurnCompletion, WebTurn, WebTurnEvent } from "./protocol";

export const MAX_RENDERED_MESSAGES = 1000;
export const MAX_CLIENT_TURNS = 200;
export const MAX_CLIENT_TURN_EVENTS = 500;

const USAGE_FIELDS = ["llm_calls", "repair_calls", "tool_calls", "mem_reads", "mem_writes", "prompt_tokens", "completion_tokens", "total_tokens", "cached_tokens", "cache_created_tokens", "shrunk_tokens"] as const;

export function trimMessages<T>(messages: T[]) {
  return messages.length > MAX_RENDERED_MESSAGES ? messages.slice(-MAX_RENDERED_MESSAGES) : messages;
}

export function trimTurnEvents<T>(events: T[]) {
  return events.length > MAX_CLIENT_TURN_EVENTS ? events.slice(-MAX_CLIENT_TURN_EVENTS) : events;
}

export function trimTurns<T>(turns: T[]) {
  return turns.length > MAX_CLIENT_TURNS ? turns.slice(-MAX_CLIENT_TURNS) : turns;
}

export function tailPath(path: string, maxChars = 28) {
  if (path.length <= maxChars) return path;
  return `…${path.slice(-(Math.max(2, maxChars) - 1))}`;
}

export function runtimeConnectionLabel(connected: boolean, snapshotReady: boolean, runtimeEverConnected: boolean, reconnectAttempt = 0) {
  if (!connected && runtimeEverConnected) {
    return reconnectAttempt >= 3 ? "Runtime unavailable. Restart timem-web." : "Connection lost. Reconnecting…";
  }
  if (!connected) return "Connecting to runtime…";
  return snapshotReady ? "Runtime connected" : "Syncing runtime…";
}

export function sessionInteractionLockReason(pendingMemSwitch: boolean, connected: boolean, runtimeEverConnected: boolean, reconnectAttempt = 0) {
  if (pendingMemSwitch) return "Mem switch is in progress";
  if (!connected && runtimeEverConnected) {
    return reconnectAttempt >= 3 ? "Runtime unavailable. Restart timem-web." : "Connection lost. Reconnecting…";
  }
  return "Waiting for runtime snapshot…";
}

export type ComposerSendDecision =
  | { kind: "skip"; reason: "no_session" | "empty_text" | "cancelling" | "mem_switching" }
  | { kind: "send"; command: Extract<ClientCommand, { type: "turn_submit" | "turn_supplement" }>; text: string; clearDraftOnSuccess: true };

export type SessionRenameDecision =
  | { kind: "skip"; reason: "no_session" | "empty_name" | "already_pending" | "mem_switching" }
  | { kind: "send"; command: Extract<ClientCommand, { type: "session_rename" }>; displayName: string };

export type SessionCreateDecision =
  | { kind: "skip"; reason: "empty_workspace" | "creating" | "mem_switching" }
  | { kind: "send"; command: Extract<ClientCommand, { type: "session_create" }>; displayName: string; workspaceDir: string; env: Record<string, string> };

export type DraftSubmissionLock = { current: boolean };
export type SessionDraftSubmissionLocks = { current: Set<string> };
export type SessionDrafts = Record<string, string>;

export function reserveDraftSubmission(lock: DraftSubmissionLock, draft: string): string | null {
  if (lock.current) return null;
  const text = draft.trim();
  if (!text) return null;
  lock.current = true;
  return text;
}

export function finishDraftSubmission(
  lock: DraftSubmissionLock,
  draft: string,
  submittedText: string | null,
  sent: boolean,
): string {
  lock.current = false;
  if (!sent || submittedText === null) return draft;
  return draft.trim() === submittedText ? "" : draft;
}

export function draftForSession(drafts: SessionDrafts, sessionId: string | undefined): string {
  return sessionId ? drafts[sessionId] ?? "" : "";
}

export function setSessionDraft(drafts: SessionDrafts, sessionId: string | undefined, value: string): SessionDrafts {
  if (!sessionId) return drafts;
  if (!value) {
    const { [sessionId]: _removed, ...remaining } = drafts;
    return remaining;
  }
  return { ...drafts, [sessionId]: value };
}

export function reserveSessionDraftSubmission(
  locks: SessionDraftSubmissionLocks,
  sessionId: string | undefined,
  drafts: SessionDrafts,
): { sessionId: string; text: string } | null {
  if (!sessionId || locks.current.has(sessionId)) return null;
  const text = draftForSession(drafts, sessionId).trim();
  if (!text) return null;
  locks.current.add(sessionId);
  return { sessionId, text };
}

export function finishSessionDraftSubmission(
  locks: SessionDraftSubmissionLocks,
  drafts: SessionDrafts,
  sessionId: string,
  submittedText: string,
  sent: boolean,
): SessionDrafts {
  locks.current.delete(sessionId);
  const current = draftForSession(drafts, sessionId);
  if (!sent) return drafts;
  return current.trim() === submittedText ? setSessionDraft(drafts, sessionId, "") : drafts;
}

export function releaseSessionDraftSubmission(locks: SessionDraftSubmissionLocks, sessionId: string): boolean {
  return locks.current.delete(sessionId);
}

export function pruneSessionDrafts(drafts: SessionDrafts, liveSessionIds: Iterable<string>): SessionDrafts {
  const live = new Set(liveSessionIds);
  let changed = false;
  const next: SessionDrafts = {};
  for (const [sessionId, draft] of Object.entries(drafts)) {
    if (live.has(sessionId)) {
      next[sessionId] = draft;
    } else {
      changed = true;
    }
  }
  return changed ? next : drafts;
}

export function pruneSessionSubmissionLocks(
  locks: SessionDraftSubmissionLocks,
  liveSessionIds: Iterable<string>,
): boolean {
  const live = new Set(liveSessionIds);
  let changed = false;
  for (const sessionId of Array.from(locks.current)) {
    if (!live.has(sessionId)) {
      locks.current.delete(sessionId);
      changed = true;
    }
  }
  return changed;
}

export function resolveActiveSessionId(currentSessionId: string, sessions: Pick<Session, "session_id">[]): string {
  if (currentSessionId && sessions.some((session) => session.session_id === currentSessionId)) {
    return currentSessionId;
  }
  return sessions[0]?.session_id ?? "";
}

export function composerSendDecision(
  session: Pick<Session, "session_id" | "state"> | undefined,
  text: string,
  isCancelling: boolean,
  isMemSwitching = false,
): ComposerSendDecision {
  if (!session) return { kind: "skip", reason: "no_session" };
  const trimmed = text.trim();
  if (!trimmed) return { kind: "skip", reason: "empty_text" };
  if (isMemSwitching) return { kind: "skip", reason: "mem_switching" };
  if (isCancelling) return { kind: "skip", reason: "cancelling" };
  return {
    kind: "send",
    text: trimmed,
    clearDraftOnSuccess: true,
    command: session.state === "working"
      ? { type: "turn_supplement", session_id: session.session_id, text: trimmed }
      : { type: "turn_submit", session_id: session.session_id, text: trimmed },
  };
}

export function manualToolGenCommand(
  sessionId: string,
  sourceTurnId: string,
  optionalGuidance: string,
): ClientCommand {
  return {
    type: "turn_submit",
    session_id: sessionId,
    input_kind: "toolgen",
    source_turn_id: sourceTurnId,
    text: optionalGuidance.trim(),
  };
}

export function sessionRenameDecision(
  sessionId: string | undefined,
  draftName: string,
  pendingSessionIds: ReadonlySet<string>,
  isMemSwitching = false,
): SessionRenameDecision {
  if (!sessionId) return { kind: "skip", reason: "no_session" };
  if (isMemSwitching) return { kind: "skip", reason: "mem_switching" };
  const displayName = draftName.trim();
  if (!displayName) return { kind: "skip", reason: "empty_name" };
  if (pendingSessionIds.has(sessionId)) return { kind: "skip", reason: "already_pending" };
  return {
    kind: "send",
    displayName,
    command: { type: "session_rename", session_id: sessionId, display_name: displayName },
  };
}

export function sessionCreateDecision(
  displayNameDraft: string,
  workspaceDirDraft: string,
  envDraft: Record<string, string>,
  creating: boolean,
  isMemSwitching = false,
): SessionCreateDecision {
  if (isMemSwitching) return { kind: "skip", reason: "mem_switching" };
  if (creating) return { kind: "skip", reason: "creating" };
  const workspaceDir = workspaceDirDraft.trim();
  if (!workspaceDir) return { kind: "skip", reason: "empty_workspace" };
  const displayName = displayNameDraft.trim();
  const env = Object.fromEntries(Object.entries(envDraft)
    .map(([key, value]) => [key, value.trim()])
    .filter(([, value]) => value));
  return {
    kind: "send",
    displayName,
    workspaceDir,
    env,
    command: {
      type: "session_create",
      ...(displayName ? { display_name: displayName } : {}),
      workspace_dir: workspaceDir,
      env,
    },
  };
}

function actionLifecycleKey(event: WebTurnEvent) {
  if (event.source !== "core_topic") return undefined;
  const topicEvent = event.payload as unknown as CoreTopicEvent;
  if (topicEvent.topic?.name !== "core.action") return undefined;
  const action = typeof topicEvent.payload.action === "string" ? topicEvent.payload.action : "";
  if (!action) return undefined;
  const actionId = typeof topicEvent.payload.action_id === "string"
    ? topicEvent.payload.action_id
    : typeof topicEvent.topic.attributes?.action_id === "string"
      ? topicEvent.topic.attributes.action_id
      : "";
  if (actionId) return `id:${actionId}`;
  return `${action}:${JSON.stringify(stableJsonValue(topicEvent.payload.input ?? null))}`;
}

function stableJsonValue(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(stableJsonValue);
  if (value && typeof value === "object") {
    return Object.fromEntries(
      Object.entries(value as Record<string, unknown>)
        .sort(([left], [right]) => left.localeCompare(right))
        .map(([key, nested]) => [key, stableJsonValue(nested)]),
    );
  }
  return value;
}

function toolgenLifecycle(event: WebTurnEvent) {
  if (event.source !== "core_topic") return undefined;
  const topicEvent = event.payload as unknown as CoreTopicEvent;
  if (topicEvent.topic?.name !== "core.toolgen") return undefined;
  const phase = typeof topicEvent.payload.phase === "string" ? topicEvent.payload.phase : "";
  return phase ? { key: `toolgen:${topicEvent.context_id ?? "unknown"}`, phase } : undefined;
}

export function coalesceActionLifecycle(events: WebTurnEvent[]) {
  const visible: WebTurnEvent[] = [];
  const pendingStarts = new Map<string, number[]>();
  const pendingToolGen = new Set<string>();
  for (const event of events) {
    const toolgen = toolgenLifecycle(event);
    if (toolgen) {
      if (toolgen.phase === "started") {
        pendingToolGen.add(toolgen.key);
      } else {
        visible.push(event);
        pendingToolGen.delete(toolgen.key);
      }
      continue;
    }
    const key = actionLifecycleKey(event);
    if (!key) {
      visible.push(event);
      continue;
    }
    const topicEvent = event.payload as unknown as CoreTopicEvent;
    const lifecycle = typeof topicEvent.payload.event === "string"
      ? topicEvent.payload.event
      : topicEvent.topic.attributes?.event;
    if (lifecycle === "start") {
      const index = visible.push(event) - 1;
      pendingStarts.set(key, [...(pendingStarts.get(key) ?? []), index]);
      continue;
    }
    if (lifecycle === "finish") {
      const indexes = pendingStarts.get(key);
      const index = indexes?.shift();
      if (index !== undefined) visible[index] = event;
      else visible.push(event);
      if (indexes?.length === 0) pendingStarts.delete(key);
      continue;
    }
    visible.push(event);
  }
  return visible;
}

export function boundSessionHistory(session: Session): Session {
  return {
    ...session,
    messages: trimMessages(session.messages),
    turns: trimTurns(session.turns).map((turn) => ({ ...turn, events: trimTurnEvents(turn.events) })),
  };
}

export function upsertSession(sessions: Session[], incoming: Session) {
  const bounded = boundSessionHistory(incoming);
  return sessions.some((session) => session.session_id === incoming.session_id)
    ? sessions.map((session) => session.session_id === incoming.session_id ? bounded : session)
    : [...sessions, bounded];
}

export function removePendingAttachment(session: Session, attachmentId: string): Session {
  return {
    ...session,
    attachments: session.attachments.filter((attachment) => attachment.id !== attachmentId),
  };
}

export function upsertTurn(session: Session, incoming: WebTurn): Session {
  const boundedIncoming = { ...incoming, events: trimTurnEvents(incoming.events) };
  const turns = trimTurns(session.turns.some((turn) => turn.turn_id === incoming.turn_id)
    ? session.turns.map((turn) => turn.turn_id === incoming.turn_id ? boundedIncoming : turn)
    : [...session.turns, boundedIncoming]);
  return {
    ...session,
    state: incoming.state === "working" ? "working" : session.state,
    active_turn_id: incoming.state === "working" ? incoming.turn_id : session.active_turn_id,
    turns,
  };
}

export function prependHistoryRecords(session: Session, records: ChatHistoryRecord[]): Session {
  const historicalTurns = turnsFromHistoryRecords(records);
  if (historicalTurns.length === 0) return session;
  const existing = new Set(session.turns.map((turn) => turn.turn_id));
  const earlier = historicalTurns.filter((turn) => !existing.has(turn.turn_id));
  if (earlier.length === 0) return session;
  const earlierTurnIds = new Set(earlier.map((turn) => turn.turn_id));
  return {
    ...session,
    turns: trimTurns([...earlier, ...session.turns]),
    messages: trimMessages([
      ...messagesFromHistoryRecords(records.filter((record) => earlierTurnIds.has(record.turn_id))),
      ...session.messages,
    ]),
  };
}

export function turnsFromHistoryRecords(records: ChatHistoryRecord[]): WebTurn[] {
  const turns = new Map<string, WebTurn>();
  for (const record of records) {
    const turn = turns.get(record.turn_id) ?? {
      turn_id: record.turn_id,
      state: "restored",
      created_at_ms: record.created_at_ms,
      user_entries: [],
      events: [],
      final_answer: null,
      completion: null,
    };
    turn.created_at_ms = Math.min(turn.created_at_ms, record.created_at_ms);
    if (record.type === "message") {
      if (record.role === "user") {
        const kind = record.kind && ["task", "supplement", "approval"].includes(record.kind)
          ? record.kind
          : "task";
        turn.user_entries.push({ kind, text: record.content, attachments: [], created_at_ms: record.created_at_ms });
      } else if (record.role === "assistant") {
        turn.final_answer = record.content;
      }
    } else if (record.type === "event") {
      const payload = typeof record.payload === "object" && record.payload !== null ? record.payload as Record<string, unknown> : { kind: record.kind, content: record.content };
      const source = typeof record.source === "string" ? record.source : "history";
      turn.events.push({
        event_id: `history_event_${record.turn_id}_${record.created_at_ms}_${turn.events.length}`,
        source,
        payload,
        created_at_ms: record.created_at_ms,
      });
    }
    turns.set(record.turn_id, turn);
  }
  return Array.from(turns.values())
    .map((turn) => ({
      ...turn,
      user_entries: [...turn.user_entries].sort((left, right) => left.created_at_ms - right.created_at_ms),
      events: [...turn.events].sort((left, right) => left.created_at_ms - right.created_at_ms),
    }))
    .sort((left, right) => left.created_at_ms - right.created_at_ms);
}

type ChatMessageHistoryRecord = Extract<ChatHistoryRecord, { type: "message" }> & { role: "user" | "assistant" };

function isChatMessageHistoryRecord(record: ChatHistoryRecord): record is ChatMessageHistoryRecord {
  return record.type === "message" && (record.role === "user" || record.role === "assistant");
}

function messagesFromHistoryRecords(records: ChatHistoryRecord[]): ChatMessage[] {
  return records
    .filter(isChatMessageHistoryRecord)
    .sort((left, right) => left.created_at_ms - right.created_at_ms)
    .map((record) => ({
      id: `history_msg_${record.turn_id}_${record.created_at_ms}_${record.role}`,
      role: record.role,
      text: record.content,
      created_at_ms: record.created_at_ms,
    }));
}

export function appendTurnEvent(session: Session, turnId: string | null | undefined, event: WebTurnEvent): Session {
  if (!turnId) return session;
  if (!turnEventBelongsToSession(session, event)) return session;
  return {
    ...session,
    turns: session.turns.map((turn) => turn.turn_id === turnId
      ? {
          ...turn,
          final_answer: finalAnswerFromTurnEvent(session, event) ?? turn.final_answer,
          events: turn.events.some((existing) => existing.event_id === event.event_id)
            ? turn.events
            : trimTurnEvents([...turn.events, event]),
        }
      : turn),
  };
}

function turnEventSessionId(event: WebTurnEvent) {
  const direct = event.payload.session_id;
  if (typeof direct === "string") return direct;
  const nested = event.payload.payload;
  if (nested && typeof nested === "object") {
    const sessionId = (nested as Record<string, unknown>).session_id;
    if (typeof sessionId === "string") return sessionId;
  }
  return undefined;
}

function turnEventBelongsToSession(session: Session, event: WebTurnEvent): boolean {
  if (event.source !== "core_topic") return true;
  const topicEvent = event.payload as unknown as CoreTopicEvent;
  if (topicEvent.session_id !== session.session_id) return false;
  const isLifecycle = topicEvent.topic?.name === "core.lifecycle";
  if (!isLifecycle && topicEvent.worker_id && !session.workers.some((worker) => worker.worker_id === topicEvent.worker_id)) return false;
  if (!isLifecycle && topicEvent.context_id && !session.contexts.some((context) => context.context_id === topicEvent.context_id)) return false;
  return true;
}

function finalAnswerFromTurnEvent(session: Session, event: WebTurnEvent) {
  if (event.source !== "core_topic") return undefined;
  const topic = event.payload.topic;
  const payload = event.payload.payload;
  if (!topic || typeof topic !== "object" || (topic as Record<string, unknown>).name !== "core.model.response") return undefined;
  if (!payload || typeof payload !== "object") return undefined;
  if ((payload as Record<string, unknown>).runtime_phase === "toolgen") return undefined;
  const workerId = event.payload.worker_id;
  if (typeof workerId === "string" && workerId !== session.primary_worker_id) return undefined;
  const finalAnswer = (payload as Record<string, unknown>).final_answer;
  return typeof finalAnswer === "string" && finalAnswer.trim() ? finalAnswer : undefined;
}

export function finishTurn(session: Session, turnId: string | null | undefined, completion: TurnCompletion): Session {
  const workers = session.workers.map((worker) => worker.state === "working"
    ? { ...worker, state: "ready" }
    : worker);
  const state = aggregateSessionState(workers, "ready");
  if (!turnId) return { ...session, workers, state };
  return {
    ...session,
    workers,
    state,
    active_turn_id: session.active_turn_id === turnId ? null : session.active_turn_id,
    turns: session.turns.map((turn) => turn.turn_id === turnId ? { ...turn, state: "finished", completion } : turn),
  };
}

export function updateSessionWorkerState(session: Session, workerId: string, state: string): Session {
  let found = false;
  const workers = session.workers.map((worker) => {
    if (worker.worker_id !== workerId) return worker;
    found = true;
    return { ...worker, state };
  });
  return found ? { ...session, workers, state: aggregateSessionState(workers, session.state) } : session;
}

function aggregateSessionState(workers: Session["workers"], fallback: string) {
  if (workers.length === 0) return fallback;
  if (workers.some((worker) => worker.state === "working")) return "working";
  if (workers.some((worker) => worker.state === "error")) return "error";
  if (workers.every((worker) => worker.state === "stopped")) return "stopped";
  return "ready";
}

export function turnLiveUsage(turn: WebTurn): { total: import("./protocol").UsageStats; latest: import("./protocol").UsageStats } | undefined {
  let latest: import("./protocol").UsageStats | undefined;
  const total: import("./protocol").UsageStats = {};
  for (const event of turn.events) {
    if (event.source !== "worker_activity" || event.payload.kind !== "model_response") continue;
    const usage = event.payload.usage;
    if (!usage || typeof usage !== "object") continue;
    const current = usage as import("./protocol").UsageStats;
    latest = current;
    for (const field of USAGE_FIELDS) {
      const value = current[field];
      if (typeof value === "number" && Number.isFinite(value)) total[field] = (total[field] ?? 0) + value;
    }
  }
  return latest ? { total, latest } : undefined;
}

export function sessionContextUsage(session: Session): import("./protocol").UsageStats | undefined {
  for (let index = session.turns.length - 1; index >= 0; index -= 1) {
    const turn = session.turns[index];
    if (turn.state === "restored") continue;
    const live = turnLiveUsage(turn);
    if (live) return live.latest;
    const latest = turn.completion?.latest_usage;
    if (latest) return latest;
  }
  return undefined;
}

export function decisionKey(decision: Decision) {
  const requestId = typeof decision.event.payload.request_id === "string" ? decision.event.payload.request_id : "";
  return [
    decision.event.session_id,
    decision.event.context_id ?? "",
    decision.event.worker_id ?? "",
    decision.event.topic.name,
    requestId,
  ].join("\u0000");
}

export function enqueueDecision(decisions: Decision[], incoming: Decision) {
  const incomingKey = decisionKey(incoming);
  const exists = decisions.some((decision) => {
    return decisionKey(decision) === incomingKey;
  });
  return exists ? decisions : [...decisions, incoming];
}

export function clearDecisionsForSession(decisions: Decision[], sessionId: string) {
  return decisions.filter((decision) => decision.event.session_id !== sessionId);
}

export function clearDecisionsForWorker(decisions: Decision[], sessionId: string, workerId: string) {
  return decisions.filter((decision) => !(
    decision.event.session_id === sessionId && decision.event.worker_id === workerId
  ));
}

/**
 * The host broadcasts a session-aware topic stream so one browser can switch
 * between agents. This reducer is intentionally strict: an event can mutate
 * only the session named by the canonical core topic envelope.
 */
export function applyCoreTopicToSession(
  session: Session,
  event: CoreTopicEvent,
  makeAssistantMessage: (text: string, id?: string) => ChatMessage,
): Session {
  if (session.session_id !== event.session_id) return session;
  const isLifecycle = event.topic.name === "core.lifecycle";
  if (!isLifecycle && event.worker_id && !session.workers.some((worker) => worker.worker_id === event.worker_id)) return session;
  if (!isLifecycle && event.context_id && !session.contexts.some((context) => context.context_id === event.context_id)) return session;
  const contextState = event.payload.context_state;
  const reportedDir = contextState && typeof contextState === "object" && typeof (contextState as Record<string, unknown>).cwd === "string"
    ? (contextState as Record<string, string>).cwd
    : undefined;
  const targetContextId = event.context_id ?? session.active_context_id;
  let contexts = reportedDir
    ? session.contexts.map((context) => context.context_id === targetContextId ? { ...context, current_dir: reportedDir } : context)
    : session.contexts;
  const currentDir = reportedDir && targetContextId === session.active_context_id ? reportedDir : session.current_dir;
  let workers = session.workers;
  if (event.topic.name === "core.model.response") {
    if (event.payload.runtime_phase === "toolgen") return session;
    const finalAnswer = typeof event.payload.final_answer === "string" ? event.payload.final_answer.trim() : "";
    const messageId = typeof event.payload.ui_message_id === "string" ? event.payload.ui_message_id : undefined;
    const isPrimary = !event.worker_id || event.worker_id === session.primary_worker_id;
    const nextMessages = finalAnswer && isPrimary ? trimMessages([...session.messages, makeAssistantMessage(finalAnswer, messageId)]) : session.messages;
    const updated = event.worker_id
      ? updateSessionWorkerState(session, event.worker_id, event.payload.continue_work === true ? "working" : "ready")
      : { ...session, state: event.payload.continue_work === true ? "working" : "ready" };
    return { ...updated, contexts, current_dir: currentDir, messages: nextMessages };
  }
  if (event.topic.name === "core.lifecycle") {
    const worker = event.payload.worker;
    if (event.worker_id && event.context_id) {
      const contextExists = contexts.some((context) => context.context_id === event.context_id);
      if (!contextExists) {
        contexts = [
          ...contexts,
          {
            context_id: event.context_id,
            current_dir: reportedDir ?? session.current_dir,
            worker_ids: [event.worker_id],
          },
        ];
      } else {
        contexts = contexts.map((context) => (
          context.context_id === event.context_id && !context.worker_ids.includes(event.worker_id!)
            ? { ...context, worker_ids: [...context.worker_ids, event.worker_id!] }
            : context
        ));
      }
      if (!workers.some((item) => item.worker_id === event.worker_id)) {
        const workerPayload = worker && typeof worker === "object" ? worker as Record<string, unknown> : {};
        workers = [
          ...workers,
          {
            worker_id: event.worker_id,
            context_id: event.context_id,
            display_name: typeof workerPayload.display_name === "string" ? workerPayload.display_name : event.worker_id,
            ordinal: typeof workerPayload.ordinal === "number" ? workerPayload.ordinal : workers.length,
            state: "ready",
            parent_worker_id: typeof workerPayload.parent_worker_id === "string" ? workerPayload.parent_worker_id : null,
          },
        ];
      }
    }
    const displayName = worker && typeof worker === "object" && typeof (worker as Record<string, unknown>).display_name === "string"
      ? (worker as Record<string, string>).display_name
      : session.display_name;
    const maxLlmInputTokens = typeof event.payload.max_llm_input_tokens === "number"
      ? event.payload.max_llm_input_tokens
      : session.max_llm_input_tokens;
    workers = event.worker_id
      ? workers.map((item) => item.worker_id === event.worker_id ? { ...item, display_name: displayName, state: "ready" } : item)
      : workers;
    return { ...session, workers, contexts, current_dir: currentDir, max_llm_input_tokens: maxLlmInputTokens, state: aggregateSessionState(workers, session.state) };
  }
  return currentDir === session.current_dir && contexts === session.contexts ? session : { ...session, contexts, current_dir: currentDir };
}

/** Attaches completion telemetry to the exact final answer produced by this turn. */
export function attachTurnCompletion(
  session: Session,
  messageId: string | null | undefined,
  completion: TurnCompletion,
): Session {
  const state = aggregateSessionState(session.workers, "ready");
  if (!messageId) return { ...session, state };
  let updated = false;
  const messages = session.messages.map((message) => {
    if (message.id !== messageId) return message;
    updated = true;
    return { ...message, completion };
  });
  return updated ? { ...session, state, messages } : { ...session, state };
}

export function activityFromTopic(event: CoreTopicEvent): Activity | null {
  const payload = event.payload;
  const label = (value: unknown) => typeof value === "string" ? value : "";
  switch (event.topic.name) {
    case "core.model.response": {
      const freeTalk = label(payload.free_talk);
      const progress = label(payload.progress);
      const detail = [freeTalk, progress].filter((text) => text.trim()).join("\n\n");
      return detail ? { id: crypto.randomUUID(), sessionId: event.session_id, tone: "thinking", title: "", detail, createdAt: Date.now() } : null;
    }
    case "core.model.repair":
      return { id: crypto.randomUUID(), sessionId: event.session_id, tone: "warning", title: `⚠️ 模型回复偏离协议，重试 (${payload.attempt ?? 0}/${payload.max_attempts ?? 5})`, detail: label(payload.issue), createdAt: Date.now() };
    case "core.action": {
      const action = label(payload.action) || "action";
      const status = label(payload.status) || label(payload.event) || "running";
      const statusText = displayToolStatus(status);
      const input = payload.input && typeof payload.input === "object" ? payload.input as Record<string, unknown> : undefined;
      const command = action === "run_bash" && input
        ? [input.cmd, input.loop_cmd].find((value): value is string => typeof value === "string")
        : undefined;
      const detail = command ? "" : formatToolArguments(input);
      return {
        id: crypto.randomUUID(),
        sessionId: event.session_id,
        tone: "action",
        title: `${toolDisplayName(action)} · ${statusText}`,
        tool_name: action,
        tool_status: status,
        detail,
        code: command,
        code_language: command ? "bash" : undefined,
        createdAt: Date.now(),
      };
    }
    case "core.context.compact": {
      const before = typeof payload.estimated_before_tokens === "number" ? payload.estimated_before_tokens : undefined;
      const after = typeof payload.estimated_after_tokens === "number" ? payload.estimated_after_tokens : undefined;
      return {
        id: crypto.randomUUID(),
        sessionId: event.session_id,
        tone: "notice",
        kind: "context_compact",
        title: "Context compacted",
        detail: `${before ?? "?"} tokens → ${after ?? "?"} tokens`,
        before_tokens: before,
        after_tokens: after,
        createdAt: Date.now(),
      };
    }
    case "core.toolgen": {
      const phase = label(payload.phase);
      const tool = payload.tool && typeof payload.tool === "object" ? payload.tool as Record<string, unknown> : undefined;
      const toolName = tool ? label(tool.name) : "";
      const retrospect = label(payload.retrospect);
      const error = label(payload.error);
      const title = phase === "published"
        ? `ToolGen: 已生成并验证 ${toolName || "可复用工具"}`
        : phase === "started"
          ? "ToolGen: 正在评估…"
          : "ToolGen: 生成失败";
      return {
        id: crypto.randomUUID(),
        sessionId: event.session_id,
        tone: phase === "published" || phase === "started" ? "notice" : "warning",
        kind: "toolgen",
        toolgen_phase: phase,
        title,
        detail: error || retrospect,
        createdAt: Date.now(),
      };
    }
    case "core.work_instruction_load":
      return null;
    default:
      return null;
  }
}

export function toolDisplayName(name: string) {
  if (name === "run_bash") return "Bash";
  if (name === "memmgr") return "MemMgr";
  if (name === "capmgr") return "CapMgr";
  if (name === "self_tool") return "Self tool";
  return name;
}

function displayToolStatus(status: string) {
  if (status === "background_running") return "background running";
  if (status === "timeout") return "timed out";
  return status.replaceAll("_", " ");
}

function formatToolArguments(input: Record<string, unknown> | undefined) {
  if (!input) return "";
  return Object.entries(input)
    .map(([key, value]) => `${key}=${formatToolValue(value)}`)
    .join(" ");
}

function formatToolValue(value: unknown): string {
  if (typeof value === "string") return JSON.stringify(value);
  if (value === null || typeof value === "boolean" || typeof value === "number") return String(value);
  return JSON.stringify(value);
}

export function requestDecision(event: CoreTopicEvent, turnId?: string | null): Decision | null {
  if (event.state.name !== "waiting_user" && event.state.name !== "waiting_user_with_timeout") return null;
  const payload = event.payload;
  const request = payload.request && typeof payload.request === "object" ? payload.request as Record<string, unknown> : {};
  const workInstructionFiles = Array.isArray(request.file_names)
    ? request.file_names.filter((name): name is string => typeof name === "string")
    : [];
  const detail = typeof request.command === "string"
    ? request.command
    : workInstructionFiles.length > 0
      ? `Load ${workInstructionFiles.join(", ")} from ${typeof request.directory === "string" ? request.directory : "this workspace"}?`
    : typeof request.message === "string"
      ? request.message
      : typeof payload.kind === "string"
        ? payload.kind
        : "Timem needs your decision before it can continue.";
  return { event, turnId: turnId ?? undefined, title: "Decision required", detail };
}
