export const COLLAPSED_QUEUE_LIMIT = 4;

export type QueuedMessage = {
 id: string;
 text: string;
 createdAtMs: number;
 attachmentIds: string[];
 roleIds?: string[];
 /** Legacy queue compatibility. */
 roleId?: string;
 deliveryError?: string;
};

const STORAGE_PREFIX = "timem-web-queued-messages:v2";
const MAX_STORED_QUEUE_ITEMS = 2_000;
const MAX_STORED_MESSAGE_BYTES = 1024 * 1024;

export function queuedMessagesStorageKey(scope: string, messageId?: string) {
  const base = `${STORAGE_PREFIX}:${encodeURIComponent(scope)}`;
  return messageId === undefined ? base : `${base}:${encodeURIComponent(messageId)}`;
}

export type QueuedMessagesPauseState = {
 paused: true;
 reason?: string;
 stoppedAtMs: number;
};

export function queuedMessagesPauseStorageKey(scope: string) {
 return `${queuedMessagesStorageKey(scope)}-pause`;
}

export function loadQueuedMessagesPause(
 storage: Pick<Storage, "getItem">,
 scope: string,
): QueuedMessagesPauseState | null {
 try {
 const raw = storage.getItem(queuedMessagesPauseStorageKey(scope));
 if (!raw || raw.length > MAX_STORED_MESSAGE_BYTES) return null;
 const value = JSON.parse(raw) as Partial<QueuedMessagesPauseState>;
 if (
 value?.paused !== true
 || typeof value.stoppedAtMs !== "number"
 || !Number.isFinite(value.stoppedAtMs)
 || value.stoppedAtMs < 0
 || (value.reason !== undefined && typeof value.reason !== "string")
 ) return null;
 return {
 paused: true,
 stoppedAtMs: value.stoppedAtMs,
 ...(value.reason === undefined ? {} : { reason: value.reason }),
 };
 } catch {
 return null;
 }
}

export function saveQueuedMessagesPause(
 storage: Pick<Storage, "setItem">,
 scope: string,
 pause: QueuedMessagesPauseState,
) {
 try {
 storage.setItem(queuedMessagesPauseStorageKey(scope), JSON.stringify(pause));
 return true;
 } catch {
 return false;
 }
}

export function clearQueuedMessagesPause(
 storage: Pick<Storage, "removeItem">,
 scope: string,
) {
 try {
 storage.removeItem(queuedMessagesPauseStorageKey(scope));
 return true;
 } catch {
 return false;
 }
}


type StoredQueuedMessage = { sessionId: string; position: number; message: QueuedMessage };

function parseStoredQueuedMessage(raw: string | null): StoredQueuedMessage | null {
 try {
 if (!raw || raw.length > MAX_STORED_MESSAGE_BYTES) return null;
 const value = JSON.parse(raw) as Partial<StoredQueuedMessage>;
 const message = value?.message as Partial<QueuedMessage> | undefined;
 if (
 typeof value?.sessionId !== "string"
 || typeof value.position !== "number"
 || !message
 || typeof message.id !== "string"
 || typeof message.text !== "string"
 || typeof message.createdAtMs !== "number"
 || (message.roleId !== undefined && typeof message.roleId !== "string")
 || (message.roleIds !== undefined && (!Array.isArray(message.roleIds) || message.roleIds.some((id) => typeof id !== "string")))
 || (message.deliveryError !== undefined && typeof message.deliveryError !== "string")
 ) return null;
 if (
 message.attachmentIds !== undefined
 && (!Array.isArray(message.attachmentIds) || message.attachmentIds.some((id) => typeof id !== "string"))
 ) return null;
 return {
 sessionId: value.sessionId,
 position: value.position,
 message: {
 id: message.id,
 text: message.text,
 createdAtMs: message.createdAtMs,
 attachmentIds: Array.from(new Set(message.attachmentIds ?? [])),
 ...((message.roleIds?.length ?? 0) > 0 || message.roleId !== undefined
   ? { roleIds: Array.from(new Set([...(message.roleIds ?? []), ...(message.roleId === undefined ? [] : [message.roleId])])) }
   : {}),
 ...(message.deliveryError === undefined ? {} : { deliveryError: message.deliveryError }),
 },
 };
 } catch {
 return null;
 }
}

export function loadQueuedMessages(storage: Pick<Storage, "length" | "key" | "getItem">, scope: string): Record<string, QueuedMessage[]> {
  const prefix = `${queuedMessagesStorageKey(scope)}:`;
  const records: StoredQueuedMessage[] = [];
  for (let index = 0; index < storage.length && records.length < MAX_STORED_QUEUE_ITEMS; index += 1) {
    const key = storage.key(index);
    if (!key?.startsWith(prefix)) continue;
    const record = parseStoredQueuedMessage(storage.getItem(key));
    if (record && queuedMessagesStorageKey(scope, record.message.id) === key) records.push(record);
  }
  records.sort((left, right) => left.position - right.position || left.message.createdAtMs - right.message.createdAtMs || left.message.id.localeCompare(right.message.id));
  const queues: Record<string, QueuedMessage[]> = {};
  for (const record of records) (queues[record.sessionId] ??= []).push(record.message);
  return queues;
}

export function saveQueuedMessages(
  storage: Pick<Storage, "setItem" | "removeItem">,
  scope: string,
  messages: Record<string, QueuedMessage[]>,
  previous: Readonly<Record<string, readonly QueuedMessage[]>> = {},
) {
  const mutations: Array<{ type: "set"; key: string; value: string } | { type: "remove"; key: string }> = [];
  try {
    const nextIds = new Set(Object.values(messages).flat().map((message) => message.id));
    for (const oldMessage of Object.values(previous).flat()) {
      if (!nextIds.has(oldMessage.id)) mutations.push({ type: "remove", key: queuedMessagesStorageKey(scope, oldMessage.id) });
    }
    for (const [sessionId, queue] of Object.entries(messages)) {
      queue.forEach((message, position) => mutations.push({
        type: "set",
        key: queuedMessagesStorageKey(scope, message.id),
        value: JSON.stringify({ sessionId, position, message } satisfies StoredQueuedMessage),
      }));
    }
    // Write the complete next state before removing obsolete records. A quota
    // failure therefore preserves every previously durable queued message.
    for (const mutation of mutations) {
      if (mutation.type === "set") storage.setItem(mutation.key, mutation.value);
    }
    for (const mutation of mutations) {
      if (mutation.type === "remove") storage.removeItem(mutation.key);
    }
    return true;
  } catch {
    return false;
  }
}

export function reservedQueuedAttachmentIds(messages: readonly QueuedMessage[]) {
 return new Set(messages.flatMap((message) => message.attachmentIds));
}

export function shouldDirectManualMessage(
 sessionState: string,
 queuedMessageCount: number,
 paused: boolean,
) {
 return sessionState === "ready" && queuedMessageCount === 0 && !paused;
}

export type QueuedMessageClaims = Set<string>;

export function queuedMessageKey(sessionId: string, messageId: string) {
  return `${sessionId}\u0000${messageId}`;
}

export function claimQueuedMessage(
  claims: QueuedMessageClaims,
  sessionId: string,
  messages: readonly QueuedMessage[],
  messageId: string,
) {
  if (!messages.some((message) => message.id === messageId)) return false;
  const key = queuedMessageKey(sessionId, messageId);
  if (claims.has(key)) return false;
  claims.add(key);
  return true;
}

export function releaseQueuedMessageClaim(claims: QueuedMessageClaims, sessionId: string, messageId: string) {
  return claims.delete(queuedMessageKey(sessionId, messageId));
}

export function unclaimedQueuedMessages(
 messages: readonly QueuedMessage[],
 claims: ReadonlySet<string>,
 sessionId: string,
) {
 return messages.filter((message) => !claims.has(queuedMessageKey(sessionId, message.id)));
}

export function releaseSessionQueuedMessageClaims(
  claims: QueuedMessageClaims,
  sessionId: string,
) {
  const prefix = `${sessionId}\u0000`;
  let released = 0;
  for (const key of Array.from(claims)) {
    if (!key.startsWith(prefix)) continue;
    claims.delete(key);
    released += 1;
  }
  return released;
}

export function clearSessionQueuedMessages(
  queues: Readonly<Record<string, readonly QueuedMessage[]>>,
  sessionId: string,
): Record<string, QueuedMessage[]> {
  return Object.fromEntries(
    Object.entries(queues)
      .filter(([candidateSessionId]) => candidateSessionId !== sessionId)
      .map(([candidateSessionId, messages]) => [candidateSessionId, [...messages]]),
  );
}

export function applyQueuedMessageAck(
  messages: readonly QueuedMessage[],
  messageId: string,
  status: "accepted" | "committed" | "rejected",
  error: string | undefined,
  replacementId: string,
) {
  if (status === "accepted") return [...messages];
  if (status === "committed") return messages.filter((message) => message.id !== messageId);
  return messages.map((message) => message.id === messageId
    ? { ...message, id: replacementId, deliveryError: error || "发送失败，请重试" }
    : message);
}

export function applyQueuedMessagesAck(
  queues: Readonly<Record<string, readonly QueuedMessage[]>>,
  messageId: string,
  status: "accepted" | "committed" | "rejected",
  error: string | undefined,
  replacementId: string,
) {
  let matchedSessionId: string | undefined;
  const next = Object.fromEntries(Object.entries(queues).map(([sessionId, messages]) => {
    if (!messages.some((message) => message.id === messageId)) return [sessionId, [...messages]];
    matchedSessionId = sessionId;
    return [sessionId, applyQueuedMessageAck(messages, messageId, status, error, replacementId)];
  }));
  return { queues: next, matchedSessionId };
}

export function selectQueuedDispatches(
  sessions: readonly { session_id: string; state: string }[],
  queues: Readonly<Record<string, readonly QueuedMessage[]>>,
  dispatchingSessionIds: ReadonlySet<string>,
  editingSessionId?: string,
) {
  return sessions.flatMap((session) => {
    if (session.state === "working" || dispatchingSessionIds.has(session.session_id) || editingSessionId === session.session_id) return [];
    const message = queues[session.session_id]?.[0];
    return message && !message.deliveryError ? [{ sessionId: session.session_id, message }] : [];
  });
}

export function removeQueuedMessage(
  messages: readonly QueuedMessage[],
  messageId: string,
  claims?: ReadonlySet<string>,
  sessionId = "",
) {
  if (claims?.has(queuedMessageKey(sessionId, messageId))) return [...messages];
  return messages.filter((message) => message.id !== messageId);
}

export function reorderQueuedMessages(
  messages: readonly QueuedMessage[],
  draggedId: string,
  targetId: string,
  claims?: ReadonlySet<string>,
  sessionId = "",
) {
  if (draggedId === targetId) return [...messages];
  if (claims?.has(queuedMessageKey(sessionId, draggedId)) || claims?.has(queuedMessageKey(sessionId, targetId))) return [...messages];
  const from = messages.findIndex((message) => message.id === draggedId);
  const to = messages.findIndex((message) => message.id === targetId);
  if (from < 0 || to < 0) return [...messages];
  const reordered = [...messages];
  const [dragged] = reordered.splice(from, 1);
  reordered.splice(to, 0, dragged);
  return reordered;
}
