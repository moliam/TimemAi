import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const source = readFileSync(new URL("../src/main.tsx", import.meta.url), "utf8");

describe("thread latest-content following", () => {
  it("does not let empty-session restoration suppress the first turn scroll", () => {
    expect(source).toMatch(
      /restoredSessionIdRef\.current = latestTurn\?\.turn_id\s*\? activeSessionId\s*:\s*undefined/,
    );
  });

  it("aligns a new turn again after the welcome-to-working layout frame", () => {
    expect(source).toMatch(
      /followThreadLatest\.current = true;\s*viewport\.scrollTop = viewport\.scrollHeight;\s*const frame = window\.requestAnimationFrame/,
    );
    expect(source).toMatch(
      /if \(!followThreadLatest\.current \|\| previousScrollMetrics\.current\) return;\s*viewport\.scrollTop = viewport\.scrollHeight/,
    );
    expect(source).toContain("window.cancelAnimationFrame(frame)");
  });
});
