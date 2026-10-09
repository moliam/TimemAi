import { describe, expect, it } from "vitest";
import {
  shouldRenderTurnWorkFrame,
  turnElapsedMs,
  turnWorkPhaseBoundary,
} from "../src/view_model";

describe("working panel behavior", () => {
  it("shows the formal work frame before the first visible process event", () => {
    expect(shouldRenderTurnWorkFrame("working", false, false)).toBe(true);
  });

  it("keeps historical process visible but does not invent a frame for idle turns", () => {
    expect(shouldRenderTurnWorkFrame("finished", false, true)).toBe(true);
    expect(shouldRenderTurnWorkFrame("finished", false, false)).toBe(false);
    expect(shouldRenderTurnWorkFrame("working", true, false)).toBe(false);
  });

  it("freezes interrupted elapsed time and clamps invalid clock order", () => {
    expect(turnElapsedMs(1_000, 99_000, 6_500)).toBe(5_500);
    expect(turnElapsedMs(1_000, 8_000)).toBe(7_000);
    expect(turnElapsedMs(8_000, 7_000)).toBe(0);
  });

  it("starts each work phase at its authoritative model boundary", () => {
    const turn = {
      turn_id: "turn-phase",
      state: "working",
      created_at_ms: 1_000,
      user_entries: [],
      events: [
        { event_id: "response-1", source: "worker_activity", created_at_ms: 5_000, payload: { kind: "model_response", round: 1 } },
        { event_id: "request-2", source: "worker_activity", created_at_ms: 8_000, payload: { kind: "model_request", round: 2 } },
        { event_id: "request-1", source: "worker_activity", created_at_ms: 2_000, payload: { kind: "model_request", round: 1 } },
        { event_id: "response-2", source: "worker_activity", created_at_ms: 12_000, payload: { kind: "model_response", round: 2 } },
      ],
    };
    expect(turnWorkPhaseBoundary(turn, "model")).toEqual({ id: "request-2", startedAtMs: 8_000 });
    expect(turnWorkPhaseBoundary(turn, "local")).toEqual({ id: "response-2", startedAtMs: 12_000 });
  });

  it("does not reuse an earlier local segment while the current response is pending", () => {
    const turn = {
      turn_id: "turn-next-response",
      state: "working",
      created_at_ms: 1_000,
      user_entries: [],
      events: [
        { event_id: "request-1", source: "worker_activity", created_at_ms: 2_000, timeline_seq: 1, payload: { kind: "model_request" } },
        { event_id: "response-1", source: "worker_activity", created_at_ms: 3_000, timeline_seq: 2, payload: { kind: "model_response" } },
        { event_id: "request-2", source: "worker_activity", created_at_ms: 4_000, timeline_seq: 3, payload: { kind: "model_request" } },
      ],
    };
    expect(turnWorkPhaseBoundary(turn, "local")).toEqual({ id: "pending:local" });
    expect(turnWorkPhaseBoundary(turn, "model")).toEqual({ id: "request-2", startedAtMs: 4_000 });
  });

  it("does not invent same-millisecond ordering without a timeline sequence", () => {
    const turn = {
      turn_id: "turn-ambiguous-ms",
      state: "working",
      created_at_ms: 1_000,
      user_entries: [],
      events: [
        { event_id: "request-z", source: "worker_activity", created_at_ms: 2_000, payload: { kind: "model_request" } },
        { event_id: "response-a", source: "worker_activity", created_at_ms: 2_000, payload: { kind: "model_response" } },
      ],
    };
    expect(turnWorkPhaseBoundary(turn, "local")).toEqual({ id: "pending:local" });
  });

  it("uses timeline order when request and response share one millisecond", () => {
    const turn = {
      turn_id: "turn-same-ms",
      state: "working",
      created_at_ms: 1_000,
      user_entries: [],
      events: [
        { event_id: "request", source: "worker_activity", created_at_ms: 2_000, timeline_seq: 10, payload: { kind: "model_request" } },
        { event_id: "response", source: "worker_activity", created_at_ms: 2_000, timeline_seq: 11, payload: { kind: "model_response" } },
      ],
    };
    expect(turnWorkPhaseBoundary(turn, "local")).toEqual({ id: "response", startedAtMs: 2_000 });
  });

  it("uses the Turn start for local work before the first model response", () => {
    const turn = {
      turn_id: "turn-initial-local",
      state: "working",
      created_at_ms: 3_000,
      user_entries: [],
      events: [],
    };
    expect(turnWorkPhaseBoundary(turn, "local")).toEqual({ id: "turn:turn-initial-local", startedAtMs: 3_000 });
    expect(turnWorkPhaseBoundary(turn, "model")).toEqual({ id: "pending:model" });
  });

});
