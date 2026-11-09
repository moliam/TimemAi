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

  it("wraps table content without creating a horizontal scroller", () => {
    const tableRule = styles.match(/\.markdown-body table \{[^}]*\}/)![0];
    expect(tableRule).toMatch(/width:\s*100%/);
    expect(tableRule).toMatch(/table-layout:\s*fixed/);

    const wrapperRule = styles.match(/\.table-scroll \{[^}]*\}/)![0];
    expect(wrapperRule).toMatch(/overflow:\s*hidden/);
    expect(wrapperRule).not.toMatch(/overflow-x:\s*auto/);

    const cellRule = styles.match(
      /\.markdown-body \.table-scroll th, \.markdown-body \.table-scroll td \{[^}]*\}/,
    )![0];
    expect(cellRule).toMatch(/min-width:\s*0/);
    expect(cellRule).toMatch(/word-break:\s*break-word/);
    expect(cellRule).toMatch(/overflow-wrap:\s*anywhere/);

    // The table wrapper is no longer a scroll region, so no scrollbar
    // affordance should remain.
    expect(styles).not.toMatch(/\.table-scroll[^}]*scrollbar-width/);
    expect(styles).not.toMatch(/\.table-scroll::\-webkit-scrollbar/);
  });

  it("wraps plain text fences but keeps language code scrollable", () => {
    const preRule = styles.match(/\.code-block pre,[\s\S]*?\{[^}]*\}/)![0];
    expect(preRule).toMatch(/overflow:\s*auto/);
    expect(styles).toMatch(/\.code-block code \{[^}]*min-width:\s*max-content/);

    const textPreRule = styles.match(/\.code-block-text pre,[\s\S]*?\{[^}]*\}/)![0];
    expect(textPreRule).toMatch(/overflow-x:\s*hidden/);
    expect(textPreRule).toMatch(/white-space:\s*pre-wrap/);
    expect(textPreRule).toMatch(/overflow-wrap:\s*anywhere/);

    const textCodeRule = styles.match(/\.code-block-text code,[\s\S]*?\{[^}]*\}/)![0];
    expect(textCodeRule).toMatch(/min-width:\s*0/);
    expect(textCodeRule).toMatch(/white-space:\s*inherit/);
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
