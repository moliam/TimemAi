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

  it("uses compact terminal labels and always shows the failure count", () => {
    expect(source).toContain('summary.status === "completed") return "Succ"');
    expect(source).toContain('return `Fail(${summary.failedCount})`');
  });

  it("keeps the command beside metadata and reserves the final column for the chevron", () => {
    expect(styles).toContain(".tool-activity-command { min-width: 0; grid-column: 4; justify-self: start;");
    expect(styles).toContain(".tool-activity-chevron { grid-column: 5; justify-self: end;");
  });
});
