import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import {
  parseResourceCapPercent,
  RESOURCE_CAP_PERCENT_OPTIONS,
  resourceCapability,
  resourceCapabilityIsInert,
  resourceCapHintKind,
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

  it("classifies dimensions into actionable, password-gated, temporarily unavailable, and unsupported", () => {
    expect(resourceCapability(undefined)).toBe("manageable");
    expect(resourceCapability({ kernel: "applied", escalation_required: false })).toBe("manageable");
    expect(resourceCapability({ kernel: "applied", escalation_required: true })).toBe("manageable");
    expect(resourceCapability({ kernel: "disabled", escalation_required: false })).toBe("manageable");
    expect(resourceCapability({ kernel: "unavailable", escalation_required: true })).toBe("needs-permission");
    // A real facility that is temporarily unusable (for example a shared
    // cgroup root) is not the same as a platform without the facility.
    expect(resourceCapability({ kernel: "unavailable", escalation_required: false })).toBe("unavailable");
    expect(resourceCapability({ kernel: "unsupported", escalation_required: true })).toBe("unsupported");
    expect(resourceCapability({ kernel: "unsupported", escalation_required: false })).toBe("unsupported");
    expect(resourceCapabilityIsInert("unavailable")).toBe(true);
    expect(resourceCapabilityIsInert("unsupported")).toBe(true);
    expect(resourceCapabilityIsInert("manageable")).toBe(false);
    expect(resourceCapabilityIsInert("needs-permission")).toBe(false);
  });

  it("maps the shared session-root blocker to actionable guidance", () => {
    expect(
      resourceCapHintKind({
        kernel: "unavailable",
        escalation_required: false,
        blocker: "shared-instance-root",
      }),
    ).toBe("shared-instance-root");
    expect(
      resourceCapHintKind({ kernel: "unavailable", escalation_required: false }),
    ).toBe("unavailable");
    expect(
      resourceCapHintKind({ kernel: "unsupported", escalation_required: false }),
    ).toBe("unsupported");
    // Permission-gated dimensions stay interactive and use the password
    // dialog; they must never be mislabelled as unsupported.
    expect(
      resourceCapHintKind({ kernel: "unavailable", escalation_required: true }),
    ).toBeNull();
    expect(
      resourceCapHintKind({ kernel: "applied", escalation_required: false }),
    ).toBeNull();
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

  it("greys inert dimensions and explains them with a native hover title", () => {
    expect(mainSource).toContain("data-capability={row.capability}");
    expect(mainSource).toContain("is-inert");
    expect(mainSource).toContain("resource-cap-warning");
    expect(mainSource).toContain("resourceCapabilityIsInert(row.capability)");
    expect(mainSource).toContain("resourceCpuUnsupportedHint");
    expect(mainSource).toContain("resourceMemUnsupportedHint");
    expect(mainSource).toContain("resourceCpuUnavailableHint");
    expect(mainSource).toContain("resourceMemUnavailableHint");
    expect(mainSource).toContain("resourceMemSharedRootHint");
    expect(mainSource).toContain("resourceCapHintKind");
    expect(mainSource).toContain("resource-cap-hint");
    expect(mainSource).toContain("title={row.hint}");
    // The settings pane scrolls and clips, so the old absolutely positioned
    // CSS tooltip was cut off; only the native title remains.
    expect(mainSource).not.toContain("data-tooltip");
    expect(stylesSource).not.toContain("resource-cap-warning::after");
    expect(stylesSource).toContain(".resource-cap-row.is-inert .resource-cap-switch");
    expect(stylesSource).toContain(".resource-cap-row.is-inert .resource-cap-name");
    expect(stylesSource).toContain(".resource-cap-row.is-inert .resource-cap-select");
    expect(stylesSource).toContain(".resource-cap-row.is-inert .resource-cap-slider");
    expect(stylesSource).toContain(".resource-cap-warning");
    expect(stylesSource).not.toContain(".resource-caps-status");
  });

  it("dismisses the password dialog with a document-level Escape and focuses it on open", () => {
    expect(mainSource).toContain("resourceCapsPasswordInputRef");
    expect(mainSource).toContain("resourceCapsPasswordInputRef.current?.focus");
    expect(mainSource).toContain('document.addEventListener("keydown", dismissOnEscape)');
    expect(mainSource).toContain('if (event.key !== "Escape") return');
    // The dialog opts into the app-level Tab containment hook.
    expect(mainSource).toContain('role="dialog"');
    expect(mainSource).toContain('aria-modal="true"');
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

  it("keeps inert dimensions inoperable instead of saving a stale ON flag", () => {
    expect(mainSource).toContain("resourceCapabilityIsInert(row.capability)");
    // Save uses the effective drafts, so unsupported/unavailable dimensions
    // submit OFF.
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
      "resourceCpuUnavailableHint",
      "resourceMemUnsupportedHint",
      "resourceMemUnavailableHint",
      "resourceMemSharedRootHint",
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
