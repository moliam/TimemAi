import { describe, expect, it } from "vitest";
import { isNearScrollBottom, preservePrependScrollTop, restoreSessionScrollTop } from "../src/scroll";

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
