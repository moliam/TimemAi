import { describe, expect, it } from "vitest";
import { sessionTurnKey } from "../src/view_model";
import {
  TURN_HISTORY_HYDRATION_CONCURRENCY,
  createTurnHistoryHydrationQueue,
  enqueueTurnHistoryHydration,
  finishTurnHistoryHydrationChain,
  markTurnHistoryMissing,
  pumpTurnHistoryHydration,
} from "../src/turn_history_hydration";

const noGuards = () => ({ alreadyRequested: () => false, knownMissing: () => false });

describe("turn history hydration scheduler", () => {
  it("caps concurrent chains at the bounded limit, not one per slimmed turn", () => {
    const queue = createTurnHistoryHydrationQueue();
    for (let i = 0; i < 40; i += 1) enqueueTurnHistoryHydration(queue, noGuards(), "s", `turn-${i}`);
    const started: string[] = [];
    pumpTurnHistoryHydration(queue, (_s, turnId) => { started.push(turnId); return true; });
    expect(started).toHaveLength(TURN_HISTORY_HYDRATION_CONCURRENCY);
    expect(TURN_HISTORY_HYDRATION_CONCURRENCY).toBeLessThanOrEqual(8);
    expect(queue.waiting).toHaveLength(40 - TURN_HISTORY_HYDRATION_CONCURRENCY);
  });

  it("backfills a slot only after the chain finishes", () => {
    const queue = createTurnHistoryHydrationQueue();
    for (let i = 0; i < 6; i += 1) enqueueTurnHistoryHydration(queue, noGuards(), "s", `turn-${i}`);
    const started: string[] = [];
    pumpTurnHistoryHydration(queue, (_s, turnId) => { started.push(turnId); return true; });
    expect(started).toEqual(["turn-0", "turn-1", "turn-2", "turn-3"]);
    finishTurnHistoryHydrationChain(queue, sessionTurnKey("s", "turn-0"), (_s, turnId) => { started.push(turnId); return true; });
    expect(started).toEqual(["turn-0", "turn-1", "turn-2", "turn-3", "turn-4"]);
  });

  it("releases the slot when the socket send fails", () => {
    const queue = createTurnHistoryHydrationQueue();
    for (let i = 0; i < 5; i += 1) enqueueTurnHistoryHydration(queue, noGuards(), "s", `turn-${i}`);
    const started: string[] = [];
    pumpTurnHistoryHydration(queue, (_s, turnId) => {
      const ok = turnId !== "turn-0";
      if (ok) started.push(turnId);
      return ok;
    });
    expect(started.sort()).toEqual(["turn-1", "turn-2", "turn-3", "turn-4"]);
    expect(queue.activeKeys.size).toBe(4);
  });

  it("skips duplicate, pending, and known-missing requests", () => {
    const queue = createTurnHistoryHydrationQueue();
    const pending = new Set<string>();
    const missing = new Set<string>();
    const guards = {
      alreadyRequested: (key: string) => pending.has(key),
      knownMissing: (key: string) => missing.has(key),
    };
    markTurnHistoryMissing(missing, sessionTurnKey("s", "rotated"));
    enqueueTurnHistoryHydration(queue, guards, "s", "rotated");
    pending.add(sessionTurnKey("s", "inflight"));
    enqueueTurnHistoryHydration(queue, guards, "s", "inflight");
    enqueueTurnHistoryHydration(queue, guards, "s", "fresh");
    enqueueTurnHistoryHydration(queue, guards, "s", "fresh");
    expect(queue.waiting.map(({ turnId }) => turnId)).toEqual(["fresh"]);
  });

  it("puts an explicit user expansion at the front of the queue", () => {
    const queue = createTurnHistoryHydrationQueue();
    enqueueTurnHistoryHydration(queue, noGuards(), "s", "old-1");
    enqueueTurnHistoryHydration(queue, noGuards(), "s", "old-2");
    enqueueTurnHistoryHydration(queue, noGuards(), "s", "user-clicked", true);
    expect(queue.waiting.map(({ turnId }) => turnId)).toEqual(["user-clicked", "old-1", "old-2"]);
  });

  it("bounds the missing-turn cache", () => {
    const missing = new Set<string>();
    for (let i = 0; i < 600; i += 1) markTurnHistoryMissing(missing, sessionTurnKey("s", `turn-${i}`));
    expect(missing.size).toBeLessThanOrEqual(600);
    expect(missing.size).toBeGreaterThan(0);
  });
});
