import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { zh } from "../src/i18n/strings.zh";
import { en } from "../src/i18n/strings.en";

const source = readFileSync(
  new URL("../src/restart_cwd_gate.tsx", import.meta.url),
  "utf8",
);
const mainSource = readFileSync(
  new URL("../src/main.tsx", import.meta.url),
  "utf8",
);
const styles = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8");

describe("restart working-directory gate", () => {
  it("replaces the composer in the sticky input area until a choice is made", () => {
    expect(mainSource).toMatch(
      /<div className="composer-wrap aui-thread-footer">[\s\S]*{activeSession && restartCwdDecision \? \([\s\S]*<RestartCwdGate[\s\S]*\) : \([\s\S]*<form[\s\S]*className="composer"/,
    );
    expect(mainSource).not.toMatch(
      /{activeSession && restartCwdDecision && \([\s\S]*<TimemThread/,
    );
    expect(styles).toMatch(
      /\.restart-cwd-gate \{[\s\S]*width: 100%;[\s\S]*margin: 0;/,
    );
  });

  it("keeps the normal mismatch notice concise and offers both valid actions", () => {
    expect(source).toContain('t("restartGate.mismatchPrompt")');
    expect(source).toContain(
      'canKeepSessionDirectory ? t("restartGate.switchToRuntime") : t("restartGate.useRuntime")',
    );
    expect(source).toMatch(
      /<button[\s\S]*>\s*\{t\("restartGate.keepInSession"\)\}\s*<\/button>/,
    );
    expect(source).not.toContain('className="restart-cwd-choice');
    // Catalog copy keeps the original wording intent in both languages.
    expect(zh.restartGate.mismatchPrompt).toContain("启动目录");
    expect(zh.restartGate.mismatchPrompt).toContain("工作目录");
    expect(zh.restartGate.switchToRuntime).toBe("切换");
    expect(zh.restartGate.useRuntime).toBe("使用当前工作目录");
    expect(en.restartGate.mismatchPrompt).toContain("working directory");
  });

  it("shows both complete paths as wrapping text outside the buttons", () => {
    expect(source).toMatch(/<\/button>[\s\S]*t\("restartGate.toNewRuntime"\)[\s\S]*<code[\s\S]*{decision.runtime_cwd}/);
    expect(source).toMatch(/<\/button>[\s\S]*t\("restartGate.inOldSession"\)[\s\S]*<code[\s\S]*{decision.session_cwd}/);
    expect(styles).toMatch(/\.restart-cwd-option code \{[\s\S]*font-family: var\(--ui-font\);[\s\S]*overflow-wrap: anywhere;[\s\S]*word-break: break-word;[\s\S]*white-space: normal;/);
    expect(styles).not.toMatch(/\.restart-cwd-option code \{[\s\S]*(SFMono|Cascadia Code|Consolas|monospace)/);
  });

  it("uses compact project-styled buttons and puts long mobile paths on their own line", () => {
    expect(styles).toMatch(/\.restart-cwd-option button \{[\s\S]*min-width: 42px;[\s\S]*height: 25px;[\s\S]*background: #315f52;/);
    expect(styles).toContain(':root[data-theme="light"] .restart-cwd-gate');
    expect(styles).toMatch(/@media \(max-width: 720px\) \{[\s\S]*\.restart-cwd-option code \{ flex-basis: 100%; padding-left: 47px; \}/);
  });

  it("gracefully falls back to a single switch action when the old directory is gone", () => {
    expect(source).toContain('t("restartGate.missingPrompt")');
    expect(source).toMatch(/canKeepSessionDirectory\s*&&\s*\([\s\S]*restartGate.keepInSession/);
    expect(source).toContain(
      'canKeepSessionDirectory ? t("restartGate.switchToRuntime") : t("restartGate.useRuntime")',
    );
    expect(source).toMatch(/canKeepSessionDirectory\s*&&\s*\([\s\S]*restartGate.toNewRuntime/);
    expect(zh.restartGate.missingPrompt).toContain("聊天记录已保留");
    expect(en.restartGate.missingPrompt).toContain("Chat history is kept");
    expect(source).toMatch(/onResolve\("use_runtime"\)/);
  });

  it("keeps command delivery and Session ownership in the parent composition", () => {
    expect(mainSource).toContain('import { RestartCwdGate } from "./restart_cwd_gate";');
    expect(mainSource).toMatch(/<RestartCwdGate[\s\S]*decision=\{restartCwdDecision\}[\s\S]*onResolve=\{onResolveRestartCwd\}/);
    expect(source).not.toContain("WebSocket");
    expect(source).not.toContain("useState");
  });
});
