import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { zh } from "../src/i18n/strings.zh";
import { en } from "../src/i18n/strings.en";

const main = readFileSync(new URL("../src/main.tsx", import.meta.url), "utf8");
const prefs = readFileSync(new URL("../src/beta_preferences.ts", import.meta.url), "utf8");
const editsView = readFileSync(new URL("../src/file_edits_view.tsx", import.meta.url), "utf8");
const styles = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8");

describe("settings category regroup", () => {
  it("splits the former system pane into context, chat, governance and about", () => {
    expect(main).toContain('{section === "context" && (');
    expect(main).toContain('{section === "chat" && (');
    expect(main).toContain('{section === "governance" && (');
    expect(main).toContain('{section === "about" && (');
    expect(main).not.toContain('{section === "system" && (');
    for (const id of ["context-settings-title", "chat-settings-title", "governance-settings-title", "about-settings-title"]) {
      expect(main).toContain(id);
    }
  });

  it("keeps context settings in the context pane", () => {
    const start = main.indexOf('{section === "context" && (');
    const end = main.indexOf('{section === "chat" && (');
    const pane = main.slice(start, end);
    for (const token of [
      "system-runtime-tuning",
      "system.toolResultLimitTitle",
      "system.contextCompressThresholdTitle",
      "system.reasoningContextToggle",
      "system-restore-prompt-context",
    ]) {
      expect(pane).toContain(token);
    }
    expect(pane).not.toContain("settings.autoCollapseTitle");
    expect(pane).not.toContain("system-resource-caps");
  });

  it("keeps display switches in the chat interface pane", () => {
    const start = main.indexOf('{section === "chat" && (');
    const end = main.indexOf('{section === "governance" && (');
    const pane = main.slice(start, end);
    for (const token of [
      "settings.autoCollapseTitle",
      "settings.fileEditsWrapTitle",
      "settings.thinkingDisplayTitle",
      "<StreamUiModeSetting />",
      "<ToolResultStatusSetting />",
    ]) {
      expect(pane).toContain(token);
    }
  });

  it("keeps resource caps and beta features in the governance pane", () => {
    const start = main.indexOf('{section === "governance" && (');
    const end = main.indexOf('{section === "about" && (');
    const pane = main.slice(start, end);
    for (const token of [
      "system-resource-caps",
      "system.betaFeaturesTitle",
      "<strong>{t(\"beta.enableToolGen\")}</strong>",
      "<strong>{t(\"beta.toolDiscovery\")}</strong>",
    ]) {
      expect(pane).toContain(token);
    }
  });

  it("hosts the upgrade and project info in the about pane", () => {
    const pane = main.slice(main.indexOf('{section === "about" && ('));
    expect(pane).toContain("<SelfUpgradeSetting />");
    expect(pane).toContain('t("settings.projectUrl")');
    expect(pane).toContain('href={t("settings.projectUrl")}');
    expect(zh.settings.projectUrl).toBe("https://github.com/moliam/TimemAi");
    expect(en.settings.projectUrl).toBe("https://github.com/moliam/TimemAi");
    expect(zh.settings.projectLabel).toBe("项目地址");
    expect(en.settings.projectLabel).toBe("Project");
  });

  it("defaults the new switches on and pins them against the debug flag", () => {
    expect(prefs).toContain('autoCollapse: "timem-web-turn-auto-collapse-v1"');
    expect(prefs).toContain('fileEditsWrap: "timem-web-file-edits-wrap-v1"');
    // 工作过程中预览工具输出结果默认关：多跳命令的结果框不再一起展开再一起收起。
    expect(prefs).toMatch(/const defaults: Record<Preference, boolean> = \{\s*stream: true, streamArchive: false, toolResults: true,\s*autoCollapse: true, fileEditsWrap: true, fileEditsDuringWork: true,\s*toolPreviewDuringWork: false,\s*\};/);
    expect(prefs).toContain('fileEditsDuringWork: "timem-web-file-edits-during-work-v1"');
    expect(prefs).toContain('toolPreviewDuringWork: "timem-web-tool-preview-during-work-v1"');
    for (const name of ["stream", "streamArchive", "toolResults", "autoCollapse", "fileEditsWrap", "fileEditsDuringWork", "toolPreviewDuringWork"]) {
      expect(prefs).toContain(`"${name}"`);
    }
  });

  it("folds file edit sections by default and wraps diffs when enabled", () => {
    expect(editsView).toContain('const autoCollapse = useBetaPreference("autoCollapse");');
    expect(editsView).toContain("const autoExpand = shouldAutoExpandDiff(edit.diff, { autoCollapse, expandDuringWork });");
    expect(editsView).toContain("const [expanded, setExpanded] = useState(autoExpand);");
    expect(editsView).toContain('useBetaPreference("fileEditsWrap")');
    expect(editsView).toContain('`file-edits-panel${wrap ? " edits-wrap" : ""}`');
    expect(styles).toContain(".file-edits-panel.edits-wrap .file-edit-diff-table { width: 100%; }");
    expect(styles).toContain(".file-edits-panel.edits-wrap .file-edit-code { white-space: pre-wrap; overflow-wrap: anywhere; }");
  });

  it("aligns the work stream and diff cards with the assistant reply column", () => {
    // 阅读列、工具行与 diff 卡跟随输入框宽度（不再单独收窄居中）。
    expect(styles).toMatch(/\.turn-interaction \{[^}]*max-width: none;/);
    expect(styles).toContain("padding: 7px 0 10px 14px;");
    expect(styles).toContain(".file-edits-panel.edits-wrap .file-edit-code { white-space: pre-wrap; overflow-wrap: anywhere; }");
  });

  it("boosts the dark-theme add/delete row contrast", () => {
    expect(styles).toContain(".file-edit-row-add { background: rgba(64, 180, 120, 0.17); }");
    expect(styles).toContain(".file-edit-row-del { background: rgba(214, 92, 84, 0.17); }");
    expect(styles).toContain(".file-edit-row-add .file-edit-code { color: #b9eccc; }");
    expect(styles).toContain(".file-edit-row-del .file-edit-code { color: #f6b9b2; }");
    expect(styles).toContain(':root[data-theme="light"] .file-edit-row-add .file-edit-code { color: #1f5c40; }');
    expect(styles).toContain(':root[data-theme="light"] .file-edit-row-del .file-edit-code { color: #7e2d26; }');
  });

  it("restores the prompt context on startup by default", () => {
    expect(main).toContain("server?.mem?.restore_prompt_context_on_startup ?? true");
  });
});
