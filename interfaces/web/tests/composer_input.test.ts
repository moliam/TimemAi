import { describe, expect, it } from "vitest";
import { createComposerWriteThrough } from "../src/composer_input";

function harness(intervalMs: number) {
  let now = 0;
  const timers = new Map<number, { callback: () => void; at: number }>();
  let nextId = 1;
  const commits: string[] = [];
  const wt = createComposerWriteThrough({
    commit: (text) => commits.push(text),
    now: () => now,
    schedule: (callback, delayMs) => {
      const id = nextId++;
      timers.set(id, { callback, at: now + delayMs });
      return id;
    },
    cancel: (id) => void timers.delete(id),
    intervalMs,
  });
  const advance = (ms: number) => {
    const end = now + ms;
    for (;;) {
      const due = [...timers.entries()]
        .filter(([, timer]) => timer.at <= end)
        .sort((a, b) => a[1].at - b[1].at)[0];
      if (!due) break;
      timers.delete(due[0]);
      now = Math.max(now, due[1].at);
      due[1].callback();
    }
    now = end;
  };
  return { wt, advance, commits, pending: () => timers.size };
}

describe("composer write-through scheduler", () => {
  it("keeps continuous typing at most one interval behind", () => {
    const h = harness(300);
    for (let i = 1; i <= 20; i++) {
      h.wt.onInput("x".repeat(i));
      h.advance(50);
    }
    // 1s of typing at 20 keys/s: commits happen on fixed ticks, not per key,
    // and the trailing key still lands after one final interval.
    expect(h.commits.length).toBeLessThanOrEqual(4);
    expect(h.pending()).toBe(1);
    h.advance(400);
    expect(h.commits.at(-1)).toBe("x".repeat(20));
  });

  it("commits the newest text only and never drops a trailing change", () => {
    const h = harness(300);
    h.wt.onInput("a");
    h.advance(100);
    h.wt.onInput("ab");
    h.advance(400);
    expect(h.commits).toEqual(["ab"]);
    h.wt.onInput("abc");
    h.wt.flush();
    expect(h.commits).toEqual(["ab", "abc"]);
  });

  it("reset drops pending typing without committing", () => {
    const h = harness(300);
    h.wt.onInput("pending");
    h.wt.reset();
    h.advance(1000);
    expect(h.commits).toEqual([]);
    expect(h.pending()).toBe(0);
  });

  it("reopen cadence: after an idle gap the next key commits quickly", () => {
    const h = harness(300);
    h.wt.onInput("first");
    h.advance(1000);
    h.wt.onInput("second");
    expect(h.commits).toEqual(["first"]);
    h.advance(1);
    expect(h.commits).toEqual(["first", "second"]);
  });
});
