import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const styles = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8");

const paletteStart = styles.indexOf(
  "/* Accessible semantic palette for formatted Markdown output.",
);
expect(paletteStart).toBeGreaterThanOrEqual(0);

function blockAfter(marker: string, from = paletteStart): string {
  const start = styles.indexOf(marker, from);
  expect(start, `${marker} block missing`).toBeGreaterThanOrEqual(0);
  const open = styles.indexOf("{", start);
  const close = styles.indexOf("}", open);
  return styles.slice(open + 1, close);
}

const dark = blockAfter(":root {");
const light = blockAfter(':root[data-theme="light"] {');

function ruleBody(selector: string): string {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const match = styles.match(new RegExp(`(?:^|\\n)${escaped} \\{\\n([\\s\\S]*?)\\n\\}`));
  expect(match, `missing CSS rule for ${selector}`).not.toBeNull();
  return match![1];
}

function token(block: string, name: string): string {
  const match = block.match(new RegExp(`${name}:\\s*(#[0-9a-fA-F]{6,8})`));
  expect(match, `${name} must be an explicit hex color`).not.toBeNull();
  return match![1];
}

function rgb(hex: string): number[] {
  return hex
    .slice(1, 7)
    .match(/.{2}/g)!
    .map((part) => Number.parseInt(part, 16));
}

function luminance(hex: string): number {
  const channels = rgb(hex).map((channel) => {
    const value = channel / 255;
    return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * channels[0] + 0.7152 * channels[1] + 0.0722 * channels[2];
}

function contrast(foreground: string, background: string): number {
  const [lighter, darker] = [luminance(foreground), luminance(background)].sort((a, b) => b - a);
  return (lighter + 0.05) / (darker + 0.05);
}

// Model replies render on the chat shell surface, not on the page background;
// the browser acceptance measures the same surfaces from computed styles.
const REPLY_SURFACE: Record<string, string> = { dark: "#212121", light: "#ffffff" };

describe("link affordance styling", () => {
  it.each(["dark", "light"])(
    "%s theme keeps link ink legible on the assistant reply surface",
    (theme) => {
      const block = theme === "dark" ? dark : light;
      const surface = REPLY_SURFACE[theme];
      expect(contrast(token(block, "--link-ink"), surface)).toBeGreaterThanOrEqual(4.5);
      expect(contrast(token(block, "--link-hover-ink"), surface)).toBeGreaterThanOrEqual(4.5);
    },
  );

  it.each(["dark", "light"])("%s link ink is a saturated hue, not neutral text", (theme) => {
    const block = theme === "dark" ? dark : light;
    const [red, green, blue] = rgb(token(block, "--link-ink"));
    const spread = Math.max(red, green, blue) - Math.min(red, green, blue);
    // Neutral body ink has a channel spread near zero; a link must read as a
    // different color at a glance, not as slightly tinted text.
    expect(spread).toBeGreaterThanOrEqual(48);
    expect(green).toBeGreaterThan(red);
    expect(green).toBeGreaterThan(blue);
  });

  it.each(["dark", "light"])("%s underline stays visible without hovering", (theme) => {
    const block = theme === "dark" ? dark : light;
    const underline = token(block, "--link-underline");
    expect(underline).toHaveLength(9);
    expect(underline.slice(1, 7)).toBe(token(block, "--link-ink").slice(1));
    expect(Number.parseInt(underline.slice(7, 9), 16) / 255).toBeGreaterThanOrEqual(0.7);
    const wash = token(block, "--link-rest-wash");
    expect(wash).toHaveLength(9);
    expect(wash.slice(1, 7)).toBe(token(block, "--link-ink").slice(1));
  });

  it("marks every anchor with an always-on underline", () => {
    const body = ruleBody("a");
    expect(body).toContain("color: var(--link-ink);");
    expect(body).toContain("text-decoration: underline;");
    expect(body).toContain("text-decoration-thickness: 2px;");
    expect(body).toContain("text-decoration-color: var(--link-underline);");
    expect(body).toContain("text-underline-offset: 2px;");
  });

  it("tints reply prose so a link separates from body copy at rest", () => {
    const body = ruleBody(".markdown-body a");
    expect(body).toContain("background-color: var(--link-rest-wash);");
    // A wrapped URL must stay one continuous band instead of one box per line.
    expect(body).toContain("box-decoration-break: clone;");
    expect(body).toContain("-webkit-box-decoration-break: clone;");
    // Backgrounds never affect layout, so the tint cannot reflow reply text.
    expect(body).not.toContain("padding");
    expect(body).not.toContain("margin");
  });

  it("steps the link up again on hover", () => {
    const body = ruleBody("a:hover");
    expect(body).toContain("color: var(--link-hover-ink);");
    expect(body).toContain("text-decoration-color: currentColor;");
    expect(body).toContain("background-color: var(--link-hover-wash);");
  });

  it("draws a focus ring for keyboard users", () => {
    const body = ruleBody("a:focus-visible");
    expect(body).toContain("outline: 2px solid var(--link-ink);");
    expect(body).toContain("outline-offset: 2px;");
    expect(body).toContain("text-decoration-color: currentColor;");
  });

  it("marks outbound links and keeps the marker out of copied text", () => {
    expect(styles).toContain(
      ".markdown-body a.markdown-external-link:not(.markdown-address-link)::after {",
    );
    const body = ruleBody(".markdown-body a.markdown-external-link:not(.markdown-address-link)::after");
    expect(body).toContain('content: "\\2197";');
    expect(body).toContain("user-select: none;");
    // The arrow is an inline-block box, so the link underline does not run
    // through it and the marker reads as a marker.
    expect(body).toContain("display: inline-block;");
    expect(body).toContain("text-decoration: none;");
  });

  it("uses a dotted underline for destinations the interface handles itself", () => {
    const local = ruleBody(".markdown-body a.markdown-local-file-link");
    expect(local).toContain("text-decoration-style: dotted;");
    expect(local).toContain("cursor: pointer;");
    const hover = ruleBody(".markdown-body a.markdown-local-file-link:hover");
    expect(hover).toContain("text-decoration-style: solid;");
  });

  it("keeps a single source of truth for link ink", () => {
    // Legacy per-container colors must not come back and silently win over
    // the tokens: `.markdown-body a` and the About row used to hardcode them.
    expect(styles).not.toContain("#75cbbb");
    expect(styles).not.toContain(":root[data-theme=\"light\"] .markdown-body a {");
  });
});
