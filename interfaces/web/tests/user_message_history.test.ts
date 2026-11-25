import { describe, expect, it } from "vitest";
import type { WebTurn, WebTurnUserEntry } from "../src/protocol";
import {
  isSystemTriggeredUserEntry,
  previewUserMessageText,
  userMessageHistoryItems,
  USER_MESSAGE_HISTORY_PREVIEW_CHARS,
} from "../src/user_message_history";

function entry(
  text: string,
  kind: WebTurnUserEntry["kind"] = "task",
  createdAtMs = 1_700_000_000_000,
): WebTurnUserEntry {
  return { kind, text, created_at_ms: createdAtMs };
}

function turn(
  turnId: string,
  entries: WebTurnUserEntry[],
  createdAtMs = 1_700_000_000_000,
): WebTurn {
  return {
    turn_id: turnId,
    state: "completed",
    created_at_ms: createdAtMs,
    user_entries: entries,
    events: [],
  } as WebTurn;
}

describe("user message history", () => {
  it("emits one item per rendered user bubble and keeps turn order", () => {
    const items = userMessageHistoryItems([
      turn("turn-1", [entry("第一条")], 1000),
      turn("turn-2", [entry("第二条")], 2000),
      turn("turn-3", [entry("第三条")], 3000),
    ]);
    expect(items.map((item) => item.turnId)).toEqual([
      "turn-1",
      "turn-2",
      "turn-3",
    ]);
    expect(items.map((item) => item.preview)).toEqual([
      "第一条",
      "第二条",
      "第三条",
    ]);
  });

  it("drops turns whose entries are never rendered as user bubbles", () => {
    const items = userMessageHistoryItems([
      turn("turn-1", [entry("真实输入")]),
      turn("turn-2", [entry("批准", "approval")]),
      turn("turn-3", [entry("直接继续", "resume_directly")]),
      turn("turn-4", []),
    ]);
    expect(items.map((item) => item.turnId)).toEqual(["turn-1"]);
  });

  it("keeps supplements inside their turn instead of creating extra steps", () => {
    const items = userMessageHistoryItems([
      turn("turn-1", [
        entry("主体问题", "task", 1000),
        entry("补充说明", "supplement", 2000),
      ]),
    ]);
    expect(items).toHaveLength(1);
    expect(items[0].entryCount).toBe(2);
    expect(items[0].text).toBe("主体问题\n\n补充说明");
    expect(items[0].createdAtMs).toBe(1000);
  });

  it("prefers human input over machine-triggered entries in a mixed bubble", () => {
    const items = userMessageHistoryItems([
      turn("turn-1", [
        entry("[mailbox] peer 发来 mail: 请继续", "mailbox"),
        entry("把结果整理一下", "task"),
      ]),
    ]);
    expect(items[0].text).toBe("把结果整理一下");
    expect(items[0].entryCount).toBe(2);
  });

  it("falls back to the notice text when the bubble is system-only", () => {
    const items = userMessageHistoryItems([
      turn("turn-1", [entry("[mailbox] peer 发来 mail: 请继续", "mailbox_status")]),
    ]);
    expect(items[0].text).toContain("请继续");
    expect(isSystemTriggeredUserEntry(entry("x", "mailbox_status"))).toBe(true);
    expect(isSystemTriggeredUserEntry(entry("[mailbox] legacy"))).toBe(true);
    expect(isSystemTriggeredUserEntry(entry("普通输入"))).toBe(false);
  });

  it("collapses whitespace and truncates by character count", () => {
    expect(previewUserMessageText("  a\n\n b\tc  ")).toBe("a b c");
    expect(previewUserMessageText("   ")).toBe("");
    const long = "记".repeat(USER_MESSAGE_HISTORY_PREVIEW_CHARS + 10);
    const preview = previewUserMessageText(long);
    expect(Array.from(preview)).toHaveLength(
      USER_MESSAGE_HISTORY_PREVIEW_CHARS + 1,
    );
    expect(preview.endsWith("…")).toBe(true);
    expect(previewUserMessageText("12345", 3)).toBe("123…");
  });
});
