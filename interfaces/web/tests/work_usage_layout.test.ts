import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";

const source = readFileSync(new URL("../src/main.tsx", import.meta.url), "utf8");
const styles = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8");

describe("work stream token usage layout", () => {
  it("renders live usage outside the scrolling work content", () => {
    expect(source).toMatch(
      /<div className="turn-work-content" ref=\{workContentRef\}>[\s\S]*?<\/div>\s*<\/div>\s*\{hasLiveUsage && <LiveTurnUsage turn=\{turn\} \/>\}/,
    );
  });

  it("only enables the fixed footer layout when usage is available", () => {
    expect(source).toContain(
      'const hasLiveUsage = isWorking && turnLiveUsage(turn) !== undefined;',
    );
    expect(source).toContain(
      'className={`turn-work-panel${hasLiveUsage ? " has-live-usage" : ""}`}',
    );
  });

  it("keeps the update button clear of the fixed usage footer", () => {
    expect(styles).toMatch(
      /\.turn-work-panel\.has-live-usage > \.turn-new-updates \{ bottom: 42px; \}/,
    );
  });
});
