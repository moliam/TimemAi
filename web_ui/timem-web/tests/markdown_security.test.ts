import { describe, expect, it } from "vitest";
import { safeMarkdownUrl } from "../src/markdown_security";

describe("markdown URL safety", () => {
  it("allows ordinary web, mail, root-relative and hash URLs", () => {
    expect(safeMarkdownUrl("https://example.com/a")).toBe("https://example.com/a");
    expect(safeMarkdownUrl("http://example.com/a")).toBe("http://example.com/a");
    expect(safeMarkdownUrl("mailto:user@example.com")).toBe("mailto:user@example.com");
    expect(safeMarkdownUrl("/local/path")).toBe("/local/path");
    expect(safeMarkdownUrl("#section")).toBe("#section");
  });

  it("rejects scriptable or browser-privileged markdown URLs", () => {
    expect(safeMarkdownUrl("javascript:alert(1)")).toBeUndefined();
    expect(safeMarkdownUrl(" data:text/html,<script>alert(1)</script>")).toBeUndefined();
    expect(safeMarkdownUrl("file:///etc/passwd")).toBeUndefined();
    expect(safeMarkdownUrl("blob:https://example.com/id")).toBeUndefined();
    expect(safeMarkdownUrl("")).toBeUndefined();
  });
});
