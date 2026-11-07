import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const source = readFileSync(new URL("../src/main.tsx", import.meta.url), "utf8");

describe("thread latest-content following", () => {
  it("refreshes portaled answer outline geometry when the enclosing turn shrinks", () => {
    expect(source).toContain('const turnContainer = root.closest(".turn-interaction")');
    expect(source).toContain("if (turnContainer) observer?.observe(turnContainer)");
  });

  it("does not let empty-session restoration suppress the first turn scroll", () => {
    expect(source).toMatch(
      /restoredSessionIdRef\.current = latestTurn\?\.turn_id\s*\? activeSessionId\s*:\s*undefined/,
    );
  });

  it("invalidates portaled outline geometry when restored turns hydrate or prepend", () => {
    // Sibling-turn hydration moves outlines without resizing any observed
    // node; the thread must broadcast the shared event on those commits.
    expect(source).toContain("const timelineOutlineShape = timelineOutlineShapeKey(turns);");
    expect(source).toContain('window.dispatchEvent(new Event("markdown-outline-layout-change"));');
    expect(source).toContain("}, [timelineOutlineShape]);");
    expect(source).toContain('window.addEventListener("markdown-outline-layout-change", handleSharedLayoutChange);');
    expect(source).toContain('window.removeEventListener("markdown-outline-layout-change", handleSharedLayoutChange);');
  });

  it("remeasures the portaled outline synchronously on shared layout changes", () => {
    // The broadcast fires from layout effects before the committing paint; a
    // rAF-deferred remeasure would paint one stale outline top per commit,
    // and the connect-time hydration storm shows that backlog as flicker.
    expect(source).toMatch(/const handleSharedLayoutChange = \(\) => \{\s*update\(\);\s*outlineGeometryTaskRef\.current\?\.request\(\);\s*\};/);
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
