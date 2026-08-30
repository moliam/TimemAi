import { describe, expect, it } from "vitest";
import { adjacentUserMessageIndex, canScrollInDirection, isNearScrollBottom, preservePrependScrollTop, restoreSessionScrollTop, scrollEdgeFades, wheelDeltaPixels } from "../src/scroll";

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


describe("scroll edge fades", () => {
  it("does not fade short content that has no hidden edge", () => {
    expect(scrollEdgeFades({ scrollTop: 0, scrollHeight: 300, clientHeight: 420 })).toEqual({ top: false, bottom: false });
  });

  it("fades only the edge that still has hidden content at each boundary", () => {
    expect(scrollEdgeFades({ scrollTop: 0, scrollHeight: 900, clientHeight: 420 })).toEqual({ top: false, bottom: true });
    expect(scrollEdgeFades({ scrollTop: 240, scrollHeight: 900, clientHeight: 420 })).toEqual({ top: true, bottom: true });
    expect(scrollEdgeFades({ scrollTop: 480, scrollHeight: 900, clientHeight: 420 })).toEqual({ top: true, bottom: false });
  });

  it("absorbs fractional browser scroll offsets at the top and bottom", () => {
    expect(scrollEdgeFades({ scrollTop: 0.5, scrollHeight: 900, clientHeight: 420 })).toEqual({ top: false, bottom: true });
    expect(scrollEdgeFades({ scrollTop: 479.5, scrollHeight: 900, clientHeight: 420 })).toEqual({ top: true, bottom: false });
  });

  it("treats content exactly fitting the viewport as having no hidden edge", () => {
    expect(scrollEdgeFades({ scrollTop: 0, scrollHeight: 420, clientHeight: 420 })).toEqual({ top: false, bottom: false });
  });

  it("does not expose false fades during browser rubber-band overscroll", () => {
    expect(scrollEdgeFades({ scrollTop: -24, scrollHeight: 900, clientHeight: 420 })).toEqual({ top: false, bottom: true });
    expect(scrollEdgeFades({ scrollTop: 520, scrollHeight: 900, clientHeight: 420 })).toEqual({ top: true, bottom: false });
    expect(scrollEdgeFades({ scrollTop: 18, scrollHeight: 300, clientHeight: 420 })).toEqual({ top: false, bottom: false });
  });

  it("lets callers choose the boundary tolerance without accepting a negative tolerance", () => {
    expect(scrollEdgeFades({ scrollTop: 3, scrollHeight: 900, clientHeight: 420 }, 4)).toEqual({ top: false, bottom: true });
    expect(scrollEdgeFades({ scrollTop: 477, scrollHeight: 900, clientHeight: 420 }, 4)).toEqual({ top: true, bottom: false });
    expect(scrollEdgeFades({ scrollTop: 0, scrollHeight: 421, clientHeight: 420 }, 2)).toEqual({ top: false, bottom: false });
    expect(scrollEdgeFades({ scrollTop: 0.1, scrollHeight: 900, clientHeight: 420 }, -4)).toEqual({ top: true, bottom: true });
  });

  it("recomputes cleanly when content changes size", () => {
    expect(scrollEdgeFades({ scrollTop: 0, scrollHeight: 900, clientHeight: 420 })).toEqual({ top: false, bottom: true });
    expect(scrollEdgeFades({ scrollTop: 0, scrollHeight: 300, clientHeight: 420 })).toEqual({ top: false, bottom: false });
    expect(scrollEdgeFades({ scrollTop: 0, scrollHeight: 1200, clientHeight: 420 })).toEqual({ top: false, bottom: true });
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


describe("user message navigation", () => {
  const anchorTops = [-420, -18, 164, 740];

  it("selects the closest prior and next user message around the viewport top", () => {
    expect(adjacentUserMessageIndex(anchorTops, 0, "previous")).toBe(1);
    expect(adjacentUserMessageIndex(anchorTops, 0, "next")).toBe(2);
  });

  it("skips the message already aligned with the viewport top", () => {
    expect(adjacentUserMessageIndex([-120, 2, 260], 0, "previous")).toBe(0);
    expect(adjacentUserMessageIndex([-120, 2, 260], 0, "next")).toBe(2);
  });

  it("reports boundaries when no user message exists in that direction", () => {
    expect(adjacentUserMessageIndex([12, 240], 0, "previous")).toBe(-1);
    expect(adjacentUserMessageIndex([-400, -20], 0, "next")).toBe(-1);
    expect(adjacentUserMessageIndex([], 0, "next")).toBe(-1);
  });
});
