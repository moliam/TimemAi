import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { zh } from "../src/i18n/strings.zh";
import { en } from "../src/i18n/strings.en";

const mainSource = readFileSync(new URL("../src/main.tsx", import.meta.url), "utf8");
const protocolSource = readFileSync(
  new URL("../src/protocol.ts", import.meta.url),
  "utf8",
);

describe("Beta settings UI contract", () => {
  it("uses Beta as the first-level settings category for ToolGen and discovery", () => {
    expect(mainSource).toContain(
      'type SettingsSection = "appearance" | "endpoints" | "memory" | "beta"',
    );
    expect(mainSource).not.toContain('| "toolgen"');
    expect(mainSource).toContain('onClick={() => selectSettingsSection("beta")}');
    expect(mainSource).toContain('<strong>{t("settings.beta")}</strong>');
    expect(mainSource).toContain('<strong>{t("beta.enableToolGen")}</strong>');
    expect(mainSource).toContain('<strong>{t("beta.toolDiscovery")}</strong>');
    expect(zh.beta.title).toBe("Beta");
    expect(zh.beta.enableToolGen).toBe("启用 ToolGen");
    expect(zh.beta.toolDiscovery).toContain("Claude/Codex 工具发现");
  });

  it("keeps discovery Host-authoritative and effective from the next request", () => {
    expect(mainSource).toContain(
      'type: "beta_claude_codex_tool_discovery_update"',
    );
    expect(mainSource).toContain('t("beta.pendingWait")');
    expect(en.beta.pendingWait).toContain("Waiting for the Host to persist and apply this setting.");
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
