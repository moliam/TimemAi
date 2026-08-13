import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const styles = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8");

function blockAfter(marker: string, from = 0): string {
  const start = styles.indexOf(marker, from);
  expect(start).toBeGreaterThanOrEqual(0);
  const open = styles.indexOf("{", start);
  const close = styles.indexOf("}", open);
  return styles.slice(open + 1, close);
}

function color(block: string, variable: string): string {
  const match = block.match(new RegExp(`${variable}:\\s*(#[0-9a-fA-F]{6})`));
  expect(match, `${variable} must use an explicit six-digit color`).not.toBeNull();
  return match![1];
}

function luminance(hex: string): number {
  const channels = hex.slice(1).match(/.{2}/g)!.map((part) => {
    const value = Number.parseInt(part, 16) / 255;
    return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * channels[0] + 0.7152 * channels[1] + 0.0722 * channels[2];
}

function contrast(foreground: string, background: string): number {
  const [lighter, darker] = [luminance(foreground), luminance(background)].sort((a, b) => b - a);
  return (lighter + 0.05) / (darker + 0.05);
}

describe("formatted Markdown contrast", () => {
  const paletteStart = styles.indexOf("/* Accessible semantic palette for formatted Markdown output.");
  const dark = blockAfter(":root {", paletteStart);
  const light = blockAfter(':root[data-theme="light"] {', paletteStart);

  it.each([
    ["dark inline code", dark, "--markdown-inline-code-fg", "--markdown-inline-code-bg"],
    ["dark code text", dark, "--markdown-code-fg", "--markdown-code-bg"],
    ["dark comments", dark, "--markdown-code-muted", "--markdown-code-bg"],
    ["dark keywords", dark, "--markdown-code-keyword", "--markdown-code-bg"],
    ["dark strings", dark, "--markdown-code-string", "--markdown-code-bg"],
    ["dark numbers", dark, "--markdown-code-number", "--markdown-code-bg"],
    ["dark titles", dark, "--markdown-code-title", "--markdown-code-bg"],
    ["dark metadata", dark, "--markdown-code-meta", "--markdown-code-bg"],
    ["dark parameters", dark, "--markdown-code-params", "--markdown-code-bg"],
    ["dark deletions", dark, "--markdown-code-deletion", "--markdown-code-bg"],
    ["light inline code", light, "--markdown-inline-code-fg", "--markdown-inline-code-bg"],
    ["light code text", light, "--markdown-code-fg", "--markdown-code-bg"],
    ["light comments", light, "--markdown-code-muted", "--markdown-code-bg"],
    ["light keywords", light, "--markdown-code-keyword", "--markdown-code-bg"],
    ["light strings", light, "--markdown-code-string", "--markdown-code-bg"],
    ["light numbers", light, "--markdown-code-number", "--markdown-code-bg"],
    ["light titles", light, "--markdown-code-title", "--markdown-code-bg"],
    ["light metadata", light, "--markdown-code-meta", "--markdown-code-bg"],
    ["light parameters", light, "--markdown-code-params", "--markdown-code-bg"],
    ["light deletions", light, "--markdown-code-deletion", "--markdown-code-bg"],
  ])("%s meets WCAG AA for normal text", (_name, theme, foreground, background) => {
    expect(contrast(color(theme, foreground), color(theme, background))).toBeGreaterThanOrEqual(4.5);
  });

  it("scopes inline-code color separately from fenced code", () => {
    expect(styles).toContain(".markdown-body :not(pre) > code");
    expect(styles).toContain("color: var(--markdown-inline-code-fg);");
    expect(styles).toContain("border: 0;");
    expect(styles).toContain("padding: .16em .38em;");
    expect(styles).toContain("font-family: var(--ui-font);");
    expect(styles).toContain("font-size: .9em;");
    expect(styles).toContain("font-weight: 400;");
    expect(styles).toContain("letter-spacing: -.01em;");
    expect(styles).toContain(".code-block .hljs-comment");
    expect(styles).toContain(".code-block .hljs-keyword");
  });

  it("overrides the legacy light transcript color for plain text fences", () => {
    expect(styles).toContain(':root[data-theme="light"] .message-content .code-block pre code');
    expect(styles).toContain(':root[data-theme="light"] .turn-final-delivery > .message-content .code-block pre code');
    expect(styles).toContain('-webkit-text-fill-color: var(--markdown-code-fg);');
  });
});
