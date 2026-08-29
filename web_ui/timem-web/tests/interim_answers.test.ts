import { describe, expect, it } from "vitest";
import { newestInterimAnswersFirst } from "../src/interim_answers";

describe("newestInterimAnswersFirst", () => {
  it("shows the last delivered interim answer first without mutating the source", () => {
    const source = [
      { id: "first" },
      { id: "second" },
      { id: "third" },
    ];

    expect(newestInterimAnswersFirst(source)).toEqual([
      { item: { id: "third" }, ordinal: 3 },
      { item: { id: "second" }, ordinal: 2 },
      { item: { id: "first" }, ordinal: 1 },
    ]);
    expect(source.map((item) => item.id)).toEqual(["first", "second", "third"]);
  });

  it("handles empty and single-answer lists", () => {
    expect(newestInterimAnswersFirst([])).toEqual([]);
    expect(newestInterimAnswersFirst(["only"])).toEqual([{ item: "only", ordinal: 1 }]);
  });
});
