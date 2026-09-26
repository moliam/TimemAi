import { describe, expect, it } from "vitest";
import { interimAnswerPresentation, newestInterimAnswersFirst } from "../src/interim_answers";

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

describe("interimAnswerPresentation", () => {
  const answer = { sub_answer_id: "a", ordinal: 1, task: "Task", answer: "Same text", created_at_ms: 10, preview_attempt: 2, preview_index: 0 };
  it("renders an accepted preview once and keeps its stable identity", () => {
    const result = interimAnswerPresentation([answer], { attempt: 2, revision: 1, chat: [{ index: 0, task: "Task", answer: "partial" }] });
    expect(result).toHaveLength(1);
    expect(result[0]).toMatchObject({ key: "preview-2-0", answer: "Same text", provisional: false });
  });
  it("keeps equal text from different deliveries and puts the newest first", () => {
    const result = interimAnswerPresentation([answer, { ...answer, sub_answer_id: "b", ordinal: 2, preview_index: 1 }]);
    expect(result.map(item => item.ordinal)).toEqual([2, 1]);
  });
  it("does not deduplicate across attempts and supports preview retraction", () => {
    expect(interimAnswerPresentation([answer], { attempt: 3, revision: 1, chat: [{ index: 0, task: "Task", answer: "Same text" }] })).toHaveLength(2);
    expect(interimAnswerPresentation([])).toEqual([]);
  });
});
