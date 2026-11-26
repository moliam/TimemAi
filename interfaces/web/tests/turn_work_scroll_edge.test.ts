import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";

const styles = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8");

function ruleBody(selector: string) {
  const start = styles.indexOf(`${selector} {`);
  expect(start).toBeGreaterThanOrEqual(0);
  const end = styles.indexOf("}", start);
  expect(end).toBeGreaterThan(start);
  return styles.slice(start, end);
}

describe("thought/action scroll edge", () => {
  it("separates the expanded work stream from the answer below", () => {
    const body = ruleBody(".turn-work-panel:not(.has-live-usage)::after");
    expect(body).toContain("position: absolute;");
    expect(body).toContain("bottom: 0;");
    expect(body).toContain("height: 1px;");
    expect(body).toContain("background: rgba(255, 201, 120, 0.24);");
  });

  it("draws a left rail on the work scroll window", () => {
    const body = ruleBody(".turn-work-scroll");
    expect(body).toContain("border-left: 2px solid rgba(255, 201, 120, 0.20);");
    expect(body).toContain("padding: 7px 0 10px 10px;");
    const light = styles.indexOf(':root[data-theme="light"] .turn-work-scroll { border-left-color: rgba(200, 158, 80, 0.36); }');
    expect(light).toBeGreaterThanOrEqual(0);
  });
});
