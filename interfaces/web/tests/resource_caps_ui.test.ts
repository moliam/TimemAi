import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import {
  parseResourceCapPercent,
  RESOURCE_CAP_PERCENT_OPTIONS,
  resourceCapability,
} from "../src/runtime_settings";
import { zh } from "../src/i18n/strings.zh";
import { en } from "../src/i18n/strings.en";

const mainSource = readFileSync(new URL("../src/main.tsx", import.meta.url), "utf8");
const protocolSource = readFileSync(new URL("../src/protocol.ts", import.meta.url), "utf8");
const stylesSource = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8");
const serverSource = readFileSync(
  new URL("../../../applications/timem/src/server.rs", import.meta.url),
  "utf8",
);

describe("resource caps ui", () => {
  it("parses valid percents and rejects everything else", () => {
    expect(parseResourceCapPercent("85")).toBe(85);
    expect(parseResourceCapPercent(" 70.5 ")).toBe(70.5);
    expect(parseResourceCapPercent("100")).toBe(100);
    expect(parseResourceCapPercent("1")).toBe(1);
    expect(parseResourceCapPercent("0")).toBeNull();
    expect(parseResourceCapPercent("101")).toBeNull();
    expect(parseResourceCapPercent("-5")).toBeNull();
    expect(parseResourceCapPercent("abc")).toBeNull();
    expect(parseResourceCapPercent("")).toBeNull();
    expect(parseResourceCapPercent("nan")).toBeNull();
    expect(parseResourceCapPercent("1.234")).toBeNull();
  });

  it("exposes fixed preset tiers only", () => {
    expect([...RESOURCE_CAP_PERCENT_OPTIONS]).toEqual([10, 25, 50, 70, 85, 100]);
  });

  it("classifies dimensions into actionable, password-gated, and unsupported", () => {
    expect(resourceCapability(undefined)).toBe("manageable");
    expect(resourceCapability({ kernel: "applied", escalation_required: false })).toBe("manageable");
    expect(resourceCapability({ kernel: "applied", escalation_required: true })).toBe("manageable");
    expect(resourceCapability({ kernel: "disabled", escalation_required: false })).toBe("manageable");
    expect(resourceCapability({ kernel: "unavailable", escalation_required: true })).toBe("needs-permission");
    expect(resourceCapability({ kernel: "unavailable", escalation_required: false })).toBe("unsupported");
    expect(resourceCapability({ kernel: "unsupported", escalation_required: true })).toBe("unsupported");
  });

  it("wires switches, preset selects, and the initial capability state end to end", () => {
    expect(protocolSource).toContain('type: "system_resource_caps_update"');
    expect(protocolSource).toContain("cpu_enabled: boolean");
    expect(protocolSource).toContain('type: "system_resource_caps_escalate"');
    expect(protocolSource).toContain("escalation_required: boolean;");
    expect(mainSource).toContain('type="checkbox"');
    expect(mainSource).toContain("resource-cap-select");
    expect(mainSource).toContain("RESOURCE_CAP_PERCENT_OPTIONS");
    expect(mainSource).toContain("onResourceCapsEscalate");
    expect(mainSource).toContain('type="password"');
    expect(mainSource).toContain("resourceCapability(resourceCaps?.cpu)");
    expect(mainSource).toContain("effectiveSystemCpuCapEnabled");
    expect(mainSource).toContain("effectiveSystemMemCapEnabled");
    expect(mainSource).toContain("mem_settings_updated");
    expect(stylesSource).toContain(".resource-cap-switch");
    expect(stylesSource).toContain(".resource-caps-modal");
    expect(stylesSource).toContain(".resource-cap-select");
    // The free-form number input interaction was replaced by the switch + select.
    expect(stylesSource).not.toContain(".resource-cap-input");
  });

  it("renders unavailable enforcement as OFF, never as a technical status badge", () => {
    // The effective state feeds both the initial draft and the dirty check, so
    // a stored `enabled: true` cannot make an unmanaged dimension look ON.
    const effectiveCpu = mainSource.indexOf("const effectiveSystemCpuCapEnabled =");
    const effectiveMem = mainSource.indexOf("const effectiveSystemMemCapEnabled =");
    expect(effectiveCpu).toBeGreaterThan(-1);
    expect(effectiveMem).toBeGreaterThan(effectiveCpu);
    expect(mainSource).toContain(
      'resourceCpuCapability === "manageable" ? systemCpuCapEnabled : false',
    );
    expect(mainSource).toContain(
      'resourceMemCapability === "manageable" ? systemMemCapEnabled : false',
    );
    // The old "不可用 · 需要 root" badge is gone entirely.
    expect(mainSource).not.toContain("resource-caps-status");
    expect(mainSource).not.toContain("resourceCapsKernel");
    expect(mainSource).not.toContain("resourceCapsEscalationBadge");
    expect(mainSource).not.toContain("resourceCapsEscalationRequired");
  });

  it("greys unsupported dimensions and leaves only a hoverable warning icon", () => {
    expect(mainSource).toContain("data-capability={row.capability}");
    expect(mainSource).toContain("is-unsupported");
    expect(mainSource).toContain("resource-cap-warning");
    expect(mainSource).toContain("resourceCpuUnsupportedHint");
    expect(mainSource).toContain("resourceMemUnsupportedHint");
    expect(mainSource).toContain('row.capability === "unsupported"');
    expect(mainSource).toContain('title={t(');
    expect(stylesSource).toContain(".resource-cap-row.is-unsupported .resource-cap-switch");
    expect(stylesSource).toContain(".resource-cap-row.is-unsupported .resource-cap-name");
    expect(stylesSource).toContain(".resource-cap-row.is-unsupported .resource-cap-select");
    expect(stylesSource).toContain(".resource-cap-row.is-unsupported .resource-cap-slider");
    expect(stylesSource).toContain(".resource-cap-warning");
    expect(stylesSource).not.toContain(".resource-caps-status");
  });

  it("prompts for a one-shot password when a permission-gated switch is turned on", () => {
    expect(mainSource).toContain("resourceCapsEscalateTarget");
    expect(mainSource).toContain('capability === "needs-permission"');
    expect(mainSource).toContain("setResourceCapsEscalateOpen(true)");
    // The gated dimension is forced on only for the escalation call after the
    // password is supplied; cancelling leaves the switch OFF.
    expect(mainSource).toContain('target === "cpu" ? true : resourceCpuEnabledDraft');
    expect(mainSource).toContain('target === "mem" ? true : resourceMemEnabledDraft');
    expect(mainSource).toContain("setResourceCapsEscalateTarget(null)");
    // The password never lands in storage or logs: the host zeroizes it in memory.
    expect(serverSource).toContain("struct ZeroizingPassword");
    expect(serverSource).toContain("SystemResourceCapsEscalate");
    expect(serverSource).toContain("apply_resource_cap_escalation_plan");
  });

  it("keeps unsupported dimensions inoperable instead of saving a stale ON flag", () => {
    expect(mainSource).toContain('row.capability === "unsupported"');
    // Save uses the effective drafts, so unsupported dimensions submit OFF.
    const submit = mainSource.slice(
      mainSource.indexOf("const submitResourceCaps = () =>"),
      mainSource.indexOf("const submitResourceCapsEscalation = () =>"),
    );
    expect(submit).toContain("resourceCpuEnabledDraft");
    expect(submit).toContain("resourceMemEnabledDraft");
  });

  it("renders the caps save action in the shared settings button language", () => {
    expect(zh.system.resourceCapsSave).toBe("保存");
    expect(en.system.resourceCapsSave).toBe("Save");
    expect(mainSource).toContain("resource-caps-action-status");
    expect(mainSource).toContain(
      'className={`primary compact ${resourceCapsPending ? "sending" : ""}`}',
    );
    expect(stylesSource).toContain(".resource-caps-actions .primary");
    expect(stylesSource).toContain(".resource-caps-action-status");
  });

  it("keeps i18n keys in sync", () => {
    const keys = [
      "resourceCapsTitle",
      "resourceCapsDesc",
      "resourceCpuCapTitle",
      "resourceCpuCapAria",
      "resourceCpuCapDesc",
      "resourceMemCapTitle",
      "resourceMemCapAria",
      "resourceMemCapDesc",
      "resourceCpuUnsupportedHint",
      "resourceMemUnsupportedHint",
      "resourceCapsSave",
      "resourceCapsInvalid",
      "resourceCapsErrorTitle",
      "resourceCapsErrorBody",
      "resourceCapsEscalateTitle",
      "resourceCapsEscalateDesc",
      "resourceCapsEscalatePassword",
      "resourceCapsEscalateConfirm",
      "resourceCapsEscalateCancel",
    ];
    for (const key of keys) {
      expect(zh.system).toHaveProperty(key);
      expect(en.system).toHaveProperty(key);
    }
  });
});
