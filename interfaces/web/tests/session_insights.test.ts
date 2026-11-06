import { describe, expect, it } from "vitest";
import {
  DEFAULT_INSIGHT_ORDER,
  clampInsightPanelPosition,
  moveInsightItem,
  normalizeInsightOrder,
  parseInsightPanelPosition,
  resourceCpuCapCores,
  resourceCpuCores,
  resourceMemoryCapBytes,
  resourceMemorySeries,
  sparklinePoints,
  type SessionResourcePulseSample,
} from "../src/session_insights";

describe("session insights ordering preferences", () => {
  it("keeps the stored order, drops unknown/duplicate ids, and appends new items", () => {
    expect(normalizeInsightOrder(["mem", "bogus", "mem", "speed"])).toEqual([
      "mem",
      "speed",
      "ttft",
      "cache",
      "context",
      "flow",
      "calls",
      "cpu",
    ]);
    expect(normalizeInsightOrder(null)).toEqual(DEFAULT_INSIGHT_ORDER);
    expect(normalizeInsightOrder({ order: ["mem"] })).toEqual(DEFAULT_INSIGHT_ORDER);
  });

  it("moves one item to the drop target and ignores invalid ranges", () => {
    const order = DEFAULT_INSIGHT_ORDER;
    expect(moveInsightItem(order, 0, 3).slice(0, 4)).toEqual(["ttft", "cache", "context", "speed"]);
    expect(moveInsightItem(order, 7, 0)[0]).toBe("mem");
    expect(moveInsightItem(order, 0, 0)).toBe(order);
    expect(moveInsightItem(order, -1, 2)).toBe(order);
    expect(moveInsightItem(order, 2, 99)).toBe(order);
  });
});

describe("session insights sparklines", () => {
  it("scales values into the requested box and flattens a constant series", () => {
    expect(sparklinePoints([0, 5, 10], 20, 10)).toBe("1.50,8.50 10.00,5.00 18.50,1.50");
    expect(sparklinePoints([7, 7], 10, 10)).toBe("1.50,5.00 8.50,5.00");
    expect(sparklinePoints([4], 10, 10)).toBe("5.00,5.00");
    expect(sparklinePoints([], 10, 10)).toBe("");
  });
});

describe("session insights resource sampling", () => {
  const sample = (
    at: number,
    cpuUsageUsec?: number,
    memBytes?: number,
  ): SessionResourcePulseSample => ({ at, cpuUsageUsec, memBytes });

  it("derives cores from adjacent cumulative counters", () => {
    expect(
      resourceCpuCores([sample(0, 1_000_000), sample(2000, 5_000_000), sample(4000, 6_000_000)]),
    ).toEqual([2, 0.5]);
    // Missing, non-monotonic, or zero-duration samples never claim load.
    expect(
      resourceCpuCores([sample(0, 5_000_000), sample(2000, 1_000_000), sample(2000, 9_000_000)]),
    ).toEqual([]);
    expect(resourceCpuCores([sample(0), sample(2000, 1_000_000)])).toEqual([]);
    expect(resourceCpuCores(undefined)).toEqual([]);
  });

  it("keeps only accounted memory samples for the curve", () => {
    expect(resourceMemorySeries([sample(0, 1, 100), sample(2000, 2), sample(4000, 3, 300)])).toEqual([
      100, 300,
    ]);
    expect(resourceMemorySeries(undefined)).toEqual([]);
  });

  it("reads the active CPU and memory caps without inventing one", () => {
    expect(resourceCpuCapCores({ at: 0, quotaCores: 2.8 })).toBe(2.8);
    expect(resourceCpuCapCores({ at: 0 })).toBeUndefined();
    expect(resourceCpuCapCores(undefined)).toBeUndefined();
    // The configured hard cap wins; memory.high is only a fallback for
    // platforms that expose the throttle point without memory.max.
    expect(
      resourceMemoryCapBytes({ at: 0, memMaxBytes: 2_000_000_000, memHighBytes: 1_800_000_000 }),
    ).toBe(2_000_000_000);
    expect(resourceMemoryCapBytes({ at: 0, memHighBytes: 1_800_000_000 })).toBe(1_800_000_000);
    expect(resourceMemoryCapBytes({ at: 0 })).toBeUndefined();
    expect(resourceMemoryCapBytes(undefined)).toBeUndefined();
  });
});

describe("session insights panel position", () => {
  it("clamps a dragged panel inside the viewport", () => {
    expect(clampInsightPanelPosition({ left: -40, top: -10 }, 300, 1000, 800)).toEqual({
      left: 8,
      top: 8,
    });
    expect(clampInsightPanelPosition({ left: 900, top: 700 }, 300, 1000, 800)).toEqual({
      left: 616,
      top: 492,
    });
  });

  it("parses only finite persisted positions", () => {
    expect(parseInsightPanelPosition('{"left":12,"top":34}')).toEqual({ left: 12, top: 34 });
    expect(parseInsightPanelPosition('{"left":"12","top":34}')).toBeNull();
    expect(parseInsightPanelPosition("not json")).toBeNull();
    expect(parseInsightPanelPosition(null)).toBeNull();
  });
});
