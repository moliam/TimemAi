import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const source = readFileSync(new URL("../src/main.tsx", import.meta.url), "utf8");
const styles = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8");

describe("reasoning usage display", () => {
  it("integrates think tokens into the completion status bar instead of a mid-turn badge", () => {
    expect(source).toContain(
      "formatOutputWithThink(stats.completion_tokens, stats.reasoning_tokens)",
    );
    expect(source).toContain('className="completion-icon-fact"');
    expect(source).toContain('t("completion.llmCallsTitle")');
    // The mid-turn "<+ 思考 N Tokens >" badge is retired: no marker element,
    // no badge CSS, no badge-only i18n lookups.
    expect(source).not.toContain("answer-reasoning-usage");
    expect(source).not.toContain("usage.reasoningAria");
    expect(styles).not.toContain(".answer-reasoning-usage");
  });
});
