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

describe("file edit block spacing", () => {
  it("keeps consecutive command edit blocks visually separate", () => {
    const body = ruleBody(
      ".stream-tool-merged-item:has(.file-edits-panel) + .stream-tool-merged-item:has(.file-edits-panel)",
    );
    expect(body).toContain("margin-top: 8px;");
  });
});
