import { Activity, ChatHistoryRecord, ChatMessage, CoreTopicEvent, Decision, Session, TurnCompletion, WebTurn, WebTurnEvent } from "./protocol";

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

function actionLifecycleKey(event: WebTurnEvent) {
  if (event.source !== "core_topic") return undefined;
  const topicEvent = event.payload as unknown as CoreTopicEvent;
  if (topicEvent.topic?.name !== "core.action") return undefined;
  const action = typeof topicEvent.payload.action === "string" ? topicEvent.payload.action : "";
  if (!action) return undefined;
  return `${action}:${JSON.stringify(topicEvent.payload.input ?? null)}`;
}

export function coalesceActionLifecycle(events: WebTurnEvent[]) {
  const visible: WebTurnEvent[] = [];
  const pendingStarts = new Map<string, number[]>();
  for (const event of events) {
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
  return {
    ...session,
    turns: trimTurns([...earlier, ...session.turns]),
    messages: trimMessages([...messagesFromHistoryRecords(records), ...session.messages]),
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

function finalAnswerFromTurnEvent(session: Session, event: WebTurnEvent) {
  if (event.source !== "core_topic") return undefined;
  const topic = event.payload.topic;
  const payload = event.payload.payload;
  if (!topic || typeof topic !== "object" || (topic as Record<string, unknown>).name !== "core.model.response") return undefined;
  if (!payload || typeof payload !== "object") return undefined;
  const workerId = event.payload.worker_id;
  if (typeof workerId === "string" && workerId !== session.primary_worker_id) return undefined;
  const finalAnswer = (payload as Record<string, unknown>).final_answer;
  return typeof finalAnswer === "string" && finalAnswer.trim() ? finalAnswer : undefined;
}

export function finishTurn(session: Session, turnId: string | null | undefined, completion: TurnCompletion): Session {
  const workers = session.workers.map((worker) => worker.worker_id === session.primary_worker_id
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
    latest = usage as import("./protocol").UsageStats;
    for (const field of USAGE_FIELDS) {
      const value = latest[field];
      if (typeof value === "number" && Number.isFinite(value)) total[field] = (total[field] ?? 0) + value;
    }
  }
  return latest ? { total, latest } : undefined;
}

export function sessionContextUsage(session: Session): import("./protocol").UsageStats | undefined {
  for (let index = session.turns.length - 1; index >= 0; index -= 1) {
    const turn = session.turns[index];
    const live = turnLiveUsage(turn);
    if (live) return live.latest;
    const latest = turn.completion?.latest_usage;
    if (latest) return latest;
  }
  return undefined;
}

export function enqueueDecision(decisions: Decision[], incoming: Decision) {
  const incomingRequestId = typeof incoming.event.payload.request_id === "string" ? incoming.event.payload.request_id : "";
  const exists = decisions.some((decision) => {
    const requestId = typeof decision.event.payload.request_id === "string" ? decision.event.payload.request_id : "";
    return decision.event.session_id === incoming.event.session_id
      && decision.event.topic.name === incoming.event.topic.name
      && requestId === incomingRequestId;
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
  if (event.worker_id && !session.workers.some((worker) => worker.worker_id === event.worker_id)) return session;
  const contextState = event.payload.context_state;
  const reportedDir = contextState && typeof contextState === "object" && typeof (contextState as Record<string, unknown>).cwd === "string"
    ? (contextState as Record<string, string>).cwd
    : undefined;
  const targetContextId = event.context_id ?? session.active_context_id;
  const contexts = reportedDir
    ? session.contexts.map((context) => context.context_id === targetContextId ? { ...context, current_dir: reportedDir } : context)
    : session.contexts;
  const currentDir = reportedDir && targetContextId === session.active_context_id ? reportedDir : session.current_dir;
  if (event.topic.name === "core.model.response") {
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
    const displayName = worker && typeof worker === "object" && typeof (worker as Record<string, unknown>).display_name === "string"
      ? (worker as Record<string, string>).display_name
      : session.display_name;
    const maxLlmInputTokens = typeof event.payload.max_llm_input_tokens === "number"
      ? event.payload.max_llm_input_tokens
      : session.max_llm_input_tokens;
    const workers = event.worker_id
      ? session.workers.map((item) => item.worker_id === event.worker_id ? { ...item, display_name: displayName, state: "ready" } : item)
      : session.workers;
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
      return freeTalk ? { id: crypto.randomUUID(), sessionId: event.session_id, tone: "thinking", title: "", detail: freeTalk, createdAt: Date.now() } : null;
    }
    case "core.model.repair":
      return { id: crypto.randomUUID(), sessionId: event.session_id, tone: "warning", title: `Response format repair (${payload.attempt ?? 0}/${payload.max_attempts ?? 5})`, detail: label(payload.issue), createdAt: Date.now() };
    case "core.action": {
      const action = label(payload.action) || "action";
      const status = label(payload.status) || label(payload.event) || "running";
      const input = payload.input && typeof payload.input === "object" ? payload.input as Record<string, unknown> : undefined;
      const command = action === "run_bash" && input
        ? [input.cmd, input.loop_cmd].find((value): value is string => typeof value === "string")
        : undefined;
      const detail = command ? "" : formatToolArguments(input);
      return {
        id: crypto.randomUUID(),
        sessionId: event.session_id,
        tone: "action",
        title: `${action} · ${status}`,
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
    case "core.work_instruction_load":
      return null;
    default:
      return null;
  }
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
