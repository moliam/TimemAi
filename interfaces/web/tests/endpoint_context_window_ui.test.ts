import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import {
  applyContextWindowChoice,
  contextWindowSelectionMode,
} from "../src/model_endpoints";

const read = (relative: string): string =>
  readFileSync(join(import.meta.dirname, "..", "src", relative), "utf8");

describe("endpoint context window mapping", () => {
  it("keeps an explicitly requested custom mode at preset and model values", () => {
    expect(contextWindowSelectionMode(100_000, 200_000, false)).toBe("100000");
    expect(contextWindowSelectionMode(200_000, 200_000, false)).toBe("model");
    // Choosing custom must win over the coincidental preset value.
    expect(contextWindowSelectionMode(100_000, 200_000, true)).toBe("custom");
    expect(contextWindowSelectionMode(123_456, 200_000, false)).toBe("custom");
    expect(contextWindowSelectionMode(123_456, undefined, false)).toBe("custom");
  });

  it("maps the unified window onto the input budget without touching output", () => {
    type Draft = Parameters<typeof applyContextWindowChoice>[0];
    const draft = {
      max_llm_input_tokens: 1_000,
      max_llm_output_tokens: 20_000,
    } as unknown as Draft;
    const wide = applyContextWindowChoice(draft, 200_000);
    expect(wide.max_llm_input_tokens).toBe(180_000);
    expect(wide.max_llm_output_tokens).toBe(20_000);
    // A window smaller than the output budget clamps input to one token
    // instead of going negative.
    expect(applyContextWindowChoice(draft, 5_000).max_llm_input_tokens).toBe(1);
  });
});

describe("unified endpoint context window UI", () => {
  it("wires the unified window select with presets, model shortcut and warning", () => {
    const main = read("main.tsx");
    expect(main).toContain("endpointContextWindowWarning(draft, selectedModel)");
    expect(main).toContain("applyContextWindowChoice(current, tokens)");
    expect(main).toContain('t("endpoints.contextWindowOverLimit"');
    expect(main).toContain("endpoint-window-badge");
    expect(main).toContain("MODEL_CONTEXT_WINDOW_OPTIONS.map");
    expect(main).toContain("contextWindowSelectionMode");
    expect(main).toContain("contextWindowCustomMode");
    expect(main).toContain("contextWindowCustomDraft");
    expect(main).toContain("applyContextWindowTotal");
    expect(main).not.toContain('code === "context_budget"');
  });

  it("downgrades the context-budget capability check to a warning-only helper", () => {
    const source = read("model_endpoints.ts");
    expect(source).not.toContain('"context_budget"');
    expect(source).toContain("export function applyContextWindowChoice");
    expect(source).toContain("export function endpointContextWindowWarning");
  });

  it("adds copy affordance and short label to memo tooltips", () => {
    const main = read("main.tsx");
    expect(main).toContain("MemoCopyButton");
    expect(main).toContain("thread-memo-copy");
    expect(main).toContain("thread-memo-caption-row");
    const zh = read("i18n/strings.zh.ts");
    const en = read("i18n/strings.en.ts");
    expect(zh).toContain('memoIndicator: "备忘录"');
    expect(en).toContain('memoIndicator: "Memo"');
    for (const catalog of [zh, en]) {
      expect(catalog).toContain("contextWindowOverLimit");
      expect(catalog).not.toContain("validationContextBudget");
    }
    const styles = read("styles.css");
    expect(styles).toContain(".endpoint-context-window-warning");
    expect(styles).toContain(".thread-memo-copy");
  });

  it("keeps the memo tooltips reachable for the copy pointer path", () => {
    const main = read("main.tsx");
    expect(main).toContain("workingMemoHideTimerRef");
    expect(main).toContain("workingMemoPointerInsideRef");
    expect(main).toContain("handleWorkingMemoTooltipEnter");
    expect(main).toContain("visible={workingMemoVisible}");
    expect(main).toContain("onPointerEnter={onPointerEnter}");
    const styles = read("styles.css");
    expect(styles).toContain(".thread-memo-indicator:focus-within .thread-memo-tooltip");
    expect(styles).toContain("pointer-events: auto;");
    expect(styles).toContain("translateX(-10px)");
  });
});
