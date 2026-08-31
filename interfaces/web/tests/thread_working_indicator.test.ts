import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const styles = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8");
const source = readFileSync(new URL("../src/main.tsx", import.meta.url), "utf8");

function rule(selector: string) {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const match = styles.match(new RegExp(`${escaped}\\s*\\{([^}]*)\\}`));
  expect(match, `missing CSS rule: ${selector}`).not.toBeNull();
  return match?.[1] ?? "";
}

describe("thread working indicator", () => {
  it("removes every solid or shadow-drawn working button border", () => {
    const button = rule(".thread-working-away");
    const active = rule(".thread-working-away.is-working");
    const lightActive = rule(':root[data-theme="light"] .thread-working-away.is-working');

    expect(button).toContain("border-radius: 50%");
    expect(button).toContain("box-shadow: 0 1px 2px");
    expect(active).toContain("border: 0");
    expect(active).toContain("box-shadow: 0 1px 3px");
    expect(active).not.toContain("0 0 0 1px");
    expect(lightActive).toContain("border: 0");
    expect(lightActive).toContain("box-shadow: 0 1px 3px");
    expect(lightActive).not.toContain("0 0 0 1px");
    expect(styles).not.toContain("has-working-session");
    expect(source).not.toContain("has-working-session");
    expect(styles).not.toContain("thread-working-orbit");
    expect(source).not.toContain("thread-working-orbit");
  });

  it("uses a thicker rotating arc with rounded ends", () => {
    const arc = rule(".thread-working-arc");
    const stroke = rule(".thread-working-arc circle");

    expect(source).toContain('<circle cx="12" cy="12" r="9" pathLength="100" />');
    expect(arc).toContain("animation: thread-working-spin 1.8s linear infinite");
    expect(stroke).toContain("stroke-width: 3");
    expect(stroke).toContain("stroke-linecap: round");
    expect(stroke).toContain("stroke-dasharray: 18 82");
    expect(stroke).toContain("fill: none");
  });

  it("keeps reduced-motion users free from the arc animation", () => {
    expect(styles).toMatch(
      /@media \(prefers-reduced-motion: reduce\)[\s\S]*\.thread-working-away\.is-working \.thread-working-arc \{ animation: none; \}/,
    );
  });
});
