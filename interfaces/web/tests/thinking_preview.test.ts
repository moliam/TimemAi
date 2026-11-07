import { describe, expect, it } from "vitest";
import {
  collapseThinkingText,
  compactThinkingTail,
} from "../src/thinking_preview";

describe("thinking preview", () => {
  it("collapses reasoning whitespace for the one-line window", () => {
    expect(collapseThinkingText(" first\n\n  second\tthird ")).toBe(
      "first second third",
    );
  });

  it("keeps the newest characters and marks compaction", () => {
    expect(compactThinkingTail("abcdef", 4)).toBe("…cdef");
    expect(compactThinkingTail("abcdef", 6)).toBe("abcdef");
    expect(compactThinkingTail("   ", 4)).toBe("");
  });

  it("does not split surrogate pairs when compacting", () => {
    expect(compactThinkingTail("a😀b😀c", 2)).toBe("…😀c");
  });
});
