import { describe, expect, it } from "vitest";
import {
  shouldRenderTurnWorkFrame,
  turnElapsedMs,
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
});
