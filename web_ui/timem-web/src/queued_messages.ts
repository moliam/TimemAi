export const COLLAPSED_QUEUE_LIMIT = 4;

export type QueuedMessage = {
  id: string;
  text: string;
  createdAtMs: number;
};

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
