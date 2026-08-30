import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const styles = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8");

function rule(selector: string) {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const match = styles.match(new RegExp(`${escaped}\\s*\\{([^}]*)\\}`));
  expect(match, `missing CSS rule: ${selector}`).not.toBeNull();
  return match?.[1] ?? "";
}

describe("thread working indicator", () => {
  it("uses a rounded blue orbit with a restrained thin rotating arc", () => {
    expect(rule(".thread-working-orbit")).toMatch(
      /width: 18px;[\s\S]*height: 18px;[\s\S]*border-radius: 50%/,
    );

    const activeArc = rule(".thread-working-away.is-working .thread-working-orbit::before");
    expect(activeArc).toMatch(
      /calc\(100% - 2px\)[\s\S]*calc\(100% - 1\.5px\)/,
    );
    expect(activeArc).toContain("filter: none");
    expect(activeArc).toContain("animation: thread-working-orbit 1.25s linear infinite");
    expect(styles).not.toContain("thread-working-breathe");
  });

  it("keeps reduced-motion users free from the orbit animation", () => {
    expect(styles).toMatch(
      /@media \(prefers-reduced-motion: reduce\)[\s\S]*\.thread-working-away\.is-working \.thread-working-orbit::before,[\s\S]*animation: none/,
    );
  });
});
