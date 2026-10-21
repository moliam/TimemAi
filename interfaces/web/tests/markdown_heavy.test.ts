import { describe, expect, it } from "vitest";
import { markdownNeedsHeavyEnhancement } from "../src/markdown_heavy";

describe("markdown heavy enhancement content gate", () => {
  it("keeps plain prose and styled markdown off the heavy chunk", () => {
    expect(markdownNeedsHeavyEnhancement("Plain prose with emphasis and lists.")).toBe(false);
    expect(markdownNeedsHeavyEnhancement("# Heading\n\n**bold** *italic* ~~gone~~\n\n- one\n- two")).toBe(false);
    expect(markdownNeedsHeavyEnhancement("| A | B |\n|---|---|\n| 1 | 2 |")).toBe(false);
    expect(markdownNeedsHeavyEnhancement("")).toBe(false);
  });

  it("stays off for escaped currency after normalization", () => {
    // normalizeMarkdownMath escapes bare currency dollars as \$.
    expect(markdownNeedsHeavyEnhancement("Costs \\$5 today and \\$19.99 tomorrow.")).toBe(false);
  });

  it("requests the chunk for code fences and real math", () => {
    expect(markdownNeedsHeavyEnhancement("Example:\n```js\nconst x = 1;\n```")).toBe(true);
    expect(markdownNeedsHeavyEnhancement("Indented fence:\n~~~\nraw\n~~~")).toBe(true);
    expect(markdownNeedsHeavyEnhancement("Inline $x^2$ here")).toBe(true);
    expect(markdownNeedsHeavyEnhancement("$$\\frac{1}{2}$$")).toBe(true);
    // The gate receives normalized text; bracket/model delimiters have already
    // become $/$$ at that point, but keep them covered for direct callers.
    expect(markdownNeedsHeavyEnhancement("\\(a+b\\)")).toBe(true);
    expect(markdownNeedsHeavyEnhancement("[/math]E=mc^2[/math]")).toBe(true);
  });
});
