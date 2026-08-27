import { describe, expect, it } from "vitest";
import { normalizeMarkdownMath } from "../src/markdown_math";

describe("assistant Markdown math normalization", () => {
  it("supports dollar, LaTeX bracket, and model-specific delimiters", () => {
    expect(normalizeMarkdownMath("Inline \\(x^2 + y^2\\).\n\n\\[\\sum_{i=1}^n i\\]\n\n[/inline]a/b[/inline] [/math]E=mc^2[/math]"))
      .toBe("Inline $x^2 + y^2$.\n\n\n\n$$\n\\sum_{i=1}^n i\n$$\n\n\n\n$a/b$ \n\n$$\nE=mc^2\n$$\n\n");
  });

  it("preserves inline and fenced code containing formula-like text", () => {
    const markdown = "Use `\\(literal\\)` here.\n```tex\n\\[not rendered\\]\n$also_literal$\n```\nReal: \\(x\\)";
    expect(normalizeMarkdownMath(markdown)).toBe("Use `\\(literal\\)` here.\n```tex\n\\[not rendered\\]\n$also_literal$\n```\nReal: $x$");
  });

  it("does not mistake currency for single-dollar math", () => {
    expect(normalizeMarkdownMath("Costs $5 today and $19.99 tomorrow; math is $x+1$."))
      .toBe("Costs \\$5 today and \\$19.99 tomorrow; math is $x+1$.");
  });

  it("leaves malformed delimiters readable instead of throwing", () => {
    expect(() => normalizeMarkdownMath("Broken \\(x and $$y")).not.toThrow();
    expect(normalizeMarkdownMath("Broken \\(x and $$y")).toBe("Broken \\(x and $$y");
  });
  it("keeps display delimiters as Markdown flow math blocks", () => {
    const normalized = normalizeMarkdownMath("Before\n\n\\[\n\\boxed{\nC_{\\text{avg}}=2P\n}\n\\]\n\nAfter");
    expect(normalized).toContain("\n\n$$\n\\boxed{\nC_{\\text{avg}}=2P\n}\n$$\n\n");
    expect(normalized).not.toContain("$$\\boxed{");
  });

  it("does not mistake number-leading math for currency", () => {
    expect(normalizeMarkdownMath("\\(2P\\), \\(100\\sim500\\), $2P$, but $5 today and $19.99 tomorrow."))
      .toBe("$2P$, $100\\sim500$, $2P$, but \\$5 today and \\$19.99 tomorrow.");
  });

});
