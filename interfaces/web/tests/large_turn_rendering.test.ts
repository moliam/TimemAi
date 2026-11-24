import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";

const source = readFileSync(new URL("../src/main.tsx", import.meta.url), "utf8");
const styles = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8");
const zh = readFileSync(new URL("../src/i18n/strings.zh.ts", import.meta.url), "utf8");
const en = readFileSync(new URL("../src/i18n/strings.en.ts", import.meta.url), "utf8");

describe("large turn rendering", () => {
  it("renders only a bounded tail of a turn's process rows", () => {
    const turn = source.slice(
      source.indexOf("const TURN_WORK_RENDER_LIMIT"),
      source.indexOf("function TurnAnswerDelivery"),
    );
    expect(turn).toContain("const TURN_WORK_RENDER_LIMIT = 240;");
    // Paging slides a fixed window instead of growing the rendered list.
    expect(turn).toContain("workItemsAll.slice(workItemStart, workItemEnd)");
    expect(turn).toContain("olderWorkItemCount > 0");
    expect(turn).toContain("newerWorkItemCount > 0");
    expect(turn).toContain("turn-work-load-earlier");
    expect(turn).toContain("turn-work-load-newer");
    expect(turn).toContain('t("turn.loadEarlierWork"');
    expect(turn).toContain('t("turn.loadNewerWork"');
    expect(styles).toContain(".turn-work-load-earlier");
    expect(zh).toContain("loadEarlierWork:");
    expect(en).toContain("loadEarlierWork:");
  });
});
