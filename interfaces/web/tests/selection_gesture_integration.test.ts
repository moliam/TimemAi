import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

// Structural guard: selection gestures on diff/command/output text must not
// flip the tool row / tool activity expansion state. The behavior itself is
// unit-tested in selection_gesture.test.ts; this file pins the main.tsx
// wiring so the guard cannot be silently unwired.
const source = readFileSync(new URL("../src/main.tsx", import.meta.url), "utf8");

describe("selection gesture guard wiring", () => {
  it("guards both tool row and tool activity toggles through the shared hook", () => {
    expect(source).toMatch(/import \{ insideMergeableStreamRow, useSelectionGestureGuard \} from "\.\/selection_gesture";/);
    expect((source.match(/useSelectionGestureGuard\(\)/g) ?? []).length).toBe(2);
    expect(source).toMatch(
      /\{ onPointerDown: trackSelectionGesture, ignoreToggle \} = useSelectionGestureGuard\(\);/,
    );
  });

  it("drops drag-selection clicks before toggling the stream tool row", () => {
    expect(source).toMatch(
      /const toggle = \(event\?: \{ clientX: number; clientY: number \}\) => \{\s*if \(event && ignoreToggle\(event\)\) return;/,
    );
    expect(source).toMatch(/onPointerDown=\{hasExpandableDetail \? trackSelectionGesture : undefined\}/);
    expect(source).toMatch(/onClick=\{hasExpandableDetail \? event => toggle\(event\) : undefined\}/);
  });

  it("keeps selections in visible diff rows from expanding a folded tool group", () => {
    expect(source).toMatch(/stream-tool-mergeable/);
    expect(source).toMatch(/insideMergeableStreamRow\(anchorElement\)/);
    expect(source).toMatch(/insideMergeableStreamRow\(focused\)/);
  });

  it("swallows the click so details does not flip natively while selecting", () => {
    expect(source).toMatch(/onPointerDown=\{trackSelectionGesture\}/);
    expect(source).toMatch(/if \(ignoreToggle\(event\)\) \{\s*event\.preventDefault\(\);\s*return;/);
  });
});
