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
    expect(body).toContain("margin-top: 12px;");
  });

  it("separates adjacent edit blocks across run groups and archived details", () => {
    const runBody = ruleBody(
      ".stream-tool-run:has(.file-edits-panel) + .stream-tool-run:has(.file-edits-panel)",
    );
    expect(runBody).toContain("margin-top: 12px;");
    const detailsBody = ruleBody(
      ".tool-activity:has(.file-edits-panel) + .tool-activity:has(.file-edits-panel)",
    );
    expect(detailsBody).toContain("margin-top: 12px;");
  });

  it("renders each edit block as a warm bordered card", () => {
    const body = ruleBody(".stream-tool-row:has(.file-edits-panel)");
    expect(body).toContain("border: 1px solid rgba(255, 201, 120, 0.14);");
    expect(body).toContain("background: rgba(255, 213, 140, 0.05);");
    const light = ruleBody(':root[data-theme="light"] .stream-tool-row:has(.file-edits-panel)');
    expect(light).toContain("border-color: rgba(200, 158, 80, 0.30);");
    expect(light).toContain("background: #f9f4e8;");
    expect(light).toContain("box-shadow: 0 1px 2px rgba(124, 94, 38, 0.07);");
  });

  it("renders the command line as a banner above the diff rows", () => {
    const head = ruleBody(".stream-tool-row:has(.file-edits-panel) > .stream-tool-head");
    expect(head).toContain("background: rgba(255, 213, 140, 0.06);");
    expect(head).toContain("border-bottom: 1px solid rgba(255, 201, 120, 0.13);");
    expect(head).toContain("border-radius: 9px 9px 0 0;");
    const light = ruleBody(
      ':root[data-theme="light"] .stream-tool-row:has(.file-edits-panel) > .stream-tool-head',
    );
    expect(light).toContain("background: rgba(200, 158, 80, 0.11);");
  });

  it("gives the diff body a faint warm inset layer without any border line", () => {
    const body = ruleBody(".file-edit-diff");
    expect(body).toContain("padding: 2px 8px;");
    expect(body).not.toContain("border:");
    expect(body).not.toContain("border-left");
    expect(body).toContain("background: rgba(255, 213, 140, 0.04);");
    expect(body).toContain("border-radius: 8px;");
    const light = ruleBody(':root[data-theme="light"] .file-edit-diff');
    expect(light).toContain("background: rgba(253, 246, 230, 0.6);");
  });

  it("keeps the archived details command line on the same banner contract", () => {
    const summary = ruleBody(".tool-activity:has(.file-edits-panel) > summary");
    expect(summary).toContain("background: rgba(255, 213, 140, 0.06);");
    expect(summary).toContain("border-bottom: 1px solid rgba(255, 201, 120, 0.13);");
    expect(summary).toContain("border-radius: 7px 7px 0 0;");
    const light = ruleBody(':root[data-theme="light"] .tool-activity:has(.file-edits-panel) > summary');
    expect(light).toContain("background: rgba(200, 158, 80, 0.11);");
  });
});
