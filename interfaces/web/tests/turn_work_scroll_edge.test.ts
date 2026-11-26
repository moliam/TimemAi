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
    expect(body).toContain("rgba(255, 255, 255, 0.16)");
  });
});
