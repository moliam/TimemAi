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
      /<div>\s*<button[\s\S]*?className="worker-role-action worker-role-edit"[\s\S]*?title=\{`Rename \${group\.name}`\}[\s\S]*?<button[\s\S]*?className="worker-role-action worker-role-delete"[\s\S]*?title=\{`Delete \${group\.name}`\}/,
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

  it("uses the role accent palette for group and role creation in both themes", () => {
    expect(source).toContain('className="worker-role-group-create"');
    expect(source).toContain('className="worker-role-primary-action"');

    const dark = rule(".worker-role-panel :is(.worker-role-group-create, .worker-role-primary-action)");
    const light = rule(':root[data-theme="light"] .worker-role-panel :is(.worker-role-group-create, .worker-role-primary-action)');
    const lightHover = rule(':root[data-theme="light"] .worker-role-panel :is(.worker-role-group-create, .worker-role-primary-action):hover:not(:disabled)');

    expect(dark).toContain("border-color: #487568");
    expect(dark).toContain("background: #23483f");
    expect(dark).toContain("color: #c7e8df");
    expect(light).toContain("border-color: #79aa9d");
    expect(light).toContain("background: #dcefeb");
    expect(light).toContain("color: #245f55");
    expect(lightHover).toContain("background: #cce6df");
    expect(light).not.toMatch(/background:\s*#(?:171717|202020|222)/i);
  });
  it("uses a compact, readable type scale inside the narrow roles panel", () => {
    const panel = rule(".worker-role-panel");
    const title = rule(".worker-role-panel > header > span");
    const input = rule(".worker-role-editor input");
    const action = rule(".worker-role-editor > div button");

    expect(panel).toContain("--worker-role-title-size: 13px");
    expect(panel).toContain("--worker-role-control-size: 12px");
    expect(panel).toContain("--worker-role-meta-size: 10.5px");
    expect(title).toContain("font-size: var(--worker-role-title-size)");
    expect(input).toContain("font-size: var(--worker-role-control-size)");
    expect(styles).toContain(".worker-role-editor textarea { min-height: 112px; resize: vertical; padding: 9px; font-size: var(--worker-role-control-size)");
    expect(action).toContain("font-size: var(--worker-role-control-size)");
    expect(input).not.toContain("font-size: var(--content-size)");
  });

});
