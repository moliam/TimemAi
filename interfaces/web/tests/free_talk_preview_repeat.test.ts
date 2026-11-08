import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { repeatsRetainedFreeTalk } from "../src/activity_groups";

describe("free-talk preview repeat suppression", () => {
  it("flags a preview paragraph that repeats retained commentary verbatim", () => {
    const retained = new Set([
      "继续修复两个失败。先拿失败详情：core_tests 重写测试的实际 prompt 内容。",
    ]);
    expect(
      repeatsRetainedFreeTalk(
        "继续修复两个失败。先拿失败详情：core_tests 重写测试的实际 prompt 内容。",
        retained,
      ),
    ).toBe(true);
  });

  it("ignores trailing whitespace differences, not new information", () => {
    const retained = new Set(["same commentary line"]);
    expect(repeatsRetainedFreeTalk("same commentary line  \n", retained)).toBe(true);
  });

  it("keeps genuinely new commentary and empty drafts visible", () => {
    const retained = new Set(["same commentary line"]);
    expect(repeatsRetainedFreeTalk("a different paragraph", retained)).toBe(false);
    expect(repeatsRetainedFreeTalk("", retained)).toBe(false);
    expect(repeatsRetainedFreeTalk("   ", retained)).toBe(false);
  });

  it("wires the suppression into the provisional answer preview", () => {
    const source = readFileSync(new URL("../src/main.tsx", import.meta.url), "utf8");
    const delivery = source.slice(
      source.indexOf("function TurnAnswerDelivery"),
      source.indexOf("function ContextCompressNotice"),
    );
    expect(delivery).toContain("const rawPreviewText = intermediate");
    expect(delivery).toContain("repeatsRetainedFreeTalk(rawPreviewText, retainedFreeTalkTexts)");
    // The retained-commentary set covers every free_talk the stream shows,
    // not only the latest one.
    expect(delivery).toContain('activity.kind === "free_talk"');
  });
});
