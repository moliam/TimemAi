import { describe, expect, it } from "vitest";
import { classifyEventSequence, snapshotEventBaseline } from "../src/event_cursor";

describe("semantic event cursor", () => {
  it("accepts only the next live sequence and detects duplicates and gaps", () => {
    expect(classifyEventSequence(7, 7)).toBe("duplicate");
    expect(classifyEventSequence(7, 6)).toBe("duplicate");
    expect(classifyEventSequence(7, 8)).toBe("next");
    expect(classifyEventSequence(7, 9)).toBe("gap");
  });

  it("does not advance after a gap", () => {
    let cursor = 4;
    for (const incoming of [4, 5, 5, 6]) {
      if (classifyEventSequence(cursor, incoming) === "next") cursor = incoming;
    }
    expect(cursor).toBe(6);
    expect(classifyEventSequence(cursor, 8)).toBe("gap");
    expect(cursor).toBe(6);
    expect(classifyEventSequence(cursor, 7)).toBe("next");
  });
});
describe("snapshot event baseline", () => {
  it("adopts an exact non-zero Host cursor for a fresh or reconnected browser", () => {
    expect(snapshotEventBaseline(40)).toBe(40);
    expect(classifyEventSequence(snapshotEventBaseline(40), 41)).toBe("next");
  });

  it.each([
    undefined,
    -1,
    Number.NaN,
    Number.POSITIVE_INFINITY,
    Number.MAX_SAFE_INTEGER + 1,
  ])("fails closed for an invalid protocol cursor %s", (cursor) => {
    expect(snapshotEventBaseline(cursor)).toBe(0);
  });
});
