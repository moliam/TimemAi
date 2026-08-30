import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";

const source = readFileSync(new URL("../src/main.tsx", import.meta.url), "utf8");
const styles = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8");

describe("tool activity row layout", () => {
  it("keeps status and duration in one grid cell", () => {
    expect(source).toContain('className="tool-activity-meta"');
    expect(source).toMatch(
      /className="tool-activity-meta"[\s\S]*className="tool-activity-status"[\s\S]*className="tool-activity-duration"/,
    );
  });

  it("keeps the command beside metadata and reserves the final column for the chevron", () => {
    expect(styles).toContain(".tool-activity-command { min-width: 0; grid-column: 4; justify-self: start;");
    expect(styles).toContain(".tool-activity-chevron { grid-column: 5; justify-self: end;");
  });
  it("keeps the top-level background status before the shrinkable tool counts", () => {
    expect(source).toContain('toolActivityGroupStatusLabel(summary)');
    expect(source).toContain('activeParts.push(`bg ${summary.backgroundRunningCount}`)');
    expect(source).toMatch(
      /className="tool-activity-group-status"[\s\S]*className="tool-activity-group-counts"/,
    );
    expect(styles).toContain(
      "grid-template-columns: 16px max-content minmax(0, 1fr) 14px",
    );
    expect(styles).toContain(
      ".tool-activity-group-counts { min-width: 0;",
    );
  });

  it("uses compact aligned terminal labels and always includes the failure count", () => {
    expect(source).toContain('summary.status === "completed") return "Succ"');
    expect(source).toContain('return `Fail(${summary.failedCount})`');
    expect(source).not.toContain('summary.failedCount > 1');
  });

  it("renders live wait-budget countdowns and clarifies timeout handoff", () => {
    expect(source).toContain('className="tool-activity-countdown"');
    expect(source).toContain("formatRemainingDuration(remainingWaitMs)");
    expect(source).toContain("wait ended · process still running · pid");
  });

});
