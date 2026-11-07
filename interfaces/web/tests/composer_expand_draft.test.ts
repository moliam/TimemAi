import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const source = readFileSync(new URL("../src/main.tsx", import.meta.url), "utf8");

// Regression guards for the "expanded editor opened blank" race: the expand
// action, the live-text adoption effect, and the expanded editor teardown
// must each preserve user typing instead of losing it.

describe("composer expanded editor draft retention", () => {
  it("lands pending throttled typing before capturing the expand base", () => {
    expect(source).toMatch(
      /onExpand=\{\(\) => \{[\s\S]*?composerWriteThroughRef\.current\?\.flush\(\);[\s\S]*?composerExpandedBaseRef\.current = composerLiveTextRef\.current;[\s\S]*?setComposerExpanded\(true\);/,
    );
  });

  it("keeps the expanded editor authoritative over external draft updates", () => {
    expect(source).toContain("if (composerExpanded) return;");
    expect(source).toMatch(
      /if \(draft !== composerWriteThroughTextRef\.current\)\s*\n\s*composerLiveTextRef\.current = draft;\s*\n\s*\}, \[draft, composerExpanded\]\)/,
    );
  });

  it("commits unsaved expanded-editor typing on teardown instead of dropping it", () => {
    expect(source).toMatch(
      /Teardown with unsaved typing[\s\S]*?current\.onLiveCommit && current\.draft !== current\.value[\s\S]*?current\.onLiveCommit\(current\.draft\);/,
    );
  });
});
