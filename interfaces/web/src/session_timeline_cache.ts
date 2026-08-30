/**
 * Reconciles a small least-recently-used set of mounted Session timelines.
 * The returned order is oldest to newest, so appending the active Session and
 * trimming from the front implements deterministic LRU eviction.
 */
export function reconcileSessionTimelineCache(
  previous: readonly string[],
  activeSessionId: string | undefined,
  liveSessionIds: readonly string[],
  capacity: number,
): string[] {
  if (capacity <= 0) return [];
  const live = new Set(liveSessionIds);
  const next: string[] = [];
  for (const sessionId of previous) {
    if (!live.has(sessionId) || sessionId === activeSessionId || next.includes(sessionId)) continue;
    next.push(sessionId);
  }
  if (activeSessionId && live.has(activeSessionId)) next.push(activeSessionId);
  return next.slice(-capacity);
}
