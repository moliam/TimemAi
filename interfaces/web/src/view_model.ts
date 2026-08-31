import {
  Activity,
  ChatHistoryRecord,
  ChatMessage,
  ClientCommand,
  clientId,
  CoreTopicEvent,
  Decision,
  Session,
  SessionWorker,
  TurnCompletion,
  VersionedTurnProjection,
  WebTurn,
  WebTurnEvent,
} from "./protocol";
import {
  humanizeToolStatus,
  TOOL_STATUS_BACKGROUND_RUNNING,
} from "./tool_status";

export const MAX_RENDERED_MESSAGES = 1000;
// The host delivers restored history in 200-turn pages.  Keep several pages in
// the browser so scrolling upward actually reveals the page just requested,
// while the thread component still renders only its visible window.
export const MAX_CLIENT_TURNS = 1200;

const USAGE_FIELDS = [
  "llm_calls",
  "repair_calls",
  "tool_calls",
  "mem_reads",
  "mem_writes",
  "prompt_tokens",
  "completion_tokens",
  "total_tokens",
  "cached_tokens",
  "cache_created_tokens",
  "shrunk_tokens",
] as const;

export function trimMessages<T>(messages: T[]) {
  return messages.length > MAX_RENDERED_MESSAGES
    ? messages.slice(-MAX_RENDERED_MESSAGES)
    : messages;
}

export function normalizeCopiedUserMessageText(text: string): string {
  return text.replace(/(?:\r?\n)+$/, "");
}

export function applyTurnProjection(
  session: Session,
  incoming: VersionedTurnProjection,
): Session {
  const currentRevision = session.turn_projection?.revision ?? 0;
  if (incoming.revision <= currentRevision) return session;
  return { ...session, turn_projection: incoming };
}

export function applySessionRuntimeProfile(
  session: Session,
  runtimeProfile: NonNullable<Session["runtime_profile"]>,
): Session {
  return {
    ...session,
    runtime_profile: runtimeProfile,
    max_llm_input_tokens: runtimeProfile.max_llm_input_tokens,
  };
}

export function trimTurns<T>(turns: T[]) {
  return turns.length > MAX_CLIENT_TURNS
    ? turns.slice(-MAX_CLIENT_TURNS)
    : turns;
}

export type TurnTimelinePlacement = {
  createdAtMs: number;
  resumedAfterRestart: boolean;
};

export function turnShouldRenderInTimeline(turn: WebTurn): boolean {
  if (turn.state !== "pending") return true;
  return (
    // A Host-accepted user task is already an independent visible Turn even
    // while Core dispatch remains behind a private terminal barrier.
    turn.user_entries.some(
      (entry) => entry.kind === "task" && !!entry.text.trim(),
    ) ||
    turn.events.length > 0 ||
    turn.user_entries.some((entry) => entry.kind === "approval") ||
    turn.sub_answers.length > 0 ||
    !!turn.final_answer ||
    !!turn.completion
  );
}

export function turnTimelinePlacement(
  turn: WebTurn,
  restartMarkers: ChatMessage[],
): TurnTimelinePlacement {
  let resumedAtMs: number | undefined;
  for (const marker of restartMarkers) {
    if (marker.created_at_ms <= turn.created_at_ms) continue;
    const hasActivityAfterRestart =
      turn.state === "working" ||
      turn.user_entries.some(
        (entry) => entry.created_at_ms >= marker.created_at_ms,
      ) ||
      turn.events.some((event) => event.created_at_ms >= marker.created_at_ms);
    if (
      hasActivityAfterRestart &&
      (resumedAtMs === undefined || marker.created_at_ms > resumedAtMs)
    ) {
      resumedAtMs = marker.created_at_ms;
    }
  }
  return resumedAtMs === undefined
    ? { createdAtMs: turn.created_at_ms, resumedAfterRestart: false }
    : { createdAtMs: resumedAtMs, resumedAfterRestart: true };
}

export type TurnTimelineOrderItem = {
  type: "turn" | "restart";
  createdAtMs: number;
  resumedAfterRestart: boolean;
  id: string;
};

export function compareTurnTimelineItems(
  left: TurnTimelineOrderItem,
  right: TurnTimelineOrderItem,
): number {
  const timeOrder = left.createdAtMs - right.createdAtMs;
  if (timeOrder !== 0) return timeOrder;
  if (left.type !== right.type) {
    const turn = left.type === "turn" ? left : right;
    if (turn.resumedAfterRestart) return left.type === "restart" ? -1 : 1;
    return left.type === "turn" ? -1 : 1;
  }
  return left.id.localeCompare(right.id);
}

export function visibleRuntimeRestartMarkers(
  turns: WebTurn[],
  markers: ChatMessage[],
): ChatMessage[] {
  const timeline = [
    ...turns.map((turn) => ({
      type: "turn" as const,
      createdAtMs: turn.created_at_ms,
      id: turn.turn_id,
    })),
    ...markers.map((marker) => ({
      type: "restart" as const,
      createdAtMs: marker.created_at_ms,
      id: marker.id,
      marker,
    })),
  ].sort(
    (left, right) =>
      left.createdAtMs - right.createdAtMs ||
      (left.type === right.type
        ? left.id.localeCompare(right.id)
        : left.type === "turn"
          ? -1
          : 1),
  );

  const visible: ChatMessage[] = [];
  let workSinceLastRestart = true;
  for (const item of timeline) {
    if (item.type === "turn") {
      workSinceLastRestart = true;
      continue;
    }
    if (!workSinceLastRestart && visible.length > 0) {
      visible[visible.length - 1] = item.marker;
    } else {
      visible.push(item.marker);
    }
    workSinceLastRestart = false;
  }
  return visible;
}

export function tailPath(path: string, maxChars = 28) {
  if (path.length <= maxChars) return path;
  return `…${path.slice(-(Math.max(2, maxChars) - 1))}`;
}

export function workspacePathLabel(path: string) {
  const normalized = path.replace(/[\\/]+$/, "");
  const leaf = normalized.split(/[\\/]/).at(-1) || normalized;
  return normalized === leaf ? leaf : `…/${leaf}`;
}

export function runtimeConnectionLabel(
  connected: boolean,
  snapshotReady: boolean,
  runtimeEverConnected: boolean,
  reconnectAttempt = 0,
) {
  if (!connected && runtimeEverConnected) {
    return reconnectAttempt >= 3
      ? "Runtime unavailable. Restart timem."
      : "Connection lost. Reconnecting…";
  }
  if (!connected) return "Connecting to runtime…";
  return snapshotReady ? "Runtime connected" : "Syncing runtime…";
}

export function sessionInteractionLockReason(
  pendingMemSwitch: boolean,
  connected: boolean,
  runtimeEverConnected: boolean,
  reconnectAttempt = 0,
) {
  if (pendingMemSwitch) return "Mem switch is in progress";
  if (!connected && runtimeEverConnected) {
    return reconnectAttempt >= 3
      ? "Runtime unavailable. Restart timem."
      : "Connection lost. Reconnecting…";
  }
  return "Waiting for runtime snapshot…";
}

export type ComposerSendDecision =
  | {
      kind: "skip";
      reason: "no_session" | "empty_text" | "cancelling" | "mem_switching";
    }
  | {
      kind: "send";
      command: Extract<
        ClientCommand,
        { type: "turn_submit" | "turn_supplement" }
      >;
      text: string;
      clearDraftOnSuccess: true;
    };

export type SessionRenameDecision =
  | {
      kind: "skip";
      reason: "no_session" | "empty_name" | "already_pending" | "mem_switching";
    }
  | {
      kind: "send";
      command: Extract<ClientCommand, { type: "session_rename" }>;
      displayName: string;
    };

export type SessionCreateDecision =
  | { kind: "skip"; reason: "empty_workspace" | "creating" | "mem_switching" }
  | {
      kind: "send";
      command: Extract<ClientCommand, { type: "session_create" }>;
      displayName: string;
      workspaceDir: string;
      env: Record<string, string>;
    };

export type DraftSubmissionLock = { current: boolean };
export type SessionDraftSubmissionLocks = { current: Set<string> };
export type SessionDrafts = Record<string, string>;

export function reserveDraftSubmission(
  lock: DraftSubmissionLock,
  draft: string,
): string | null {
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

export function draftForSession(
  drafts: SessionDrafts,
  sessionId: string | undefined,
): string {
  return sessionId ? (drafts[sessionId] ?? "") : "";
}

export function setSessionDraft(
  drafts: SessionDrafts,
  sessionId: string | undefined,
  value: string,
): SessionDrafts {
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
  return current.trim() === submittedText
    ? setSessionDraft(drafts, sessionId, "")
    : drafts;
}

export function releaseSessionDraftSubmission(
  locks: SessionDraftSubmissionLocks,
  sessionId: string,
): boolean {
  return locks.current.delete(sessionId);
}

export function pruneSessionDrafts(
  drafts: SessionDrafts,
  liveSessionIds: Iterable<string>,
): SessionDrafts {
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

export function resolveActiveSessionId(
  currentSessionId: string,
  sessions: Pick<Session, "session_id">[],
): string {
  if (
    currentSessionId &&
    sessions.some((session) => session.session_id === currentSessionId)
  ) {
    return currentSessionId;
  }
  return sessions[0]?.session_id ?? "";
}

export type SessionWorkerTreeRow = {
  worker: SessionWorker;
  depth: number;
  isLast: boolean;
};

export function sessionWorkerTreeRows(
  workers: readonly SessionWorker[],
): SessionWorkerTreeRow[] {
  const ordered = [...workers].sort(
    (left, right) =>
      left.ordinal - right.ordinal ||
      left.worker_id.localeCompare(right.worker_id),
  );
  const byId = new Map(ordered.map((worker) => [worker.worker_id, worker]));
  const children = new Map<string, SessionWorker[]>();
  const roots: SessionWorker[] = [];
  for (const worker of ordered) {
    const parentId = worker.parent_worker_id;
    if (!parentId || parentId === worker.worker_id || !byId.has(parentId)) {
      roots.push(worker);
      continue;
    }
    children.set(parentId, [...(children.get(parentId) ?? []), worker]);
  }
  const rows: SessionWorkerTreeRow[] = [];
  const visited = new Set<string>();
  const append = (worker: SessionWorker, depth: number, isLast: boolean) => {
    if (visited.has(worker.worker_id)) return;
    visited.add(worker.worker_id);
    rows.push({ worker, depth, isLast });
    const nested = children.get(worker.worker_id) ?? [];
    nested.forEach((child, index) =>
      append(child, depth + 1, index === nested.length - 1),
    );
  };
  roots.forEach((worker, index) =>
    append(worker, 0, index === roots.length - 1),
  );
  // A malformed parent cycle has no root. Keep every worker visible instead of
  // dropping it from the Session tree.
  ordered
    .filter((worker) => !visited.has(worker.worker_id))
    .forEach((worker, index, remaining) => {
      append(worker, 0, index === remaining.length - 1);
    });
  return rows;
}

export type TurnInteractionPhase =
  | { kind: "idle" }
  | { kind: "submit_persisted"; commandId: string }
  | { kind: "host_pending"; turnId: string; commandId?: string }
  | { kind: "working"; turnId?: string; commandId?: string }
  | { kind: "cancelling"; turnId?: string; commandId?: string };

export function turnCommandId(
  session: Session,
  turnId: string | null | undefined,
) {
  if (!turnId) return undefined;
  const turn = session.turns.find((candidate) => candidate.turn_id === turnId);
  return turn?.user_entries.find((entry) => entry.kind === "task")?.command_id;
}

export function sessionCancellationApplies(
  session: Session | undefined,
): boolean {
  const cancellingTurnId = session?.cancelling_turn_id;
  if (!cancellingTurnId) return false;
  const cancellingTurn = session.turns.find(
    (turn) => turn.turn_id === cancellingTurnId,
  );
  // A terminal Turn projection is authoritative for composer admission. Host
  // fields such as cancelling_turn_id may be cleared by a later projection,
  // but must not keep the browser in the Stop state after the chat is terminal.
  return !cancellingTurn ||
    (cancellingTurn.state !== "finished" && !cancellingTurn.completion);
}

export function shouldRenderTurnWorkFrame(
  turnState: WebTurn["state"],
  isCancelling: boolean,
  hasVisibleProcess: boolean,
): boolean {
  return (turnState === "working" && !isCancelling) || hasVisibleProcess;
}

export function turnElapsedMs(
  createdAtMs: number,
  nowMs: number,
  endedAtMs?: number | null,
): number {
  return Math.max(0, (endedAtMs ?? nowMs) - createdAtMs);
}

export function turnInteractionPhase(
  session: Session | undefined,
  localSubmitCommandId: string | undefined,
  isCancelling: boolean,
): TurnInteractionPhase {
  const pendingTurnId = session?.pending_turn_id ?? undefined;
  const activeTurnId = session?.active_turn_id ?? undefined;
  const turnId = pendingTurnId ?? activeTurnId;
  const commandId = session ? turnCommandId(session, turnId) : undefined;
  if (isCancelling || sessionCancellationApplies(session))
    return {
      kind: "cancelling",
      turnId,
      commandId: commandId ?? localSubmitCommandId,
    };
  const projectedTurn = session?.turns.find(
    (turn) => turn.turn_id === turnId,
  );
  if (projectedTurn?.state === "finished" || projectedTurn?.completion)
    return { kind: "idle" };
  if (pendingTurnId)
    return { kind: "host_pending", turnId: pendingTurnId, commandId };
  if (activeTurnId || session?.state === "working") {
    return { kind: "working", turnId: activeTurnId, commandId };
  }
  if (localSubmitCommandId) {
    const alreadyRecorded = session?.turns.some((turn) =>
      turn.user_entries.some(
        (entry) => entry.command_id === localSubmitCommandId,
      ),
    );
    const acceptedForFutureTurn = session?.message_queue?.items.some(
      (item) => item.command_id === localSubmitCommandId,
    );
    // Once Host projects the command in its Session-owned future-message FIFO,
    // it is accepted queued input, not an active/pending Turn that can be
    // stopped. The queue panel renders it and Host decides when it may start.
    if (!alreadyRecorded && !acceptedForFutureTurn)
      return { kind: "submit_persisted", commandId: localSubmitCommandId };
  }
  return { kind: "idle" };
}

export function composerPrimaryAction(
  phase: TurnInteractionPhase,
  text: string,
): "send" | "stop" {
  if (phase.kind === "cancelling") return "send";
  return phase.kind !== "idle" && !text.trim() ? "stop" : "send";
}

export function composerSendDecision(
  session: Pick<Session, "session_id" | "state"> | undefined,
  text: string,
  isCancelling: boolean,
  isMemSwitching = false,
  attachmentIds?: readonly string[],
  forceSupplement = false,
  forceNewTurn = false,
): ComposerSendDecision {
  if (!session) return { kind: "skip", reason: "no_session" };
  const trimmed = text.trim();
  if (!trimmed) return { kind: "skip", reason: "empty_text" };
  if (isMemSwitching) return { kind: "skip", reason: "mem_switching" };
  return {
    kind: "send",
    text: trimmed,
    clearDraftOnSuccess: true,
    command:
      forceSupplement && !forceNewTurn
        ? {
            type: "turn_supplement",
            session_id: session.session_id,
            text: trimmed,
            ...(attachmentIds === undefined
              ? {}
              : { attachment_ids: [...attachmentIds] }),
          }
        : {
            type: "turn_submit",
            session_id: session.session_id,
            text: trimmed,
            ...(attachmentIds === undefined
              ? {}
              : { attachment_ids: [...attachmentIds] }),
          },
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
  if (pendingSessionIds.has(sessionId))
    return { kind: "skip", reason: "already_pending" };
  return {
    kind: "send",
    displayName,
    command: {
      type: "session_rename",
      session_id: sessionId,
      display_name: displayName,
    },
  };
}

export function sessionCreateDecision(
  displayNameDraft: string,
  workspaceDirDraft: string,
  envDraft: Record<string, string>,
  groupId: string | null,
  creating: boolean,
  isMemSwitching = false,
): SessionCreateDecision {
  if (isMemSwitching) return { kind: "skip", reason: "mem_switching" };
  if (creating) return { kind: "skip", reason: "creating" };
  const workspaceDir = workspaceDirDraft.trim();
  if (!workspaceDir) return { kind: "skip", reason: "empty_workspace" };
  const displayName = displayNameDraft.trim();
  const env = Object.fromEntries(
    Object.entries(envDraft)
      .map(([key, value]) => [key, value.trim()])
      .filter(([, value]) => value),
  );
  return {
    kind: "send",
    displayName,
    workspaceDir,
    env,
    command: {
      type: "session_create",
      ...(displayName ? { display_name: displayName } : {}),
      workspace_dir: workspaceDir,
      group_id: groupId,
      env,
    },
  };
}

function actionLifecycleKey(event: WebTurnEvent) {
  if (event.source !== "core_topic") return undefined;
  const topicEvent = event.payload as unknown as CoreTopicEvent;
  if (topicEvent.topic?.name !== "core.action") return undefined;
  const action =
    typeof topicEvent.payload.action === "string"
      ? topicEvent.payload.action
      : "";
  if (!action) return undefined;
  const actionId =
    typeof topicEvent.payload.action_id === "string"
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
  const phase =
    typeof topicEvent.payload.phase === "string"
      ? topicEvent.payload.phase
      : "";
  return phase
    ? { key: `toolgen:${topicEvent.context_id ?? "unknown"}`, phase }
    : undefined;
}

export function coalesceActionLifecycle(events: WebTurnEvent[]) {
  const visible: WebTurnEvent[] = [];
  const pendingStarts = new Map<string, number[]>();
  const pendingBackgroundFinishes = new Map<string, number[]>();
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
    const lifecycle =
      typeof topicEvent.payload.event === "string"
        ? topicEvent.payload.event
        : topicEvent.topic.attributes?.event;
    if (lifecycle === "start") {
      const index = visible.push(event) - 1;
      pendingStarts.set(key, [...(pendingStarts.get(key) ?? []), index]);
      continue;
    }
    if (lifecycle === "execution_start") {
      const startIndexes = pendingStarts.get(key);
      const startIndex = startIndexes?.[0];
      if (startIndex !== undefined) {
        visible[startIndex] = event;
      } else {
        const index = visible.push(event) - 1;
        pendingStarts.set(key, [index]);
      }
      continue;
    }
    if (lifecycle === "finish") {
      const status =
        typeof topicEvent.payload.status === "string"
          ? topicEvent.payload.status
          : "";
      const startIndexes = pendingStarts.get(key);
      const startIndex = startIndexes?.[0];
      if (startIndex !== undefined) {
        const started = visible[startIndex];
        const elapsedMs = event.created_at_ms - started.created_at_ms;
        visible[startIndex] =
          elapsedMs >= 0 ? withActionElapsed(event, elapsedMs) : event;
        if (status !== TOOL_STATUS_BACKGROUND_RUNNING) startIndexes?.shift();
      } else {
        // A trimmed history may no longer contain the action start. Only a
        // structured action id is unique enough to settle its background row;
        // legacy action+input keys can collide across repeated commands.
        const canSettleTrimmedBackground = key.startsWith("id:");
        const backgroundIndexes = canSettleTrimmedBackground
          ? pendingBackgroundFinishes.get(key)
          : undefined;
        const backgroundIndex = backgroundIndexes?.[0];
        if (
          status !== TOOL_STATUS_BACKGROUND_RUNNING &&
          backgroundIndex !== undefined
        ) {
          visible[backgroundIndex] = event;
          backgroundIndexes?.shift();
        } else {
          const index = visible.push(event) - 1;
          if (
            canSettleTrimmedBackground &&
            status === TOOL_STATUS_BACKGROUND_RUNNING
          ) {
            pendingBackgroundFinishes.set(key, [
              ...(backgroundIndexes ?? []),
              index,
            ]);
          }
        }
        if (backgroundIndexes?.length === 0)
          pendingBackgroundFinishes.delete(key);
      }
      if (startIndexes?.length === 0) pendingStarts.delete(key);
      continue;
    }
    visible.push(event);
  }
  return visible;
}

function withActionElapsed(
  event: WebTurnEvent,
  elapsedMs: number,
): WebTurnEvent {
  const topic = event.payload as unknown as CoreTopicEvent;
  return {
    ...event,
    payload: {
      ...event.payload,
      payload: {
        ...topic.payload,
        elapsed_ms: elapsedMs,
      },
    },
  };
}

export function boundSessionHistory(session: Session): Session {
  return {
    ...session,
    messages: trimMessages(session.messages),
    turns: trimTurns(session.turns).map((turn) => ({
      ...turn,
      events: turn.events,
    })),
  };
}

export function upsertSession(sessions: Session[], incoming: Session) {
  const bounded = boundSessionHistory(incoming);
  return sessions.some((session) => session.session_id === incoming.session_id)
    ? sessions.map((session) =>
        session.session_id === incoming.session_id ? bounded : session,
      )
    : [...sessions, bounded];
}

export function removePendingAttachment(
  session: Session,
  attachmentId: string,
): Session {
  return {
    ...session,
    attachments: session.attachments.filter(
      (attachment) => attachment.id !== attachmentId,
    ),
  };
}

export function upsertTurn(session: Session, incoming: WebTurn): Session {
  const previous = session.turns.find(
    (turn) => turn.turn_id === incoming.turn_id,
  );
  const previousIsTerminal = !!(
    previous &&
    (previous.state === "finished" || previous.completion)
  );
  // Turn state is monotonic. A delayed started/updated event from a cancelled
  // execution must not overwrite the terminal projection already shown by UI.
  const boundedIncoming =
    previousIsTerminal && incoming.state !== "finished" ? previous : incoming;
  const turns = trimTurns(
    previous
      ? session.turns.map((turn) =>
          turn.turn_id === incoming.turn_id ? boundedIncoming : turn,
        )
      : [...session.turns, boundedIncoming],
  );
  const started = boundedIncoming.state === "working" && !previousIsTerminal;
  const visuallyWorking = started && !session.cancelling_turn_id;
  const pending = incoming.state === "pending";
  return {
    ...session,
    state: visuallyWorking ? "working" : session.state,
    active_turn_id: started ? incoming.turn_id : session.active_turn_id,
    pending_turn_id: started
      ? null
      : pending
        ? incoming.turn_id
        : session.pending_turn_id,
    turns,
  };
}

export function applyChatMessageDeleted(
  session: Session,
  turnId: string,
  role: "user" | "assistant",
  roleIndex: number,
): Session {
  const previous = session.turns.find((turn) => turn.turn_id === turnId);
  if (!previous) return session;
  const deleted =
    role === "user"
      ? previous.user_entries[roleIndex] && {
          text: previous.user_entries[roleIndex].text,
          createdAtMs: previous.user_entries[roleIndex].created_at_ms,
        }
      : roleIndex === 0 && previous.final_answer
        ? { text: previous.final_answer, createdAtMs: previous.created_at_ms }
        : undefined;
  const updatedTurn =
    role === "user"
      ? {
          ...previous,
          user_entries: previous.user_entries.filter(
            (_entry, index) => index !== roleIndex,
          ),
        }
      : { ...previous, final_answer: null };
  const updated = {
    ...session,
    turns: session.turns.map((turn) =>
      turn.turn_id === turnId ? updatedTurn : turn,
    ),
  };
  if (!deleted) return updated;
  let candidate = -1;
  let candidateDistance = Number.POSITIVE_INFINITY;
  updated.messages.forEach((message, index) => {
    if (message.role !== role || message.text !== deleted.text) return;
    const distance = Math.abs(message.created_at_ms - deleted.createdAtMs);
    if (distance < candidateDistance) {
      candidate = index;
      candidateDistance = distance;
    }
  });
  if (candidate < 0) return updated;
  return {
    ...updated,
    messages: updated.messages.filter((_message, index) => index !== candidate),
  };
}

export function prependHistoryRecords(
  session: Session,
  records: ChatHistoryRecord[],
): Session {
  const historicalTurns = turnsFromHistoryRecords(records).map((turn) => ({
    ...turn,
    events: turn.events,
  }));
  const existingTurnIds = new Set(session.turns.map((turn) => turn.turn_id));
  const earlier = historicalTurns.filter(
    (turn) => !existingTurnIds.has(turn.turn_id),
  );
  const earlierTurnIds = new Set(earlier.map((turn) => turn.turn_id));
  const existingMessageIds = new Set(
    session.messages.map((message) => message.id),
  );
  const earlierMessages = messagesFromHistoryRecords(
    records.filter(
      (record) =>
        earlierTurnIds.has(record.turn_id) ||
        (record.type === "message" &&
          record.role === "system" &&
          record.kind === "runtime_restart"),
    ),
  ).filter((message) => !existingMessageIds.has(message.id));
  if (earlier.length === 0 && earlierMessages.length === 0) return session;
  return {
    ...session,
    turns: trimTurns([...earlier, ...session.turns]),
    messages: trimMessages([...earlierMessages, ...session.messages]),
  };
}

export function turnsFromHistoryRecords(
  records: ChatHistoryRecord[],
): WebTurn[] {
  const turns = new Map<string, WebTurn>();
  for (const record of records) {
    if (record.type === "message" && record.role === "system") continue;
    const turn = turns.get(record.turn_id) ?? {
      turn_id: record.turn_id,
      state: "restored",
      created_at_ms: record.created_at_ms,
      user_entries: [],
      events: [],
      sub_answers: [],
      final_answer: null,
      completion: null,
    };
    turn.created_at_ms = Math.min(turn.created_at_ms, record.created_at_ms);
    if (record.type === "message") {
      if (record.role === "user") {
        const kind =
          record.kind &&
          ["task", "supplement", "approval"].includes(record.kind)
            ? record.kind
            : "task";
        turn.user_entries.push({
          kind,
          text: record.content,
          attachments: [],
          created_at_ms: record.created_at_ms,
        });
      } else if (record.role === "assistant") {
        turn.final_answer = record.content;
      }
    } else if (record.type === "event") {
      const payload =
        typeof record.payload === "object" && record.payload !== null
          ? (record.payload as Record<string, unknown>)
          : { kind: record.kind, content: record.content };
      const source =
        typeof record.source === "string" ? record.source : "history";
      const subAnswer = subAnswerFromTurnEventPayload(
        payload,
        record.created_at_ms,
      );
      if (
        source === "core_topic" &&
        subAnswer &&
        !turn.sub_answers.some(
          (item) => item.sub_answer_id === subAnswer.sub_answer_id,
        )
      ) {
        turn.sub_answers.push(subAnswer);
        turn.sub_answers.sort((left, right) => left.ordinal - right.ordinal);
      }
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
      user_entries: [...turn.user_entries].sort(
        (left, right) => left.created_at_ms - right.created_at_ms,
      ),
      events: [...turn.events].sort(
        (left, right) => left.created_at_ms - right.created_at_ms,
      ),
    }))
    .sort((left, right) => left.created_at_ms - right.created_at_ms);
}

type ChatMessageHistoryRecord = Extract<ChatHistoryRecord, { type: "message" }>;

function isChatMessageHistoryRecord(
  record: ChatHistoryRecord,
): record is ChatMessageHistoryRecord {
  return (
    record.type === "message" &&
    (record.role === "user" ||
      record.role === "assistant" ||
      (record.role === "system" && record.kind === "runtime_restart"))
  );
}

function messagesFromHistoryRecords(
  records: ChatHistoryRecord[],
): ChatMessage[] {
  return records
    .filter(isChatMessageHistoryRecord)
    .sort((left, right) => left.created_at_ms - right.created_at_ms)
    .map((record) => ({
      id: `history_msg_${record.turn_id}_${record.created_at_ms}_${record.role}`,
      role: record.role,
      text: record.content,
      created_at_ms: record.created_at_ms,
      kind: record.kind,
    }));
}

export function appendActivityToCurrentTurn(
  session: Session,
  activity: Activity,
): Session {
  const turnId = session.active_turn_id ?? session.turns.at(-1)?.turn_id;
  if (!turnId) return session;
  return appendTurnEvent(session, turnId, {
    event_id: activity.id,
    source: "ui_activity",
    payload: { ...activity, sessionId: session.session_id },
    created_at_ms: activity.createdAt,
  });
}

export function appendTurnEvent(
  session: Session,
  turnId: string | null | undefined,
  event: WebTurnEvent,
): Session {
  if (!turnId) return session;
  if (!turnEventBelongsToSession(session, event)) return session;
  const turnIndex = session.turns.findIndex((turn) => turn.turn_id === turnId);
  if (turnIndex < 0) return session;
  const target = session.turns[turnIndex];
  if (target.events.some((existing) => existing.event_id === event.event_id))
    return session;
  const turns = [...session.turns];
  const subAnswer = subAnswerFromTurnEventPayload(
    event.payload,
    event.created_at_ms,
  );
  const subAnswers =
    subAnswer &&
    !target.sub_answers.some(
      (item) => item.sub_answer_id === subAnswer.sub_answer_id,
    )
      ? [...target.sub_answers, subAnswer].sort(
          (left, right) => left.ordinal - right.ordinal,
        )
      : target.sub_answers;
  turns[turnIndex] = {
    ...target,
    sub_answers: subAnswers,
    final_answer:
      finalAnswerFromTurnEvent(session, event) ?? target.final_answer,
    events: [...target.events, event],
  };
  return {
    ...session,
    turns,
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

function turnEventBelongsToSession(
  session: Session,
  event: WebTurnEvent,
): boolean {
  if (event.source !== "core_topic") return true;
  const topicEvent = event.payload as unknown as CoreTopicEvent;
  if (topicEvent.session_id !== session.session_id) return false;
  const isLifecycle = topicEvent.topic?.name === "core.lifecycle";
  if (
    !isLifecycle &&
    topicEvent.worker_id &&
    !session.workers.some((worker) => worker.worker_id === topicEvent.worker_id)
  )
    return false;
  if (
    !isLifecycle &&
    topicEvent.context_id &&
    !session.contexts.some(
      (context) => context.context_id === topicEvent.context_id,
    )
  )
    return false;
  return true;
}

function subAnswerFromTurnEventPayload(
  payload: Record<string, unknown>,
  createdAtMs: number,
) {
  const topic = payload.topic;
  const body = payload.payload;
  if (
    !topic ||
    typeof topic !== "object" ||
    (topic as Record<string, unknown>).name !== "core.sub_answer"
  )
    return undefined;
  if (!body || typeof body !== "object") return undefined;
  const item = body as Record<string, unknown>;
  if (
    typeof item.sub_answer_id !== "string" ||
    typeof item.ordinal !== "number" ||
    typeof item.task !== "string" ||
    typeof item.answer !== "string"
  )
    return undefined;
  if (!item.sub_answer_id.trim() || !item.task.trim() || !item.answer.trim())
    return undefined;
  return {
    sub_answer_id: item.sub_answer_id,
    ordinal: item.ordinal,
    task: item.task,
    answer: item.answer,
    created_at_ms: createdAtMs,
  };
}

function finalAnswerFromTurnEvent(session: Session, event: WebTurnEvent) {
  if (event.source !== "core_topic") return undefined;
  const topic = event.payload.topic;
  const payload = event.payload.payload;
  if (
    !topic ||
    typeof topic !== "object" ||
    (topic as Record<string, unknown>).name !== "core.model.response"
  )
    return undefined;
  if (!payload || typeof payload !== "object") return undefined;
  if ((payload as Record<string, unknown>).runtime_phase === "toolgen")
    return undefined;
  const workerId = event.payload.worker_id;
  if (typeof workerId === "string" && workerId !== session.primary_worker_id)
    return undefined;
  const finalAnswer = (payload as Record<string, unknown>).final_answer;
  return typeof finalAnswer === "string" && finalAnswer.trim()
    ? finalAnswer
    : undefined;
}

export function sessionVisuallyWorking(session: Session): boolean {
  return session.state === "working" && !sessionCancellationApplies(session);
}


export function finishTurn(
  session: Session,
  turnId: string | null | undefined,
  completion: TurnCompletion,
): Session {
  const workers = session.workers.map((worker) =>
    worker.state === "working" ? { ...worker, state: "ready" } : worker,
  );
  const state = aggregateSessionState(workers, "ready");
  if (!turnId) return { ...session, workers, state };
  return {
    ...session,
    workers,
    state,
    active_turn_id:
      session.active_turn_id === turnId ? null : session.active_turn_id,
    pending_turn_id:
      session.pending_turn_id === turnId ? null : session.pending_turn_id,
    cancelling_turn_id:
      session.cancelling_turn_id === turnId ? null : session.cancelling_turn_id,
    turns: session.turns.map((turn) =>
      turn.turn_id === turnId
        ? { ...turn, state: "finished", completion }
        : turn,
    ),
  };
}

export function updateSessionWorkerState(
  session: Session,
  workerId: string,
  state: string,
  turnId?: string | null,
): Session {
  if (state === "working" && session.cancelling_turn_id) return session;
  if (
    state === "working" &&
    turnId &&
    session.turns.some(
      (turn) =>
        turn.turn_id === turnId &&
        (turn.state === "finished" || !!turn.completion),
    )
  )
    return session;
  let found = false;
  let changed = false;
  const workers = session.workers.map((worker) => {
    if (worker.worker_id !== workerId) return worker;
    found = true;
    if (worker.state === state) return worker;
    changed = true;
    return { ...worker, state };
  });
  return found && changed
    ? {
        ...session,
        workers,
        state: aggregateSessionState(workers, session.state),
      }
    : session;
}

function aggregateSessionState(workers: Session["workers"], fallback: string) {
  if (workers.length === 0) return fallback;
  if (workers.some((worker) => worker.state === "working")) return "working";
  if (workers.some((worker) => worker.state === "error")) return "error";
  if (workers.every((worker) => worker.state === "stopped")) return "stopped";
  return fallback === "interrupted" ? "interrupted" : "ready";
}

function turnLiveUsageSince(
  turn: WebTurn,
  createdAtOrAfterMs?: number,
):
  | {
      total: import("./protocol").UsageStats;
      latest: import("./protocol").UsageStats;
    }
  | undefined {
  let latest: import("./protocol").UsageStats | undefined;
  const total: import("./protocol").UsageStats = {};
  for (const event of turn.events) {
    if (
      createdAtOrAfterMs !== undefined &&
      event.created_at_ms < createdAtOrAfterMs
    )
      continue;
    if (
      event.source !== "worker_activity" ||
      event.payload.kind !== "model_response"
    )
      continue;
    const usage = event.payload.usage;
    if (!usage || typeof usage !== "object") continue;
    const current = usage as import("./protocol").UsageStats;
    latest = current;
    for (const field of USAGE_FIELDS) {
      const value = current[field];
      if (typeof value === "number" && Number.isFinite(value))
        total[field] = (total[field] ?? 0) + value;
    }
  }
  return latest ? { total, latest } : undefined;
}

export function turnLiveUsage(turn: WebTurn):
  | {
      total: import("./protocol").UsageStats;
      latest: import("./protocol").UsageStats;
    }
  | undefined {
  return turnLiveUsageSince(turn);
}

function sessionRuntimeRestartAtMs(session: Session): number | undefined {
  return session.messages.reduce<number | undefined>(
    (latest, message) =>
      message.role === "system" &&
      message.kind === "runtime_restart" &&
      (latest === undefined || message.created_at_ms > latest)
        ? message.created_at_ms
        : latest,
    undefined,
  );
}

export function sessionContextUsage(
  session: Session,
): import("./protocol").UsageStats | undefined {
  const runtimeRestartAtMs = sessionRuntimeRestartAtMs(session);

  for (let index = session.turns.length - 1; index >= 0; index -= 1) {
    const turn = session.turns[index];
    if (turn.state === "restored") continue;

    // A restarted host restores historical turns for display, but Core starts
    // with a fresh context. Only model responses emitted by the new runtime
    // instance may refill the context meter.
    const live = turnLiveUsageSince(turn, runtimeRestartAtMs);
    if (live) return live.latest;

    // Completion telemetry has no independent timestamp. It is safe only when
    // the whole turn began after the latest runtime restart boundary.
    if (
      runtimeRestartAtMs !== undefined &&
      turn.created_at_ms < runtimeRestartAtMs
    )
      continue;
    const latest = turn.completion?.latest_usage;
    if (latest) return latest;
  }
  return undefined;
}

export function sessionRuntimeUsage(
  session: Session,
): import("./protocol").UsageStats | undefined {
  const runtimeRestartAtMs = sessionRuntimeRestartAtMs(session);
  const total: import("./protocol").UsageStats = {};
  let found = false;

  const add = (usage: import("./protocol").UsageStats | undefined) => {
    if (!usage) return;
    found = true;
    for (const field of USAGE_FIELDS) {
      const value = usage[field];
      if (typeof value === "number" && Number.isFinite(value))
        total[field] = (total[field] ?? 0) + value;
    }
  };

  for (const turn of session.turns) {
    if (turn.state === "restored") continue;
    const live = turnLiveUsageSince(turn, runtimeRestartAtMs);
    if (live) {
      add(live.total);
      continue;
    }
    // Completion stats have no independent timestamp, so they count only when
    // the complete turn started in this runtime instance.
    if (
      runtimeRestartAtMs !== undefined &&
      turn.created_at_ms < runtimeRestartAtMs
    )
      continue;
    add(turn.completion?.stats);
  }
  return found ? total : undefined;
}

export function sessionCacheHitPercent(session: Session): number | undefined {
  const usage = sessionRuntimeUsage(session);
  const promptTokens = usage?.prompt_tokens ?? 0;
  if (promptTokens <= 0) return undefined;
  return Math.min(
    100,
    Math.max(0, ((usage?.cached_tokens ?? 0) * 100) / promptTokens),
  );
}

export function decisionKey(decision: Decision) {
  const requestId =
    typeof decision.event.payload.request_id === "string"
      ? decision.event.payload.request_id
      : "";
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

export function decisionsFromSessions(sessions: readonly Session[]) {
  let decisions: Decision[] = [];
  for (const session of sessions) {
    for (const turn of session.turns) {
      if (turn.state !== "working") continue;
      for (const event of turn.events) {
        if (event.source !== "core_topic") continue;
        const decision = requestDecision(
          event.payload as unknown as CoreTopicEvent,
          turn.turn_id,
        );
        if (decision) decisions = enqueueDecision(decisions, decision);
      }
    }
  }
  return decisions;
}

export function sessionTurnKey(sessionId: string, turnId: string) {
  return `${sessionId}\u0000${turnId}`;
}

export function groupDecisionsBySessionTurn(decisions: Decision[]) {
  const grouped = new Map<string, Decision[]>();
  for (const decision of decisions) {
    if (!decision.turnId) continue;
    const key = sessionTurnKey(decision.event.session_id, decision.turnId);
    const current = grouped.get(key);
    if (current) current.push(decision);
    else grouped.set(key, [decision]);
  }
  return grouped;
}

export function clearDecisionsForSession(
  decisions: Decision[],
  sessionId: string,
) {
  return decisions.filter(
    (decision) => decision.event.session_id !== sessionId,
  );
}

export function clearDecisionsForWorker(
  decisions: Decision[],
  sessionId: string,
  workerId: string,
) {
  return decisions.filter(
    (decision) =>
      !(
        decision.event.session_id === sessionId &&
        decision.event.worker_id === workerId
      ),
  );
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
  turnId?: string | null,
): Session {
  if (session.session_id !== event.session_id) return session;
  const isLifecycle = event.topic.name === "core.lifecycle";
  if (
    !isLifecycle &&
    event.worker_id &&
    !session.workers.some((worker) => worker.worker_id === event.worker_id)
  )
    return session;
  if (
    !isLifecycle &&
    event.context_id &&
    !session.contexts.some((context) => context.context_id === event.context_id)
  )
    return session;
  const contextState = event.payload.context_state;
  const reportedDir =
    contextState &&
    typeof contextState === "object" &&
    typeof (contextState as Record<string, unknown>).cwd === "string"
      ? (contextState as Record<string, string>).cwd
      : undefined;
  const targetContextId = event.context_id ?? session.active_context_id;
  let contexts = reportedDir
    ? session.contexts.map((context) =>
        context.context_id === targetContextId
          ? { ...context, current_dir: reportedDir }
          : context,
      )
    : session.contexts;
  const currentDir =
    reportedDir && targetContextId === session.active_context_id
      ? reportedDir
      : session.current_dir;
  let workers = session.workers;
  if (event.topic.name === "core.model.response") {
    if (event.payload.runtime_phase === "toolgen") return session;
    const finalAnswer =
      typeof event.payload.final_answer === "string"
        ? event.payload.final_answer.trim()
        : "";
    const messageId =
      typeof event.payload.ui_message_id === "string"
        ? event.payload.ui_message_id
        : undefined;
    const isPrimary =
      !event.worker_id || event.worker_id === session.primary_worker_id;
    const nextMessages =
      finalAnswer && isPrimary
        ? trimMessages([
            ...session.messages,
            makeAssistantMessage(finalAnswer, messageId),
          ])
        : session.messages;
    const updated = event.worker_id
      ? updateSessionWorkerState(
          session,
          event.worker_id,
          event.payload.continue_work === true ? "working" : "ready",
          turnId,
        )
      : {
          ...session,
          state:
            event.payload.continue_work === true &&
            !(
              turnId &&
              session.turns.some(
                (turn) =>
                  turn.turn_id === turnId &&
                  (turn.state === "finished" || !!turn.completion),
              )
            )
              ? "working"
              : "ready",
        };
    return {
      ...updated,
      contexts,
      current_dir: currentDir,
      messages: nextMessages,
    };
  }
  if (event.topic.name === "core.lifecycle") {
    const worker = event.payload.worker;
    if (event.worker_id && event.context_id) {
      const contextExists = contexts.some(
        (context) => context.context_id === event.context_id,
      );
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
        contexts = contexts.map((context) =>
          context.context_id === event.context_id &&
          !context.worker_ids.includes(event.worker_id!)
            ? {
                ...context,
                worker_ids: [...context.worker_ids, event.worker_id!],
              }
            : context,
        );
      }
      if (!workers.some((item) => item.worker_id === event.worker_id)) {
        const workerPayload =
          worker && typeof worker === "object"
            ? (worker as Record<string, unknown>)
            : {};
        workers = [
          ...workers,
          {
            worker_id: event.worker_id,
            context_id: event.context_id,
            display_name:
              typeof workerPayload.display_name === "string"
                ? workerPayload.display_name
                : event.worker_id,
            ordinal:
              typeof workerPayload.ordinal === "number"
                ? workerPayload.ordinal
                : workers.length,
            state: "ready",
            parent_worker_id:
              typeof workerPayload.parent_worker_id === "string"
                ? workerPayload.parent_worker_id
                : null,
          },
        ];
      }
    }
    const displayName =
      worker &&
      typeof worker === "object" &&
      typeof (worker as Record<string, unknown>).display_name === "string"
        ? (worker as Record<string, string>).display_name
        : session.display_name;
    const maxLlmInputTokens =
      typeof event.payload.max_llm_input_tokens === "number"
        ? event.payload.max_llm_input_tokens
        : session.max_llm_input_tokens;
    workers = event.worker_id
      ? workers.map((item) =>
          item.worker_id === event.worker_id
            ? { ...item, display_name: displayName, state: "ready" }
            : item,
        )
      : workers;
    return {
      ...session,
      workers,
      contexts,
      current_dir: currentDir,
      max_llm_input_tokens: maxLlmInputTokens,
      state: aggregateSessionState(workers, session.state),
    };
  }
  return currentDir === session.current_dir && contexts === session.contexts
    ? session
    : { ...session, contexts, current_dir: currentDir };
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

function protocolRepairDisplayReason(payload: Record<string, unknown>): string {
  const issue = typeof payload.issue === "string" ? payload.issue : "";
  const knownReasons: Record<string, string> = {
    xml_recovered_final_answer_requires_retry:
      "回复根节点外包含了额外内容。系统虽然识别出了最终回答，但无法将它安全地视为完整响应，因此正在重新请求。",
    invalid_xml: "模型回复不是有效的 XML 协议消息，因此正在重新请求。",
    invalid_xml_response_root:
      "回复没有使用唯一且完整的 response 根节点，因此正在重新请求。",
    xml_response_root_missing:
      "回复缺少必需的 response 根节点，因此正在重新请求。",
    missing_response_root: "回复缺少必需的 response 根节点，因此正在重新请求。",
    xml_response_root_unclosed:
      "回复的 response 根节点没有完整闭合，因此正在重新请求。",
    xml_content_before_response:
      "response 根节点前存在额外内容，因此正在重新请求。",
    xml_content_after_response:
      "response 根节点后存在额外内容，因此正在重新请求。",
    empty_response: "模型没有返回可解析的内容，因此正在重新请求。",
    truncated_model_output:
      "模型输出在完整响应生成前被截断，因此正在重新请求。",
    finish_confirm_required_before_final_answer:
      "最终回答前缺少协议要求的完成确认，因此正在重新请求。",
    finish_confirm_prefix_invalid:
      "最终回答前的完成确认格式不正确，因此正在重新请求。",
  };
  const knownReason = knownReasons[issue];
  if (knownReason) return knownReason;

  const reason =
    typeof payload.reason === "string" ? payload.reason.trim() : "";
  return reason || "模型回复格式不符合当前协议要求，系统正在自动重新请求。";
}

export type ActiveModelRetryStatus = {
  kind:
    | "network-error"
    | "response-timeout"
    | "rate-limited"
    | "service-error"
    | "retrying";
  label: string;
  progress?: string;
  detail: string;
};

type ModelSystemRetryDisplay = Pick<
  ActiveModelRetryStatus,
  "kind" | "label"
> & {
  summary: string;
};

function modelSystemRetryDisplay(error: string): ModelSystemRetryDisplay {
  const normalized = error.toLowerCase();
  if (
    normalized.startsWith("model_timeout") ||
    normalized.includes("operation timed out") ||
    normalized.includes("curl: (28)")
  ) {
    return {
      kind: "response-timeout",
      label: "响应超时",
      summary: "模型服务在超时期限内没有返回新的响应数据，系统正在自动重试。",
    };
  }
  if (normalized.startsWith("model_http_429")) {
    return {
      kind: "rate-limited",
      label: "服务限流",
      summary:
        "模型接入点返回限流错误，可能与请求频率、并发限制或额度有关；系统正在自动重试。",
    };
  }
  if (/^model_http_(408|409|425|5\d\d)/.test(normalized)) {
    return {
      kind: "service-error",
      label: "上游异常",
      summary: "模型接入点或其上游服务返回可重试错误，系统正在自动重试。",
    };
  }
  if (
    normalized.startsWith("model_network_error") ||
    normalized.startsWith("curl_failed") ||
    normalized.includes("curl:") ||
    normalized.includes("http2 framing") ||
    normalized.includes("connection reset") ||
    normalized.includes("could not resolve host")
  ) {
    return {
      kind: "network-error",
      label: "网络异常",
      summary: "连接模型服务时发生网络异常，系统正在自动重连。",
    };
  }
  return {
    kind: "service-error",
    label: "模型服务异常",
    summary: "模型请求发生可重试错误，系统正在自动重试。",
  };
}

function retryProgress(payload: Record<string, unknown>): string | undefined {
  const attempt =
    typeof payload.attempt === "number" ? payload.attempt : undefined;
  const maxAttempts =
    typeof payload.max_attempts === "number" ? payload.max_attempts : undefined;
  if (attempt === undefined) return undefined;
  return maxAttempts === undefined
    ? `第 ${attempt} 次`
    : `${attempt}/${maxAttempts}`;
}

export function activeModelRetryStatus(
  turn: WebTurn,
): ActiveModelRetryStatus | null {
  if (turn.state !== "working") return null;

  let status: ActiveModelRetryStatus | null = null;
  for (const event of turn.events) {
    if (event.source === "worker_activity") {
      const kind = event.payload.kind;
      if (kind === "model_request" || kind === "model_response") {
        status = null;
        continue;
      }
      if (kind !== "model_retry") continue;

      const progress = retryProgress(event.payload);
      const error =
        typeof event.payload.error === "string"
          ? event.payload.error.trim()
          : "";
      const display = modelSystemRetryDisplay(error);
      const delayMs =
        typeof event.payload.delay_ms === "number"
          ? event.payload.delay_ms
          : undefined;
      const detail = [
        display.summary,
        progress ? `重试进度：${progress}` : "",
        delayMs !== undefined
          ? `下次尝试：约 ${Math.ceil(delayMs / 1000)} 秒后`
          : "",
        error ? `错误详情：${error}` : "",
      ]
        .filter(Boolean)
        .join("\n\n");
      status = { kind: display.kind, label: display.label, progress, detail };
      continue;
    }

    if (event.source !== "core_topic") continue;
    const topic = event.payload as unknown as CoreTopicEvent;
    if (topic.topic?.name !== "core.model.repair") continue;

    const progress = retryProgress(topic.payload);
    status = {
      kind: "retrying",
      label: "retrying",
      progress,
      detail: [
        "模型回复偏离当前响应协议，系统正在自动重新请求。",
        progress ? `修复进度：${progress}` : "",
        protocolRepairDisplayReason(topic.payload),
      ]
        .filter(Boolean)
        .join("\n\n"),
    };
  }
  return status;
}

export function activityFromTopic(event: CoreTopicEvent): Activity | null {
  const payload = event.payload;
  const label = (value: unknown) => (typeof value === "string" ? value : "");
  switch (event.topic.name) {
    case "core.model.response": {
      const finalAnswer = label(payload.final_answer).trim();
      if (finalAnswer && payload.runtime_phase !== "toolgen") return null;
      const freeTalk = label(payload.free_talk);
      const progress = label(payload.progress);
      const detail = [freeTalk, progress]
        .filter((text) => text.trim())
        .join("\n\n");
      return detail
        ? {
            id: clientId(),
            sessionId: event.session_id,
            tone: "thinking",
            kind: "free_talk",
            title: "",
            detail,
            createdAt: Date.now(),
          }
        : null;
    }
    case "core.model.repair":
      return null;
    case "core.action": {
      const action = label(payload.action) || "action";
      const status = label(payload.status) || label(payload.event) || "running";
      const statusText = humanizeToolStatus(status);
      const input =
        payload.input && typeof payload.input === "object"
          ? (payload.input as Record<string, unknown>)
          : undefined;
      const kind =
        payload.kind && typeof payload.kind === "object"
          ? (payload.kind as Record<string, unknown>)
          : undefined;
      const toolMode =
        typeof kind?.mode === "string"
          ? kind.mode
          : typeof input?.loop_cmd === "string"
            ? "poll"
            : undefined;
      const command =
        action === "run_bash"
          ? [input?.cmd, input?.loop_cmd, kind?.command].find(
              (value): value is string =>
                typeof value === "string" && value.trim().length > 0,
            )
          : undefined;
      const numericKindValue = (name: string) =>
        typeof kind?.[name] === "number" && Number.isFinite(kind[name])
          ? (kind[name] as number)
          : undefined;
      const detail = command ? "" : formatToolArguments(input);
      return {
        id: clientId(),
        sessionId: event.session_id,
        tone: "action",
        title: `${toolActivityDisplayName(action, toolMode)} · ${statusText}`,
        tool_name: action,
        tool_status: status,
        tool_mode: toolMode,
        elapsed_ms:
          typeof payload.elapsed_ms === "number"
            ? payload.elapsed_ms
            : undefined,
        timeout_ms: numericKindValue("timeout_ms"),
        loop_timeout_ms: numericKindValue("loop_timeout_ms"),
        interval_ms: numericKindValue("interval_ms"),
        pid:
          typeof payload.pid === "number" && Number.isFinite(payload.pid)
            ? payload.pid
            : undefined,
        execution_started:
          payload.event === "execution_start" || payload.event === "finish",
        detail,
        code: command ? redactSensitiveDisplayText(command) : undefined,
        code_language: command ? "bash" : undefined,
        createdAt: Date.now(),
      };
    }
    case "core.context.compact": {
      const before =
        typeof payload.estimated_before_tokens === "number"
          ? payload.estimated_before_tokens
          : undefined;
      const after =
        typeof payload.estimated_after_tokens === "number"
          ? payload.estimated_after_tokens
          : undefined;
      const textBefore =
        typeof payload.estimated_text_before_tokens === "number"
          ? payload.estimated_text_before_tokens
          : undefined;
      const textAfter =
        typeof payload.estimated_text_after_tokens === "number"
          ? payload.estimated_text_after_tokens
          : undefined;
      const nativeBefore =
        typeof payload.estimated_native_before_tokens === "number"
          ? payload.estimated_native_before_tokens
          : undefined;
      const nativeAfter =
        typeof payload.estimated_native_after_tokens === "number"
          ? payload.estimated_native_after_tokens
          : undefined;
      return {
        id: clientId(),
        sessionId: event.session_id,
        tone: "notice",
        kind: "context_compact",
        title: "Dynamic context compacted",
        detail: `Dynamic context ${before ?? "?"} tokens → ${after ?? "?"} tokens`,
        before_tokens: before,
        after_tokens: after,
        text_before_tokens: textBefore,
        text_after_tokens: textAfter,
        native_before_tokens: nativeBefore,
        native_after_tokens: nativeAfter,
        createdAt: Date.now(),
      };
    }
    case "core.toolgen": {
      const phase = label(payload.phase);
      const tool =
        payload.tool && typeof payload.tool === "object"
          ? (payload.tool as Record<string, unknown>)
          : undefined;
      const toolName = tool ? label(tool.name) : "";
      const retrospect = label(payload.retrospect);
      const error = label(payload.error);
      const title =
        phase === "published"
          ? `ToolGen: 已生成并验证 ${toolName || "可复用工具"}`
          : phase === "started"
            ? "ToolGen: 正在评估…"
            : "ToolGen: 生成失败";
      return {
        id: clientId(),
        sessionId: event.session_id,
        tone:
          phase === "published" || phase === "started" ? "notice" : "warning",
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

export function hasOnlyFreeTalkActivity(
  activities: Activity[],
  decisionCount: number,
) {
  return (
    activities.length > 0 &&
    activities.every((activity) => activity.tone === "thinking") &&
    decisionCount === 0
  );
}

export function toolActivityDisplayName(name: string, mode?: string) {
  if (name === "run_bash" && mode === "poll") return "Poll";
  return toolDisplayName(name);
}

export function toolDisplayName(name: string) {
  if (name === "run_bash") return "Bash";
  if (name === "memmgr") return "MemMgr";
  if (name === "capmgr") return "CapMgr";
  if (name === "self_tool") return "Self Tool";
  return name;
}

function formatToolArguments(input: Record<string, unknown> | undefined) {
  if (!input) return "";
  return Object.entries(input)
    .map(
      ([key, value]) =>
        `${key}=${formatToolValue(redactSensitiveToolValue(key, value))}`,
    )
    .join(" ");
}

function isSensitiveToolKey(key: string) {
  return /^(?:authorization|x-[\w-]*(?:token|key)|(?:[\w-]+[-_])?(?:api[-_]?key|access[-_]?token|refresh[-_]?token|auth[-_]?token|token|secret|password|credential|gwtoken))$/i.test(
    key,
  );
}

function redactSensitiveToolValue(key: string, value: unknown): unknown {
  if (isSensitiveToolKey(key)) return "****";
  if (Array.isArray(value))
    return value.map((item) => redactSensitiveToolValue("", item));
  if (value && typeof value === "object") {
    return Object.fromEntries(
      Object.entries(value as Record<string, unknown>).map(
        ([nestedKey, nestedValue]) => [
          nestedKey,
          redactSensitiveToolValue(nestedKey, nestedValue),
        ],
      ),
    );
  }
  return typeof value === "string" ? redactSensitiveDisplayText(value) : value;
}

export function redactSensitiveDisplayText(text: string) {
  const sensitiveName =
    "authorization|x-[\\w-]*(?:token|key)|(?:[\\w-]+[-_])?(?:api[-_]?key|access[-_]?token|refresh[-_]?token|auth[-_]?token|token|secret|password|credential|gwtoken)";
  const quotedHeader = new RegExp(
    `(["'])(${sensitiveName})(\\s*[:=]\\s*)(?:bearer\\s+)?[^"']+\\1`,
    "gi",
  );
  const quotedAssignment = new RegExp(
    `\\b(${sensitiveName})\\b(\\s*[:=]\\s*)(["'])(?:bearer\\s+)?[^"']+\\3`,
    "gi",
  );
  const sensitiveAssignment = new RegExp(
    `\\b(${sensitiveName})\\b(\\s*[:=]\\s*)(?:bearer\\s+)?([^\\s"'\`;|&]+)`,
    "gi",
  );
  const sensitiveFlag =
    /(--(?:api[_-]?key|access[_-]?token|refresh[_-]?token|auth[_-]?token|token|secret|password|credential)\s+)(["']?)([^\s"'`;|&]+)(["']?)/gi;
  return text
    .replace(
      quotedHeader,
      (_match, quote: string, key: string, separator: string) =>
        `${quote}${key}${separator}****${quote}`,
    )
    .replace(
      quotedAssignment,
      (_match, key: string, separator: string, quote: string) =>
        `${key}${separator}${quote}****${quote}`,
    )
    .replace(
      sensitiveAssignment,
      (_match, key: string, separator: string) => `${key}${separator}****`,
    )
    .replace(
      sensitiveFlag,
      (_match, flag: string, quote: string) => `${flag}${quote}****${quote}`,
    );
}

function formatToolValue(value: unknown): string {
  if (typeof value === "string") return JSON.stringify(value);
  if (value === null || typeof value === "boolean" || typeof value === "number")
    return String(value);
  return JSON.stringify(value);
}

export function requestDecision(
  event: CoreTopicEvent,
  turnId?: string | null,
): Decision | null {
  if (
    event.state.name !== "waiting_user" &&
    event.state.name !== "waiting_user_with_timeout"
  )
    return null;
  const payload = event.payload;
  const request =
    payload.request && typeof payload.request === "object"
      ? (payload.request as Record<string, unknown>)
      : {};
  const isLongRunningCommand =
    event.topic.name === "core.user.long_running_command.request";
  const workInstructionFiles = Array.isArray(request.file_names)
    ? request.file_names.filter(
        (name): name is string => typeof name === "string",
      )
    : [];
  const detail =
    isLongRunningCommand && typeof request.command === "string"
      ? formatLongRunningCommandDecision(request)
      : typeof request.command === "string"
        ? request.command
        : workInstructionFiles.length > 0
          ? `Load ${workInstructionFiles.join(", ")} from ${typeof request.directory === "string" ? request.directory : "this workspace"}?`
          : typeof request.message === "string"
            ? request.message
            : typeof payload.kind === "string"
              ? payload.kind
              : "Timem needs your decision before it can continue.";
  return {
    event,
    turnId: turnId ?? undefined,
    title: isLongRunningCommand ? "Long-running command" : "Decision required",
    detail,
  };
}

function formatLongRunningCommandDecision(
  request: Record<string, unknown>,
): string {
  const elapsed =
    typeof request.elapsed_ms === "number" &&
    Number.isFinite(request.elapsed_ms)
      ? formatDecisionDuration(request.elapsed_ms)
      : "an extended period";
  const timeout =
    typeof request.timeout_ms === "number" &&
    Number.isFinite(request.timeout_ms)
      ? `; timeout is ${formatDecisionDuration(request.timeout_ms)}`
      : "";
  return `The command has been running for ${elapsed}${timeout}.\nCommand: ${request.command}`;
}

function formatDecisionDuration(milliseconds: number): string {
  const seconds = Math.max(0, Math.floor(milliseconds / 1000));
  if (seconds < 60) return `${seconds}s`;
  return `${Math.floor(seconds / 60)}m${String(seconds % 60).padStart(2, "0")}s`;
}
