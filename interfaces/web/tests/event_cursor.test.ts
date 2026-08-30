import { describe, expect, it } from "vitest";
import { classifyEventSequence } from "../src/event_cursor";

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
