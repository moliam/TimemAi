import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { zh } from "../src/i18n/strings.zh";
import { en } from "../src/i18n/strings.en";

const mainSource = readFileSync(new URL("../src/main.tsx", import.meta.url), "utf8");
const streamSettingsSource = readFileSync(
  new URL("../src/stream_ui_mode.tsx", import.meta.url),
  "utf8",
);
const protocolSource = readFileSync(
  new URL("../src/protocol.ts", import.meta.url),
  "utf8",
);
const stylesSource = readFileSync(
  new URL("../src/styles.css", import.meta.url),
  "utf8",
);

describe("System settings UI contract", () => {
  it("uses System as the first-level category and keeps Beta features below it", () => {
    expect(mainSource).toContain(
      'type SettingsSection = "appearance" | "endpoints" | "memory" | "system"',
    );
    expect(mainSource).toContain('onClick={() => selectSettingsSection("system")}');
    expect(mainSource).toContain('<strong>{t("settings.system")}</strong>');
    expect(mainSource).toContain('t("system.betaFeaturesTitle")');
    expect(mainSource).toContain('<strong>{t("beta.enableToolGen")}</strong>');
    expect(mainSource).toContain('<strong>{t("beta.toolDiscovery")}</strong>');
    expect(zh.settings.system).toBe("系统");
    expect(zh.system.title).toBe("系统");
    expect(zh.system.betaFeaturesTitle).toBe("Beta 功能");
    expect(en.settings.system).toBe("System");
  });

  it("offers the exact 30K, 20K, 16K, 10K and 8K Host-backed choices", () => {
    expect(mainSource).toContain(
      "([30720, 20480, 16384, 10240, 8192] as const)",
    );
    expect(mainSource).toMatch(/<select[\s\S]*?value=\{modelToolResultBytes\}/);
    expect(mainSource).not.toContain("system-tool-result-options");
    expect(mainSource).toContain('type: "system_model_tool_result_bytes_update"');
    expect(mainSource).toContain("server?.mem?.model_tool_result_bytes ?? 16384");
    expect(protocolSource).toContain(
      "export type ModelToolResultBytes = 8192 | 10240 | 16384 | 20480 | 30720",
    );
    expect(protocolSource).toContain(
      '| { type: "system_model_tool_result_bytes_update"; max_bytes: ModelToolResultBytes }',
    );
    expect(protocolSource).toContain("model_tool_result_bytes: ModelToolResultBytes");
    expect(mainSource).toContain("system-runtime-tuning");
    expect(mainSource).toContain("system-select-grid");
    expect(zh.system.toolResultLimitTitle).toBe("工具结果保留长度");
    expect(zh.system.toolResultLimitDesc).toBe("设置单个工具结果的最大保留长度(KB)");
    expect(en.system.toolResultLimitDesc).toContain("(KB)");
    expect(mainSource).not.toContain("system.toolResultLimitCurrent");
    expect(mainSource).not.toContain('t("beta.instructionPresent")');
  });

  it("aligns the runtime tuning columns: title row, select row, then trailing description", () => {
    const fieldBlock = (token: string) => {
      const start = mainSource.lastIndexOf("<label", mainSource.indexOf(`t("system.${token}")`));
      const end = mainSource.indexOf("</label>", start);
      return mainSource.slice(start, end);
    };
    for (const token of ["toolResultLimitTitle", "contextCompressThresholdTitle"]) {
      const block = fieldBlock(token);
      const titleAt = block.indexOf("system-select-title");
      const selectAt = block.search(/<select/);
      const descAt = block.indexOf("system-select-desc");
      expect(titleAt).toBeGreaterThan(-1);
      expect(selectAt).toBeGreaterThan(titleAt);
      expect(descAt).toBeGreaterThan(selectAt);
    }
    expect(stylesSource).toContain("grid-template-rows: subgrid");
    expect(stylesSource).toMatch(/\.system-select-field[^}]*grid-row: span 4/);
  });

  it("offers the exact Host-backed context compression thresholds with a 90% default", () => {
    expect(mainSource).toContain("([80, 85, 90, 95, 100] as const)");
    expect(mainSource).toMatch(
      /<select[\s\S]*?value=\{contextCompressThresholdPercent\}/,
    );
    expect(mainSource).not.toContain("system-context-compress-options");
    expect(mainSource).toContain(
      'type: "system_context_compress_threshold_update"',
    );
    expect(mainSource).toContain(
      "server?.mem?.context_compress_threshold_percent ?? 90",
    );
    expect(protocolSource).toContain(
      "export type ContextCompressThresholdPercent = 80 | 85 | 90 | 95 | 100",
    );
    expect(protocolSource).toContain(
      'type: "system_context_compress_threshold_update";',
    );
    expect(protocolSource).toContain(
      "context_compress_threshold_percent: ContextCompressThresholdPercent",
    );
    expect(zh.system.contextCompressThresholdTitle).toBe("上下文压缩阈值");
    expect(zh.system.contextCompressThresholdDesc).toContain("当前 MEM 保存");
    expect(zh.system.contextCompressThresholdDesc).toContain("下一次模型请求");
    expect(en.system.contextCompressThresholdTitle).toBe(
      "Context compression threshold",
    );
    expect(en.system.contextCompressThresholdDesc).toContain("current MEM");
    expect(en.system.contextCompressThresholdDesc).toContain("next model request");
  });

  it("places answer streaming in formal System settings before the Beta subsection", () => {
    expect(zh.system.streamUiTitle).toBe("即时显示回答");
    expect(zh.system.streamUiDesc).toBe("回答生成过程中即可查看内容。");
    expect(en.system.streamUiTitle).toBe("Show answers as they arrive");
    expect(en.system.streamUiDesc).toBe("See the answer while it is being generated.");
    expect("streamUiTitle" in zh.beta).toBe(false);
    expect("streamUiTitle" in en.beta).toBe(false);
    expect(streamSettingsSource).toContain('t("system.streamUiTitle")');
    expect(streamSettingsSource).not.toContain('t("beta.streamUiTitle")');
    expect(streamSettingsSource).toContain('className="settings-group system-feature-card"');
    const streamPosition = mainSource.indexOf("<StreamUiModeSetting />");
    const betaHeadingPosition = mainSource.indexOf('t("system.betaFeaturesTitle")');
    const betaSwitchPosition = mainSource.indexOf("<ToolResultStatusSetting />");
    expect(streamPosition).toBeGreaterThan(-1);
    expect(streamPosition).toBeLessThan(betaHeadingPosition);
    expect(betaHeadingPosition).toBeLessThan(betaSwitchPosition);
  });

  it("nests the archive sub-setting under answer streaming, default off", () => {
    expect(zh.system.streamArchiveTitle).toBe("任务完成后收起中间对话");
    expect(zh.system.streamArchiveDesc).toBe("任务结束后将过程内容收进折叠面板；关闭时过程内容保持展开。");
    expect(zh.system.streamArchiveAria).toBe("任务完成后收起中间对话");
    expect(en.system.streamArchiveTitle).toBe("Collapse interim conversation after the task finishes");
    expect(en.system.streamArchiveDesc).toBe("Fold process content into the collapsed panel once the task ends; keep it expanded when off.");
    expect("streamArchiveTitle" in zh.beta).toBe(false);
    expect("streamArchiveTitle" in en.beta).toBe(false);
    expect(streamSettingsSource).toContain("useStreamArchive");
    expect(streamSettingsSource).toContain('t("system.streamArchiveTitle")');
    expect(streamSettingsSource).toContain("stream-archive-subsetting");
    expect(streamSettingsSource).toMatch(/\{active &&/);
    expect(protocolSource).not.toContain("stream-archive");
  });

  it("describes settings by user benefit without implementation details", () => {
    expect(zh.beta.toolResultTitle).toBe("工具执行状态");
    expect(zh.beta.toolResultDesc).toBe("显示工具执行是否成功。");
    const copy = [
      zh.system.streamUiDesc,
      en.system.streamUiDesc,
      zh.beta.toolResultDesc,
      en.beta.toolResultDesc,
    ].join(" ");
    expect(copy).not.toMatch(/--debug|浏览器|browser|撤回|retract|invalid repl/i);
  });

  it("keeps discovery Host-authoritative and effective from the next request", () => {
    expect(mainSource).toContain(
      'type: "beta_claude_codex_tool_discovery_update"',
    );
    expect(mainSource).not.toContain('t("beta.pendingWait")');
    expect(mainSource).not.toContain('t("beta.instructionPresent")');
    expect(zh.beta.toolDiscoveryDesc).toContain("从下一次模型 API 请求开始生效");
    expect(mainSource).toContain(
      "server?.mem?.claude_codex_tool_discovery ?? false",
    );
    expect(protocolSource).toContain(
      '| { type: "beta_claude_codex_tool_discovery_update"; enabled: boolean }',
    );
    expect(protocolSource).toContain("claude_codex_tool_discovery: boolean");
  });
});
