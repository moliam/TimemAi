import { describe, expect, it } from "vitest";
import { extractMarkdownOutline, finalAnswerNeedsOutline, markdownHeadingSlug, markdownHeadingText, markdownOutlineActiveId, markdownOutlineAnimationPosition, markdownOutlineFitsBesideContent, MARKDOWN_OUTLINE_START_ID, markdownOutlineTargetScrollTop, MAX_OUTLINE_HEADINGS, MAX_OUTLINE_HEADING_SOURCE_CHARS, MAX_OUTLINE_LINE_CHARS, MAX_OUTLINE_SOURCE_CHARS, MAX_OUTLINE_TITLE_CHARS } from "../src/markdown_outline";

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

  it("requires at least two sections and more than one viewport page", () => {
    expect(finalAnswerNeedsOutline(601, 600, 2)).toBe(true);
    expect(finalAnswerNeedsOutline(600, 600, 2)).toBe(false);
    expect(finalAnswerNeedsOutline(1800, 600, 1)).toBe(false);
    expect(finalAnswerNeedsOutline(1800, 0, 3)).toBe(false);
  });
  it("fails closed for oversized answers, lines, and heading floods", () => {
    expect(extractMarkdownOutline(`# First\n${"x".repeat(MAX_OUTLINE_SOURCE_CHARS)}`)).toEqual([]);
    expect(extractMarkdownOutline(`# ${"x".repeat(MAX_OUTLINE_LINE_CHARS + 1)}\n## Safe`).map((item) => item.title)).toEqual(["Safe"]);
    const flooded = Array.from({ length: MAX_OUTLINE_HEADINGS + 1 }, (_, index) => `## Section ${index}`).join("\n");
    expect(extractMarkdownOutline(flooded)).toEqual([]);
  });

  it("bounds malformed inline markup and title/id size", () => {
    const malicious = `[${"[".repeat(MAX_OUTLINE_HEADING_SOURCE_CHARS)}](${"(".repeat(MAX_OUTLINE_HEADING_SOURCE_CHARS)})`;
    const title = markdownHeadingText(malicious);
    expect(title.length).toBeLessThanOrEqual(MAX_OUTLINE_TITLE_CHARS);
    expect(markdownHeadingSlug(title).length).toBeLessThanOrEqual(MAX_OUTLINE_TITLE_CHARS);
    const outline = extractMarkdownOutline(`# ${malicious}\n## Still responsive`);
    expect(outline.length).toBe(2);
    expect(outline[0].title.length).toBeLessThanOrEqual(MAX_OUTLINE_TITLE_CHARS);
  });

  it("handles unterminated fences and long non-heading input without throwing", () => {
    const malformed = `# Visible\n\`\`\`md\n${"# hidden\n".repeat(10_000)}`;
    expect(() => extractMarkdownOutline(malformed)).not.toThrow();
    expect(extractMarkdownOutline(malformed).map((item) => item.title)).toEqual(["Visible"]);
    const dense = `${"ordinary text\n".repeat(20_000)}## End`;
    expect(() => extractMarkdownOutline(dense)).not.toThrow();
    expect(extractMarkdownOutline(dense).at(-1)?.title).toBe("End");
  });


  it("keeps Start active before the first heading and advances in document order", () => {
    const items = extractMarkdownOutline("# First\n## Second\n### Third");
    const tops = new Map([[items[0].id, 120], [items[1].id, 260], [items[2].id, 410]]);
    expect(markdownOutlineActiveId(items, tops, 119)).toBe(MARKDOWN_OUTLINE_START_ID);
    expect(markdownOutlineActiveId(items, tops, 120)).toBe("first");
    expect(markdownOutlineActiveId(items, tops, 300)).toBe("second");
    expect(markdownOutlineActiveId(items, tops, 999)).toBe("third");
  });

  it("handles missing or invalid heading measurements without losing Start", () => {
    const items = extractMarkdownOutline("# First\n## Missing\n### Third");
    expect(markdownOutlineActiveId(items, new Map([["first", 80], ["third", 160]]), 200)).toBe("third");
    expect(markdownOutlineActiveId(items, new Map([["first", Number.NaN], ["third", 160]]), 120)).toBe(MARKDOWN_OUTLINE_START_ID);
    expect(markdownOutlineActiveId(items, new Map(), 200)).toBe(MARKDOWN_OUTLINE_START_ID);
    expect(markdownOutlineActiveId(items, new Map([["first", 80]]), Number.POSITIVE_INFINITY)).toBe(MARKDOWN_OUTLINE_START_ID);
  });

  it("requires the complete outline rail width and rejects invalid space measurements", () => {
    expect(markdownOutlineFitsBesideContent(260, 184, 60, 16)).toBe(true);
    expect(markdownOutlineFitsBesideContent(259.99, 184, 60, 16)).toBe(false);
    expect(markdownOutlineFitsBesideContent(184, 184, -20, -10)).toBe(true);
    expect(markdownOutlineFitsBesideContent(Number.NaN, 184, 60, 16)).toBe(false);
    expect(markdownOutlineFitsBesideContent(Number.POSITIVE_INFINITY, 184, 60, 16)).toBe(false);
  });

  it("computes Start and heading targets in viewport coordinates and clamps above-page targets", () => {
    expect(markdownOutlineTargetScrollTop(900, 300, 100, 24)).toBe(1076);
    expect(markdownOutlineTargetScrollTop(40, 20, 100, 24)).toBe(0);
    expect(markdownOutlineTargetScrollTop(120, 200, 100, -10)).toBe(220);
    expect(markdownOutlineTargetScrollTop(120, Number.NaN, 100, 24)).toBe(120);
    expect(markdownOutlineTargetScrollTop(Number.NaN, 200, 100, 24)).toBe(0);
  });

  it("animates forward and backward with bounded cubic easing", () => {
    expect(markdownOutlineAnimationPosition(100, 400, 0, 180)).toBe(100);
    expect(markdownOutlineAnimationPosition(100, 400, 180, 180)).toBe(400);
    expect(markdownOutlineAnimationPosition(400, 100, 180, 180)).toBe(100);
    const forwardMidpoint = markdownOutlineAnimationPosition(100, 400, 90, 180);
    const backwardMidpoint = markdownOutlineAnimationPosition(400, 100, 90, 180);
    expect(forwardMidpoint).toBeGreaterThan(250);
    expect(forwardMidpoint).toBeLessThan(400);
    expect(backwardMidpoint).toBeGreaterThan(100);
    expect(backwardMidpoint).toBeLessThan(250);
  });

  it("finishes immediately for reduced-motion-style durations and malformed animation metrics", () => {
    expect(markdownOutlineAnimationPosition(100, 400, 0, 0)).toBe(400);
    expect(markdownOutlineAnimationPosition(100, 400, 20, -1)).toBe(400);
    expect(markdownOutlineAnimationPosition(100, 400, 250, 180)).toBe(400);
    expect(markdownOutlineAnimationPosition(100, 400, Number.NaN, 180)).toBe(400);
    expect(markdownOutlineAnimationPosition(100, Number.NaN, 20, 180)).toBe(0);
  });

});
