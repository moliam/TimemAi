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

  it("wraps long cell content first and only scrolls when columns cannot fit", () => {
    // `width: max-content` + `table-layout: fixed` made long unbreakable cell
    // content drive an extremely wide horizontal scrollbar. Tables must size
    // to the container and wrap; scrolling stays for genuinely wide tables.
    const tableRule = styles.match(/\.markdown-body table \{[^}]*\}/)![0];
    expect(tableRule).not.toMatch(/width:\s*max-content/);
    expect(tableRule).toMatch(/width:\s*100%/);
    expect(tableRule).not.toMatch(/table-layout:\s*fixed/);
    const cellRule = styles.match(
      /\.markdown-body \.table-scroll th, \.markdown-body \.table-scroll td \{[^}]*\}/,
    )![0];
    expect(cellRule).toMatch(/min-width:\s*6em/);
    expect(cellRule).toMatch(/overflow-wrap:\s*anywhere/);
    // Scroll affordance stays visible but visually quiet.
    expect(styles).toMatch(/\.table-scroll \{[^}]*scrollbar-width:\s*thin/);
    expect(styles).toMatch(/\.table-scroll::-webkit-scrollbar-thumb \{[^}]*border-radius:/);
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
