import { describe, expect, it } from "vitest";
import { extractMarkdownOutline, finalAnswerNeedsOutline, markdownHeadingSlug, markdownHeadingText } from "../src/markdown_outline";

describe("final answer markdown outline", () => {
  it("extracts level one through three ATX headings in document order", () => {
    expect(extractMarkdownOutline("# Overview\ntext\n## Details\n### Edge cases\n#### Hidden depth")).toEqual([
      { id: "overview", level: 1, title: "Overview" },
      { id: "details", level: 2, title: "Details" },
      { id: "edge-cases", level: 3, title: "Edge cases" },
    ]);
  });

  it("ignores heading-shaped lines inside backtick and tilde fences", () => {
    const markdown = "# Real\n```md\n## Not real\n```\n~~~\n### Also hidden\n~~~\n## Visible";
    expect(extractMarkdownOutline(markdown).map((item) => item.title)).toEqual(["Real", "Visible"]);
  });

  it("normalizes inline markdown and creates stable unique ids", () => {
    expect(markdownHeadingText("**[API guide](https://example.com)** and `CLI`")).toBe("API guide and CLI");
    expect(extractMarkdownOutline("## 结论\n## 结论\n## **[API guide](https://example.com)**").map((item) => item.id)).toEqual([
      "结论",
      "结论-2",
      "api-guide",
    ]);
    expect(markdownHeadingSlug("***")).toBe("section");
  });

  it("requires at least two sections and more than two viewport pages", () => {
    expect(finalAnswerNeedsOutline(1201, 600, 2)).toBe(true);
    expect(finalAnswerNeedsOutline(1200, 600, 2)).toBe(false);
    expect(finalAnswerNeedsOutline(1800, 600, 1)).toBe(false);
    expect(finalAnswerNeedsOutline(1800, 0, 3)).toBe(false);
  });
});
