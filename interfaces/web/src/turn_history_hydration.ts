import { sessionTurnKey } from "./view_model";

/**
 * Slimmed-turn hydration scheduler.
 *
 * Stream-UI hydration fans `turn_history_page` chains out for every slimmed
 * restored turn at once. Each chain is sequential, but N parallel chains put N
 * commands in flight; with a large restored timeline that alone exceeds the
 * Host's per-connection command queue (capacity 32), which then rejects
 * uncued read commands with `browser_command_queue_full`. The scheduler keeps
 * at most TURN_HISTORY_HYDRATION_CONCURRENCY chains running and queues the
 * rest, so timeline hydration can never crowd out other live commands.
 */
export const TURN_HISTORY_HYDRATION_CONCURRENCY = 4;

/**
 * Negative cache cap for turns whose records retention already rotated away.
 * A capped clear-all is deliberate: re-requesting a missing turn once is a
 * cheap empty page, so the cache self-heals after a reset.
 */
export const TURN_HISTORY_MISSING_CACHE_LIMIT = 512;

export type TurnHistoryHydrationRequest = { sessionId: string; turnId: string; key: string };

export type TurnHistoryHydrationQueue = {
  waiting: TurnHistoryHydrationRequest[];
  activeKeys: Set<string>;
};

export function createTurnHistoryHydrationQueue(): TurnHistoryHydrationQueue {
  return { waiting: [], activeKeys: new Set() };
}

/**
 * Queue one hydration request unless its chain is already in flight, already
 * waiting, already tracked by the caller's pending map, or known to be missing
 * from stored history. `priority` (explicit user expansion) jumps the queue.
 */
export function enqueueTurnHistoryHydration(
  queue: TurnHistoryHydrationQueue,
  guards: { alreadyRequested: (key: string) => boolean; knownMissing: (key: string) => boolean },
  sessionId: string,
  turnId: string,
  priority = false,
): void {
  const key = sessionTurnKey(sessionId, turnId);
  if (guards.alreadyRequested(key) || guards.knownMissing(key)) return;
  if (queue.activeKeys.has(key)) return;
  if (queue.waiting.some((item) => item.key === key)) return;
  const request = { sessionId, turnId, key };
  if (priority) queue.waiting.unshift(request);
  else queue.waiting.push(request);
}

/**
 * Start waiting chains while free slots remain. `start` owns the caller's
 * pending entry; returning false (socket send failed) releases the slot.
 */
export function pumpTurnHistoryHydration(
  queue: TurnHistoryHydrationQueue,
  start: (sessionId: string, turnId: string) => boolean,
): void {
  while (
    queue.waiting.length > 0
    && queue.activeKeys.size < TURN_HISTORY_HYDRATION_CONCURRENCY
  ) {
    const request = queue.waiting.shift()!;
    if (queue.activeKeys.has(request.key)) continue;
    queue.activeKeys.add(request.key);
    if (!start(request.sessionId, request.turnId)) {
      queue.activeKeys.delete(request.key);
    }
  }
}

/** Release one finished or failed chain and immediately backfill its slot. */
export function finishTurnHistoryHydrationChain(
  queue: TurnHistoryHydrationQueue,
  key: string,
  start: (sessionId: string, turnId: string) => boolean,
): void {
  queue.activeKeys.delete(key);
  pumpTurnHistoryHydration(queue, start);
}

/**
 * Remember a turn whose stored records are gone (full or leading-record
 * retention rotation): it can never hydrate, so never request it again.
 */
export function markTurnHistoryMissing(missing: Set<string>, key: string): void {
  missing.add(key);
  if (missing.size > TURN_HISTORY_MISSING_CACHE_LIMIT) missing.clear();
}
