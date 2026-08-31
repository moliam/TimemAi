import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const source = readFileSync(new URL("../src/main.tsx", import.meta.url), "utf8");
const styles = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8");

describe("restart working-directory gate", () => {
  it("replaces the composer in the sticky input area until a choice is made", () => {
    expect(source).toMatch(
      /<ThreadPrimitive\.ViewportFooter className="composer-wrap aui-thread-footer">[\s\S]*{activeSession && restartCwdDecision \? \([\s\S]*className="restart-cwd-gate"[\s\S]*\) : \([\s\S]*<form[\s\S]*className="composer"/,
    );
    expect(source).not.toMatch(
      /{activeSession && restartCwdDecision && \([\s\S]*<TimemThread/,
    );
    expect(styles).toMatch(
      /\.restart-cwd-gate \{[\s\S]*width: 100%;[\s\S]*margin: 0;/,
    );
  });

  it("keeps the notice concise and makes only the two actions buttons", () => {
    expect(source).toContain("当前 Timem 的启动目录和 Session 上次工作的目录不同");
    expect(source).toMatch(/<button[\s\S]*>\s*切换\s*<\/button>/);
    expect(source).toMatch(/<button[\s\S]*>\s*保持\s*<\/button>/);
    expect(source).not.toContain('className="restart-cwd-choice');
  });

  it("shows both complete paths as wrapping text outside the buttons", () => {
    expect(source).toMatch(/<\/button>[\s\S]*至新启动目录：[\s\S]*<code[\s\S]*{restartCwdDecision.runtime_cwd}/);
    expect(source).toMatch(/<\/button>[\s\S]*在旧工作目录：[\s\S]*<code[\s\S]*{restartCwdDecision.session_cwd}/);
    expect(styles).toMatch(/\.restart-cwd-option code \{[\s\S]*font-family: var\(--ui-font\);[\s\S]*overflow-wrap: anywhere;[\s\S]*word-break: break-word;[\s\S]*white-space: normal;/);
    expect(styles).not.toMatch(/\.restart-cwd-option code \{[\s\S]*(SFMono|Cascadia Code|Consolas|monospace)/);
  });

  it("uses compact project-styled buttons and puts long mobile paths on their own line", () => {
    expect(styles).toMatch(/\.restart-cwd-option button \{[\s\S]*min-width: 42px;[\s\S]*height: 25px;[\s\S]*background: #315f52;/);
    expect(styles).toContain(':root[data-theme="light"] .restart-cwd-gate');
    expect(styles).toMatch(/@media \(max-width: 720px\) \{[\s\S]*\.restart-cwd-option code \{ flex-basis: 100%; padding-left: 47px; \}/);
  });
});
