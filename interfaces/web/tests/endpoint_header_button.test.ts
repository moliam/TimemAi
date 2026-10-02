import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { zh as zhCatalog } from "../src/i18n/strings.zh";
import { en as enCatalog } from "../src/i18n/strings.en";

const source = readFileSync(new URL("../src/main.tsx", import.meta.url), "utf8");
const styles = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8");

describe("header endpoint selector", () => {
  it("renders the endpoint name and folding control inside one button", () => {
    expect(source).toMatch(
      /className={`header-model[\s\S]*<span title={headerModelLabel}>{headerModelLabel}<\/span>[\s\S]*<ChevronDown/,
    );
    expect(source).toMatch(/headerModelEndpoint[\s\S]*apiProtocolShort\(headerModelEndpoint\.api_protocol\)/);
    expect(source).toMatch(/headerModelEndpoint\.reasoning_effort &&/);
    expect(source).toMatch(/className="header-model-meta"/);
    expect(source).toContain("aria-expanded={showRuntime}");
  });

  it("uses Lucide direction icons instead of in/out text in the cache readout", () => {
    expect(source).toContain('<ArrowBigUp size={12} strokeWidth={1.8} />');
    expect(source).toContain('<ArrowBigDown size={12} strokeWidth={1.8} />');
    expect(source).toContain('className="header-cache-token header-cache-input"');
    expect(source).toContain('className="header-cache-token header-cache-output"');
    expect(source).not.toContain('`cache: ${cacheHitPercent.toFixed(1)}%, in:');
    expect(source).toContain('`${cachePercentLabel}, input: ${cacheInputLabel}, output: ${cacheOutputLabel}`');
    expect(styles).toContain('.header-cache-rate { justify-self: start; display: inline-flex; align-items: center; gap: 5px;');
    expect(styles).toContain('.header-cache-token { display: inline-flex; align-items: center; gap: 1px; }');
  });

  it("uses a deep borderless surface with the same height as the ctx/cache readout", () => {
    expect(styles).toContain(
      ".header-context-actions { align-self: center; border-left: 1px solid #2a3944; padding-left: 12px; margin-left: 2px; height: 38px; }",
    );
    expect(styles).toContain(
      ':root[data-theme="light"] .header-context-actions { border-left-color: #ccd9dd; }',
    );
    expect(styles).toContain(".header-context { min-height: 38px; }");
    expect(styles).toMatch(
      /\.header-session-cluster \.header-model \{[\s\S]*font-size: 13px;/,
    );
    expect(styles).toMatch(
      /\.header-session-cluster \.header-model \{[\s\S]*height: 38px;[\s\S]*border: 0;[\s\S]*background: #244a40;[\s\S]*color: #fff;[\s\S]*box-shadow: none;/,
    );
    expect(styles).toMatch(
      /:root\[data-theme="light"\] \.header-session-cluster \.header-model \{[\s\S]*border: 0;[\s\S]*background: #315f52;[\s\S]*color: #fff;[\s\S]*box-shadow: none;/,
    );
  });
  it("moves the current session name out of the header and into the collapsed sidebar", () => {
    expect(source).not.toContain(
      '<strong title={activeSession?.display_name ?? "No session"}>',
    );
    expect(source).toContain('className="collapsed-session-card"');
    expect(source).toContain(
      'title={activeSession?.display_name ?? "No session"}',
    );
    expect(styles).toContain(
      '.sidebar.collapsed > :not(.collapsed-brand, .collapsed-session-card, .sidebar-footer)',
    );
    expect(styles).toMatch(
      /\.collapsed-session-card \{[\s\S]*min-height: 92px;[\s\S]*border: 0;[\s\S]*background: linear-gradient\(180deg, #17372f[\s\S]*box-shadow:/,
    );
    expect(styles).toMatch(
      /\.collapsed-session-card span \{[\s\S]*text-overflow: ellipsis;[\s\S]*transform: translate\(-50%, -50%\) rotate\(90deg\);/,
    );
    expect(styles).not.toContain("writing-mode: vertical-rl");
    expect(styles).toMatch(
      /@media \(max-width: 1050px\) \{[\s\S]*\.collapsed-brand,[\s\S]*\.collapsed-session-card,[\s\S]*display: none;/,
    );
  });

});

it("keeps editor checkboxes compact instead of styled as text inputs", () => {
  expect(styles).toContain('.endpoint-transport-toggle > span { display: flex; align-items: center; gap: 7px; }');
  expect(styles).toContain('.endpoint-transport-toggle input { width: 14px; height: 14px; margin: 0; accent-color: #64bbaa; }');
  const bareTextInputRules =
    styles.match(/\.endpoint-editor-grid input(?!:not)/g) ?? [];
  expect(bareTextInputRules).toEqual([]);
});

it("explains the redirect impact in plain language for both toggle states", () => {
  expect(source).toContain('endpoints.redirectImpact');
  expect(source).toMatch(/draft\.allow_cross_origin_redirects[\s\S]*endpoint-redirect-impact on/);
  expect(styles).toContain('.endpoint-redirect-impact.on { color: #d4b25f; font-weight: 700; }');
});

it("orders basic endpoint fields before optional transport settings", () => {
  const editor = source.slice(source.indexOf('<div className="endpoint-editor-grid">'));
  const fields = ['value={draft.name}', 'value={draft.model}', 'value={draft.base_url}', 'value={apiKey}', 'value={draft.api_protocol}', 'value={draft.max_llm_input_tokens}', 'checked={draft.allow_cross_origin_redirects}'];
  const positions = fields.map((field) => editor.indexOf(field));
  expect(positions.every((position) => position >= 0)).toBe(true);
  expect(positions).toEqual([...positions].sort((a, b) => a - b));
});

it("uses direct numeric entry without one-token budget steppers", () => {
  expect(source.match(/className="endpoint-token-budget" type="number" inputMode="numeric"/g)).toHaveLength(2);
  expect(styles).toContain(".endpoint-token-budget { appearance: textfield;");
  expect(styles).toContain(".endpoint-token-budget::-webkit-inner-spin-button,");
  expect(styles).toContain(".endpoint-token-budget::-webkit-outer-spin-button { -webkit-appearance: none;");
});


it("groups reasoning controls with accessible chips and a compact policy row", () => {
  const panel = source.slice(source.indexOf('<section className="wide endpoint-reasoning-panel"'), source.indexOf('{catalogInvalid &&'));
  expect(panel).toContain('aria-label={t("endpoints.reasoningSettings")}');
  expect(panel).toContain('<legend className="endpoint-reasoning-legend">{t("endpoints.allowedReasoning")}</legend>');
  expect(panel).toContain('className="endpoint-reasoning-chip"');
  expect(panel).toContain('type="checkbox"');
  expect(panel).toContain('aria-pressed={selectedTemplate');
  expect(panel).toContain(': allowed == null}');
  expect(panel).toContain('className="endpoint-reasoning-policy"');
  expect(panel).toContain('className="endpoint-reasoning-adaptive"');
  expect(panel).not.toContain('adaptiveReasoningHint');
  expect(styles).toContain('.endpoint-reasoning-chip input:focus-visible + span');
  expect(styles).toContain(':root[data-theme="light"] .endpoint-reasoning-chip input:checked + span');
  expect(styles).toContain('.endpoint-reasoning-policy { min-width: 0; display: grid; grid-template-columns: minmax(0, 1fr) max-content;');
  expect(styles).toMatch(/@media \(max-width: 600px\) \{[\s\S]*\.endpoint-reasoning-policy \{ grid-template-columns: minmax\(0, 1fr\)/);
});

it("renders localized capability and save feedback instead of internal reason strings", () => {
  expect(source).toContain("endpointCapabilityIssue(draft, selectedModel)");
  expect(source).toContain("endpointCapabilityIssueMessage(capabilityIssue)");
  expect(source).toContain("endpointSaveErrorMessage(event.error)");
  expect(source).not.toContain("selectedProtocol?.disabled_reason ?? selectedProtocol?.fixed_reason");
  expect(source).not.toContain('<small className="endpoint-constraint-note">{selectedProtocol.fixed_reason}</small>');
  expect(source).toContain("dailyReasoningOptions.map");
});

it("renders localized import diagnostics and command failures instead of raw Host details", () => {
  expect(source).toContain("endpointImportIssueMessage(issue)");
  expect(source).not.toContain("<li key={issue}>{issue}</li>");
  expect(source).toContain('endpointImportCommandErrorMessage("scan", event.error)');
  expect(source).toContain('endpointImportCommandErrorMessage("apply", event.error)');
  expect(source).not.toMatch(/model_endpoint_import_[a-z_]+.*event\.error/);
});

it("uses a template-specific restore action without overstating unknown capabilities", () => {
  expect(zhCatalog.endpoints.restoreTemplateDefault).toBe("恢复模板推理档位");
  expect(enCatalog.endpoints.restoreTemplateDefault).toBe("Restore template reasoning levels");
  expect(source).toContain('? "endpoints.restoreTemplateDefault"');
  expect(source).toContain('restoreEndpointTemplateReasoning(current, selectedTemplate)');
  expect(source).toContain(': selectedModel');
  expect(source).toContain('? "endpoints.modelReasoningRange"');
  expect(source).toContain(': "endpoints.unknownCapabilities"');
  expect(source).not.toContain('endpoints.unknownReasoningRangeHint');
});


it("keeps explicit save and cancel actions above editable fields in a sticky bar", () => {
  const start = source.indexOf('<div className="endpoint-editor-topbar">');
  const end = source.indexOf('<label className="endpoint-catalog-picker">', start);
  const topbar = source.slice(start, end);
  expect(topbar).toContain('onClick={onClose}');
  expect(topbar).toContain('t("common.cancel")');
  expect(topbar).toContain('disabled={saveDisabled}');
  expect(source).toContain('const hasChanges = endpointDraftChanged(initialDraftRef.current, endpointDraft)');
  expect(source).toMatch(/const saveDisabled =\s*!hasChanges \|\|/);
  expect(topbar).toContain('onClick={save}');
  expect(topbar).toContain('t("endpoints.saveEndpoint")');
  expect(topbar).not.toContain('endpoints.saveRequiredHint');
  expect(source.match(/className="endpoint-editor-buttons"/g)).toHaveLength(1);
  expect(styles).toContain('.endpoint-editor-topbar { position: sticky; top: 0;');
  expect(styles).toContain('.endpoint-editor-buttons .primary:disabled {');
  expect(styles).toContain(':root[data-theme="light"] .endpoint-editor-buttons .primary:disabled {');
});


it("keeps endpoint setup concise without introductory or template-help copy", () => {
  expect(source).not.toContain('t("endpoints.builderIntro")');
  expect(source).not.toContain('t("endpoints.templateHint")');
  expect(source).not.toContain('t("endpoints.saveRequiredHint")');
  expect(source).toContain('className="endpoint-catalog-picker"');
});


it("keeps normal and adaptive reasoning controls together on one compact row", () => {
  const panel = source.slice(source.indexOf('<section className="wide endpoint-reasoning-panel"'), source.indexOf('{catalogInvalid &&'));
  const policy = panel.slice(panel.indexOf('<div className="endpoint-reasoning-policy">'), panel.indexOf('</div>', panel.indexOf('<div className="endpoint-reasoning-policy">')));
  expect(panel.indexOf('endpoint-reasoning-config')).toBeLessThan(panel.indexOf('endpoint-reasoning-policy'));
  expect(policy).toContain('endpoint-reasoning-daily');
  expect(policy).toContain('endpoint-reasoning-adaptive');
  expect(policy).toContain('endpoints.dailyReasoningLevel');
  expect(policy).toContain('endpoints.adaptiveReasoning');
  expect(policy).not.toContain('endpoints.adaptiveReasoningHint');
  expect(panel).toContain('<summary><strong>{t("endpoints.configureReasoningClick")}</strong>');
  expect(panel).not.toContain('endpoint-reasoning-heading');
});


it("prefixes template display names with the provider without changing option identities", () => {
  expect(source).toContain('<option key={m.id} value={m.id}>{m.provider === "openai" ? "OpenAI" : m.provider === "zhipu" ? t("endpoints.zhipu") : m.provider}: {m.label}</option>');
});


it("shows a top-right check only for selected reasoning chips", () => {
  expect(source).toContain('<Check className="endpoint-reasoning-check" size={11} strokeWidth={3} aria-hidden="true" />');
  expect(styles).toContain('.endpoint-reasoning-check { position: absolute; top: 2px; right: 2px; visibility: hidden;');
  expect(styles).toContain('.endpoint-reasoning-chip input:checked + span .endpoint-reasoning-check { visibility: visible; }');
});


it("keeps endpoint share and edit actions together on the right", () => {
  const row = source.slice(source.indexOf('className={`endpoint-settings-row'), source.indexOf('</div>\n            );', source.indexOf('className={`endpoint-settings-row')));
  expect(row).toContain('className="endpoint-settings-actions"');
  expect(row).toContain('endpoint-share-export');
  expect(row).toContain('endpoint-settings-edit');
  expect(row).toContain('<Pencil size={14} /> {t("common.edit")}');
  expect(row).not.toContain('<Pencil size={14} /> {t("endpoints.editEndpoint")}');
  expect(row.indexOf('endpoint-share-export')).toBeLessThan(row.indexOf('endpoint-settings-edit'));
  expect(styles).toContain('.endpoint-settings-row { min-width: 0; display: grid; grid-template-columns: minmax(0, 1fr) auto;');
  expect(styles).toContain('.endpoint-settings-actions { display: flex; align-items: center; justify-content: flex-end;');
});

it("opens endpoint sharing as a separate top-level dialog instead of inline settings content", () => {
  const panel = readFileSync(new URL("../src/endpoint_share.tsx", import.meta.url), "utf8");
  expect(panel).toContain("createPortal(");
  expect(panel).toContain("document.body");
  expect(panel).toContain('className="endpoint-share-backdrop"');
  expect(panel).toContain('role="dialog"');
  expect(source).toContain("<EndpointSharePanel");
  expect(styles).toContain(".endpoint-share-backdrop { position: fixed; z-index: 70;");
  expect(styles).not.toContain(".endpoint-share-panel { border: 1px solid");
});

it("presents endpoint sharing without exposing its encoding format", () => {
  const zh = readFileSync(new URL("../src/i18n/strings.zh.ts", import.meta.url), "utf8");
  const en = readFileSync(new URL("../src/i18n/strings.en.ts", import.meta.url), "utf8");
  const panel = readFileSync(new URL("../src/endpoint_share.tsx", import.meta.url), "utf8");
  expect(zh).toContain('shareExport: "分享"');
  expect(zh).toContain('shareGenerate: "分享"');
  expect(en).toContain('shareExport: "Share"');
  expect(en).toContain('shareGenerate: "Share"');
  const zhShareCopy = ["shareExport", "shareImport", "shareString", "shareGenerate", "shareInvalidBase64", "shareTooLarge"]
    .map((key) => (zhCatalog.endpoints as Record<string, string>)[key])
    .join(" ");
  const enShareCopy = ["shareExport", "shareImport", "shareString", "shareGenerate", "shareInvalidBase64", "shareTooLarge"]
    .map((key) => (enCatalog.endpoints as Record<string, string>)[key])
    .join(" ");
  expect(zhShareCopy).not.toMatch(/Base64/i);
  expect(enShareCopy).not.toMatch(/Base64/i);
  expect(panel).not.toMatch(/Base64 string|Base64 字符串/);
});


it("allows endpoint selection while a request is working and explains the request boundary", () => {
  const panelStart = source.indexOf("function ModelEndpointPanel");
  const panelEnd = source.indexOf("function ModelEndpointEditor", panelStart);
  const panel = source.slice(panelStart, panelEnd);
  const applyStart = source.indexOf("onApply={(endpointId) => {");
  const applyEnd = source.indexOf("}}", applyStart);
  const apply = source.slice(applyStart, applyEnd);
  expect(panel).toContain("disabled={!session}");
  expect(panel).not.toContain('disabled={!session || session.state === "working"}');
  expect(apply).toContain("if (!activeSession) return;");
  expect(apply).not.toContain('activeSession.state === "working"');
  expect(zhCatalog.endpoints.workingNote).toContain("下一次请求生效");
  expect(enCatalog.endpoints.workingNote).toContain("next request");
});
