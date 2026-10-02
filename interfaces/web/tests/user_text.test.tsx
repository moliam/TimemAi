import { readFileSync } from "node:fs";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { UserText } from "../src/user_text";

const mainSource = readFileSync(new URL("../src/main.tsx", import.meta.url), "utf8");
const styles = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8");

describe("user-authored text presentation", () => {
  it("renders Markdown and HTML syntax literally", () => {
    const text = "# Heading\n\n**bold** [link](https://example.com) `code`\n<script>alert(1)</script>";
    const html = renderToStaticMarkup(createElement(UserText, { text }));

    expect(html).toContain("# Heading\n\n**bold** [link](https://example.com) `code`");
    expect(html).toContain("&lt;script&gt;alert(1)&lt;/script&gt;");
    expect(html).not.toMatch(/<(?:h1|strong|a|code|script)(?:\s|>)/);
  });

  it("uses plain text for sent messages and live user supplements only", () => {
    expect(mainSource).toContain("<UserText text={entry.text} />");
    expect(mainSource).toContain("activity.detail && <UserText text={activity.detail} />");
    expect(mainSource).not.toContain("<MarkdownContent text={entry.text} />");
    expect(mainSource).not.toContain("activity.detail && <MarkdownContent text={activity.detail} />");
    expect(mainSource).toContain("<MarkdownContent\n          text={text}");
  });

  it("preserves user whitespace without Markdown-specific bubble styles", () => {
    expect(styles).toContain(".user-plain-text { display: block; min-width: 0; white-space: pre-wrap;");
    expect(styles).toContain(".turn-user-entry > .user-plain-text { padding-right: 26px; }");
    expect(styles).not.toMatch(/turn-user-entry[^{}]*markdown-body/);
  });
});
