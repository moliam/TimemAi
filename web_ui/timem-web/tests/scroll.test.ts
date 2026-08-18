import { describe, expect, it } from "vitest";
import { canScrollInDirection, isNearScrollBottom, preservePrependScrollTop, restoreSessionScrollTop, wheelDeltaPixels } from "../src/scroll";

describe("progressive history scroll anchoring", () => {
  it("keeps the same content under the viewport after older content is prepended", () => {
    expect(preservePrependScrollTop({ scrollTop: 5364.5, scrollHeight: 6002 }, 7388)).toBe(6750.5);
  });

  it("does not depend on a transient browser scrollTop reset", () => {
    const previous = { scrollTop: 420, scrollHeight: 1000 };
    expect(preservePrependScrollTop(previous, 1600)).toBe(1020);
  });

  it("does not move backward when layout height is unchanged or smaller", () => {
    const previous = { scrollTop: 240, scrollHeight: 900 };
    expect(preservePrependScrollTop(previous, 900)).toBe(240);
    expect(preservePrependScrollTop(previous, 800)).toBe(240);
  });

  it("follows new work only while the reader remains near the bottom", () => {
    expect(isNearScrollBottom({ scrollTop: 928, scrollHeight: 1600, clientHeight: 600 })).toBe(true);
    expect(isNearScrollBottom({ scrollTop: 700, scrollHeight: 1600, clientHeight: 600 })).toBe(false);
  });

  it("restores each session position without a cross-session scroll animation", () => {
    expect(restoreSessionScrollTop({ scrollTop: 480, followLatest: false }, 2400)).toBe(480);
    expect(restoreSessionScrollTop({ scrollTop: 480, followLatest: true }, 2400)).toBe(2400);
    expect(restoreSessionScrollTop(undefined, 2400)).toBe(2400);
    expect(restoreSessionScrollTop({ scrollTop: 3000, followLatest: false }, 2400)).toBe(2400);
  });
});


describe("composer wheel ownership", () => {
  it("keeps wheel input inside a multiline composer while that direction can still scroll", () => {
    expect(canScrollInDirection({ scrollTop: 80, scrollHeight: 420, clientHeight: 140 }, -24)).toBe(true);
    expect(canScrollInDirection({ scrollTop: 80, scrollHeight: 420, clientHeight: 140 }, 24)).toBe(true);
  });

  it("hands wheel input back to the chat viewport at the matching textarea boundary", () => {
    expect(canScrollInDirection({ scrollTop: 0, scrollHeight: 420, clientHeight: 140 }, -24)).toBe(false);
    expect(canScrollInDirection({ scrollTop: 280, scrollHeight: 420, clientHeight: 140 }, 24)).toBe(false);
    expect(canScrollInDirection({ scrollTop: 0, scrollHeight: 120, clientHeight: 140 }, 24)).toBe(false);
    expect(canScrollInDirection({ scrollTop: 80, scrollHeight: 420, clientHeight: 140 }, 0)).toBe(false);
  });

  it("normalizes pixel, line, and page wheel deltas before moving the textarea", () => {
    expect(wheelDeltaPixels(12, 0, 140)).toBe(12);
    expect(wheelDeltaPixels(3, 1, 140)).toBe(48);
    expect(wheelDeltaPixels(-1, 2, 140)).toBe(-140);
  });
});
