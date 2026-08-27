import { describe, expect, it } from "vitest";
import { reconcileSessionTimelineCache } from "../src/session_timeline_cache";

describe("session timeline cache", () => {
  it("keeps the active session and the most recently visited inactive session", () => {
    expect(reconcileSessionTimelineCache([], "session-a", ["session-a", "session-b", "session-c"], 2))
      .toEqual(["session-a"]);
    expect(reconcileSessionTimelineCache(["session-a"], "session-b", ["session-a", "session-b", "session-c"], 2))
      .toEqual(["session-a", "session-b"]);
    expect(reconcileSessionTimelineCache(["session-a", "session-b"], "session-a", ["session-a", "session-b", "session-c"], 2))
      .toEqual(["session-b", "session-a"]);
  });

  it("keeps repeated warm A/B switching inside the two-pane cache", () => {
    let cache: string[] = [];
    for (let index = 0; index < 10_000; index += 1) {
      cache = reconcileSessionTimelineCache(cache, index % 2 === 0 ? "session-a" : "session-b", ["session-a", "session-b"], 2);
      expect(cache.length).toBeLessThanOrEqual(2);
      expect(cache).toContain(index % 2 === 0 ? "session-a" : "session-b");
    }
    expect(new Set(cache)).toEqual(new Set(["session-a", "session-b"]));
  });

  it("evicts the least recently visited session when capacity is reached", () => {
    expect(reconcileSessionTimelineCache(["session-a", "session-b"], "session-c", ["session-a", "session-b", "session-c"], 2))
      .toEqual(["session-b", "session-c"]);
  });

  it("prunes deleted sessions before applying the active selection", () => {
    expect(reconcileSessionTimelineCache(["deleted", "session-a"], "session-b", ["session-a", "session-b"], 2))
      .toEqual(["session-a", "session-b"]);
  });

  it("handles no active session, duplicate ids, and non-positive capacity", () => {
    expect(reconcileSessionTimelineCache(["session-a", "session-a", "deleted"], undefined, ["session-a"], 2))
      .toEqual(["session-a"]);
    expect(reconcileSessionTimelineCache(["session-a"], "session-a", ["session-a"], 0)).toEqual([]);
  });
});
