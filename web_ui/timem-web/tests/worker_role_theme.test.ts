import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";

const source = readFileSync(new URL("../src/main.tsx", import.meta.url), "utf8");
const styles = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8");

function rule(selector: string): string {
  const start = styles.indexOf(`${selector} {`);
  expect(start, `missing CSS rule: ${selector}`).toBeGreaterThanOrEqual(0);
  const bodyStart = styles.indexOf("{", start) + 1;
  const bodyEnd = styles.indexOf("}", bodyStart);
  expect(bodyEnd, `unterminated CSS rule: ${selector}`).toBeGreaterThan(bodyStart);
  return styles.slice(bodyStart, bodyEnd);
}

describe("worker role action theme", () => {
  it("applies the themed edit and delete action classes to group controls", () => {
    const groupHeader = source.match(
      /<div><button type="button" className="worker-role-action worker-role-edit"[^>]*title=\{`Rename \$\{group\.name\}`\}[\s\S]*?<button type="button" className="worker-role-action worker-role-delete"[^>]*title=\{`Delete \$\{group\.name\}`\}/,
    );

    expect(groupHeader).not.toBeNull();
  });

  it("uses a light surface and dark foreground for actions in the light theme", () => {
    const action = rule(':root[data-theme="light"] .worker-role-panel .worker-role-action');

    expect(action).toContain("border-color: #d5dde0");
    expect(action).toContain("background: #f6f8f9");
    expect(action).toContain("color: #68777e");
    expect(action).not.toMatch(/background:\s*#(?:171717|202020|222|2a3439|392929)/i);
  });

  it("keeps edit and delete hover states compatible with the light palette", () => {
    const edit = rule(':root[data-theme="light"] .worker-role-panel .worker-role-edit:hover:not(:disabled)');
    const remove = rule(':root[data-theme="light"] .worker-role-panel .worker-role-delete:hover:not(:disabled)');

    expect(edit).toContain("background: #eaf0f2");
    expect(edit).toContain("color: #344c56");
    expect(remove).toContain("background: #f8eae9");
    expect(remove).toContain("color: #9e4c47");
  });
});
