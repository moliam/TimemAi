import { afterEach, describe, expect, it, vi } from "vitest";
import { readFileSync } from "node:fs";

afterEach(() => { vi.unstubAllGlobals(); vi.resetModules(); });
async function setup(values: Record<string, string> = {}, blocked = false) {
  const data = new Map(Object.entries(values));
  let storage: (event: { key: string | null }) => void = () => {};
  vi.stubGlobal("window", {
    localStorage: {
      getItem: (key: string) => { if (blocked) throw Error("blocked"); return data.get(key) ?? null; },
      setItem: (key: string, value: string) => { if (blocked) throw Error("blocked"); data.set(key, value); },
    },
    addEventListener: (_: string, listener: typeof storage) => { storage = listener; },
  });
  return { ...(await import("../src/beta_preferences")), data, storage: (key: string | null) => storage({ key }) };
}
describe("Beta browser preferences", () => {
  it("defaults both off normally and on for debug without persisting defaults", async () => {
    const p = await setup();
    for (const name of ["stream", "toolResults"] as const) expect(p.getBetaPreference(name)).toBe(false);
    p.applyBetaDebugDefault(true);
    for (const name of ["stream", "toolResults"] as const) expect(p.getBetaPreference(name)).toBe(true);
    expect(p.data.size).toBe(0);
    p.applyBetaDebugDefault(false);
    expect(p.getBetaPreference("stream")).toBe(false);
  });
  it("preserves explicit true and false across reconnect and reload", async () => {
    let p = await setup();
    p.setBetaPreference("stream", false);
    p.setBetaPreference("toolResults", true);
    p.applyBetaDebugDefault(true);
    expect(p.getBetaPreference("stream")).toBe(false);
    p.applyBetaDebugDefault(false);
    expect(p.getBetaPreference("toolResults")).toBe(true);
    const saved = Object.fromEntries(p.data);
    vi.resetModules();
    p = await setup(saved);
    p.applyBetaDebugDefault(true);
    expect(p.getBetaPreference("stream")).toBe(false);
    expect(p.getBetaPreference("toolResults")).toBe(true);
  });
  it("syncs other tabs, removal and clear, with invalid values using defaults", async () => {
    const key = "timem-web-tool-result-status-v1";
    const p = await setup({ [key]: "invalid" });
    p.applyBetaDebugDefault(true);
    expect(p.getBetaPreference("toolResults")).toBe(true);
    p.data.set(key, "false"); p.storage(key);
    expect(p.getBetaPreference("toolResults")).toBe(false);
    p.data.delete(key); p.storage(null);
    expect(p.getBetaPreference("toolResults")).toBe(true);
  });
  it("works page-locally when storage is unavailable", async () => {
    const p = await setup({}, true);
    p.applyBetaDebugDefault(true);
    p.setBetaPreference("toolResults", false);
    expect(p.getBetaPreference("toolResults")).toBe(false);
  });
  it("wires Host defaults, Beta switch, neutral rows and folded groups without mutating tool status", () => {
    const main = readFileSync(new URL("../src/main.tsx", import.meta.url), "utf8");
    expect(main).toContain("applyBetaDebugDefault(snapshot.server.debug_mode)");
    expect(main).toContain("<ToolResultStatusSetting />");
    expect(main).toContain('!showResults && !isToolActivityRunning(status) ? "Done" : label');
    expect(main).toContain('`${completed.length} Done`');
    expect(main).toContain('if (!showResults && summary.status !== "running") return "Done"');
    expect(main).toContain('if (showResults && summary.failedCount > 0)');
  });
});
