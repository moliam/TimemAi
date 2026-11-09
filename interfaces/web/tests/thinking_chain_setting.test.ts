import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { en } from "../src/i18n/strings.en";
import { zh } from "../src/i18n/strings.zh";

const source = readFileSync(new URL("../src/main.tsx", import.meta.url), "utf8");
const styles = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8");

describe("thinking chain setting", () => {
  it("persists the display preference under a stable storage key", () => {
    expect(source).toContain('const THINKING_CHAIN_STORAGE_KEY = "timem.thinking-chain.v1";');
    expect(source).toContain("function thinkingChainVisible(): boolean");
    expect(source).toContain("function setThinkingChainVisible(visible: boolean): void");
    expect(source).toContain("window.dispatchEvent(new Event(THINKING_CHAIN_CHANGE_EVENT))");
    expect(source).toContain("useSyncExternalStore(");
  });

  it("gates the inline thinking window on the instance-wide preference", () => {
    expect(source).toContain("{showThinkingChain &&\n                  thinkingTailShown && (");
    expect(source).toContain("<ThinkingPreviewWindow text={thinkingTailShown} />");
    expect(source).toContain("const showThinkingChain = useShowThinkingChain();");
  });

  it("exposes the toggle inside the system settings pane with the shared switch", () => {
    expect(source).toContain('className="settings-group system-thinking-chain"');
    expect(source).toContain('className="resource-cap-switch thinking-chain-switch"');
    expect(source).toContain('t("system.thinkingChainToggle")');
    expect(styles).toMatch(/\.resource-cap-switch \{[^}]*display: inline-flex/);
    expect(styles).toMatch(/\.resource-cap-switch input:checked \+ \.resource-cap-slider \{ background: #4d8f76; \}/);
    expect(styles).toMatch(/\.resource-cap-switch input:focus-visible \+ \.resource-cap-slider \{ outline: 2px solid #4d6a5f; outline-offset: 2px; \}/);
  });

  it("keeps localized labels in both catalogs", () => {
    for (const catalog of [zh, en]) {
      expect(catalog.system.thinkingChainTitle.length).toBeGreaterThan(0);
      expect(catalog.system.thinkingChainToggle.length).toBeGreaterThan(0);
      expect(catalog.system.thinkingChainAria.length).toBeGreaterThan(0);
      expect(catalog.system.reasoningContextToggle.length).toBeGreaterThan(0);
      expect(catalog.system.reasoningContextAria.length).toBeGreaterThan(0);
      expect(catalog.system.reasoningContextDesc.length).toBeGreaterThan(0);
    }
    expect(zh.system.thinkingChainToggle).toBe("显示思考链");
    expect(zh.system.reasoningContextToggle).toBe("思维链加入上下文");
    expect(zh.system.reasoningContextDesc).toBe(
      "把思维链添加入上下文，有助于提升任务能力，但消耗多余 token。",
    );
  });

  it("groups the context replay switch under the same thinking-chain block", () => {
    expect(source).toContain('className="thinking-chain-context-option"');
    expect(source).toContain('className="thinking-chain-context-desc"');
    expect(source).toContain('type: "system_include_reasoning_in_context_update"');
    expect(source).toContain('server?.mem?.include_reasoning_in_context ?? true');
  });
});
