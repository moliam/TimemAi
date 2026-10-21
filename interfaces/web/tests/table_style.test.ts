import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const styles = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8");

describe("Markdown table styling", () => {
  it("gives every markdown table inner cell lines, not only final-answer tables", () => {
    // Inner lines must not depend on a .message-content ancestor: process
    // area tables (stream thoughts, tool details) share .table-scroll.
    const cellRule = styles.match(
      /\.markdown-body \.table-scroll th, \.markdown-body \.table-scroll td \{[^}]*border:/,
    );
    expect(cellRule).not.toBeNull();
    expect(styles).toMatch(
      /:root\[data-theme="light"\] \.markdown-body \.table-scroll th[^{]*\{[^}]*border-color:/,
    );
  });

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
