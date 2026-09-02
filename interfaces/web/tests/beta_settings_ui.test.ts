import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

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
    expect(mainSource).toContain("<strong>Beta</strong>");
    expect(mainSource).toContain("<strong>Enable ToolGen</strong>");
    expect(mainSource).toContain("<strong>Claude/Codex 工具发现</strong>");
  });

  it("keeps discovery Host-authoritative and effective from the next request", () => {
    expect(mainSource).toContain(
      'type: "beta_claude_codex_tool_discovery_update"',
    );
    expect(mainSource).toContain(
      "Waiting for the Host to persist and apply this setting.",
    );
    expect(mainSource).toContain("从下一次模型 API 请求开始生效");
    expect(mainSource).toContain(
      "server?.mem?.claude_codex_tool_discovery ?? false",
    );
    expect(protocolSource).toContain(
      '| { type: "beta_claude_codex_tool_discovery_update"; enabled: boolean }',
    );
    expect(protocolSource).toContain("claude_codex_tool_discovery: boolean");
  });
});
