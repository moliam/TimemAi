import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { MarkdownContent } from "../src/markdown_render";
import { extractMarkdownOutline } from "../src/markdown_outline";

function render(markdown: string, headingIdPrefix?: string) {
  return renderToStaticMarkup(createElement(MarkdownContent, { text: markdown, headingIdPrefix }));
}

describe("MarkdownContent rendered output", () => {
  it("renders core Markdown structures and escaping", () => {
    const html = render("# Heading\n\n**bold** and *italic* and `x < y`\n\n> quote\n\n- one\n- two\n\n---");
    expect(html).toContain("<h1>Heading</h1>");
    expect(html).toContain("<strong>bold</strong>");
    expect(html).toContain("<em>italic</em>");
    expect(html).toContain("<code>x &lt; y</code>");
    expect(html).toContain("<blockquote>");
    expect(html).toContain("<ul>");
    expect(html).toContain("<hr/>");
  });

  it("renders GFM tables, task lists, deletion and autolinks accessibly", () => {
    const html = render("| A | B |\n|---|---|\n| 1 | 2 |\n\n- [x] done\n\n~~old~~\n\nhttps://example.com/path");
    expect(html).toContain('class="table-scroll"');
    expect(html).toContain('role="region"');
    expect(html).toContain('tabindex="0"');
    expect(html).toContain('aria-label="Scrollable table. Use horizontal scroll to inspect all columns."');
    expect(html).toContain("<table>");
    expect(html).toMatch(/<input type="checkbox"[^>]*disabled=""[^>]*checked=""/);
    expect(html).toContain("<del>old</del>");
    expect(html).toContain('href="https://example.com/path"');
    expect(html).toContain('target="_blank" rel="noopener noreferrer"');
  });

  it("wraps fenced code with language and copy affordances", () => {
    const highlighted = render("```js\nconst value = '<tag>';\n```");
    expect(highlighted).toContain('<figure class="code-block">');
    expect(highlighted).toContain('<span title="js">js</span>');
    expect(highlighted).toContain('aria-label="Copy js code"');
    expect(highlighted).toContain('class="hljs-keyword"');
    expect(highlighted).toContain('class="hljs-string"');
    expect(highlighted).toContain("&lt;tag&gt;");

    const c = render("```c\n#include <stdio.h>\nint main(void) { const char *message = \"Hello\"; printf(\"%s\\n\", message); return 0; }\n```");
    expect(c).toContain('<span title="c">c</span>');
    expect(c).toContain('class="hljs-meta"');
    expect(c).toContain('class="hljs-type"');
    expect(c).toContain('class="hljs-title function_"');
    expect(c).toContain('class="hljs-built_in"');
    expect(c).toContain('class="hljs-number"');

    const javascript = render("```javascript\nfunction greet(name) { const message = `Hello, ${name}`; console.log(message); return true; }\n```");
    expect(javascript).toContain('<span title="javascript">javascript</span>');
    expect(javascript).toContain('class="hljs-keyword"');
    expect(javascript).toContain('class="hljs-title function_"');
    expect(javascript).toContain('class="hljs-string"');
    expect(javascript).toContain('class="hljs-subst"');
    expect(javascript).toContain('class="hljs-literal"');

    const upperC = render("```C\nint main(void) { return 0; }\n```");
    expect(upperC).toContain('class="hljs-type"');
    expect(upperC).toContain('class="hljs-number"');

    const titledJavascript = render("```JavaScript\nconst enabled = true;\n```");
    expect(titledJavascript).toContain('class="hljs-keyword"');
    expect(titledJavascript).toContain('class="hljs-literal"');

    const plain = render("```\nraw & <safe>\n```");
    expect(plain).toContain('<span title="text">text</span>');
    expect(plain).toContain('aria-label="Copy text code"');
    expect(plain).toContain("raw &amp; &lt;safe&gt;");
  });

  it("renders inline and display math while tolerating malformed formulas", () => {
    const html = render("Inline $x^2$ and display:\n\n$$\n\\frac{1}{2}\n$$\n\n\\(a+b\\)\n\n\\[c=d\\]");
    expect(html).toContain('class="katex"');
    expect(html).toContain('class="katex-display"');
    expect(() => render("Malformed $\\frac{ and text")).not.toThrow();
    expect(render("Malformed $\\frac{ and text")).toContain("Malformed");
  });

  it("keeps multiline boxed math isolated from following Markdown tables", () => {
    const markdown = `Before:

\\[
\\boxed{
C_{\\text{avg}}
=
\\frac{2P}{MFU}
}
\\]

Inline \\(2P\\) and \\(P_{\\text{decode,active}}\\).

| 参数 | 含义 |
|---|---|
| \\(P_{\\text{decode,active}}\\) | 每轮 Decode 激活参数量 |
| \\(T_{\\text{budget}}\\) | 应小于 100 ms |`;
    const html = render(markdown);
    expect(html).toContain('class="katex-display"');
    expect(html).not.toContain('class="katex-error"');
    expect(html).toContain("<table>");
    expect(html).toContain("每轮 Decode 激活参数量");
    expect(html).toContain("应小于 100 ms");
    expect((html.match(/class="katex"/g) ?? [])).toHaveLength(5);
  });

  it("enforces separate link and image URL policies", () => {
    const links = render("[web](https://example.com) [mail](mailto:a@example.com) [local](/docs) [hash](#part) [js](javascript:alert(1)) [data](data:text/html,x) [file](file:///tmp/x) [blob](blob:https://example.com/id)");
    expect(links).toContain('href="https://example.com"');
    expect(links).toContain('href="mailto:a@example.com"');
    expect(links).toContain('href="/docs"');
    expect(links).toContain('href="#part"');
    expect(links).not.toContain("javascript:");
    expect(links).not.toContain("data:text");
    expect(links).not.toContain("file://");
    expect(links).not.toContain("blob:");
    expect(links).toContain("<span>js</span>");

    const images = render("![web](https://example.com/a.png) ![local](/a.png) ![mail](mailto:a@example.com) ![hash](#part) ![js](javascript:alert(1)) ![data](data:image/svg+xml,x)");
    expect(images).toContain('src="https://example.com/a.png"');
    expect(images).toContain('src="/a.png"');
    expect(images).not.toContain('alt="mail"');
    expect(images).not.toContain('alt="hash"');
    expect(images).not.toContain('alt="js"');
    expect(images).not.toContain('alt="data"');
  });

  it("keeps raw HTML inert", () => {
    const html = render('<script>alert(1)</script>\n\n<details open><summary>x</summary>secret</details>');
    expect(html).not.toContain("<script>");
    expect(html).not.toContain("<details");
    expect(html).toContain("&lt;script&gt;");
    expect(html).toContain("&lt;details open&gt;");
  });

  it("keeps rendered heading IDs aligned with the extracted outline", () => {
    const markdown = "Setext ignored by outline\n---\n\n# Rich *title* with [link](https://example.com), `code`, and ![diagram](https://example.com/a.png)\n\n## Duplicate\n\n## Duplicate\n\n### Formula $x^2$";
    const outline = extractMarkdownOutline(markdown);
    const html = render(markdown, "answer-1");
    expect(outline).toHaveLength(4);
    for (const item of outline) expect(html).toContain(`id="answer-1-${item.id}"`);
  });

  it("distinguishes soft and hard line breaks", () => {
    expect(render("first\nsecond")).toContain("<p>first\nsecond</p>");
    expect(render("first  \nsecond")).toContain("<p>first<br/>\nsecond</p>");
  });

  it("handles empty, large and malformed input without throwing", () => {
    expect(render("")).toBe('<div class="markdown-body"></div>');
    expect(() => render("a".repeat(200_000))).not.toThrow();
    expect(() => render("[broken](\n```unterminated\n<not-html")).not.toThrow();
  });
});
