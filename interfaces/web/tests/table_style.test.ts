import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const styles = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8");

describe("Markdown table styling", () => {
  it("uses backgrounds for table headers but not alternating body rows", () => {
    expect(styles).toMatch(/\.message-content th\s*\{[^}]*background:/);
    expect(styles).toMatch(
      /:root\[data-theme="light"\] \.message-content th\s*\{[^}]*background:/,
    );
    expect(styles).not.toMatch(
      /(?:tbody\s+)?tr\s*:\s*nth-child\([^)]*\)\s*\{[^}]*background\s*:/,
    );
  });
});
