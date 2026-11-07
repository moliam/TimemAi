import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { zh as zhCatalog } from "../src/i18n/strings.zh";
import { en as enCatalog } from "../src/i18n/strings.en";

const source = readFileSync(new URL("../src/main.tsx", import.meta.url), "utf8");
const styles = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8");

const panelStart = source.indexOf("function ModelEndpointPanel({");
const panelEnd = source.indexOf("function ModelEndpointEditor({");
const panel = source.slice(panelStart, panelEnd);

describe("endpoint chain selection panel", () => {
  it("renders the two-column chain picker with ordered dnd sorting", () => {
    expect(panel).toContain('className="endpoint-chain"');
    expect(panel).toContain('t("endpoints.chainAvailable")');
    expect(panel).toContain('t("endpoints.chainSelected")');
    expect(panel).toContain("verticalListSortingStrategy");
    expect(panel).toContain("moveEndpointInChain");
    expect(panel).toContain("toggleEndpointInChain");
    expect(panel).toContain("MAX_ENDPOINT_CHAIN_LENGTH");
    // Selection is commit-on-click: every toggle/reorder/remove applies
    // immediately; there is no separate confirm action.
    const actions = panel.indexOf('className={`endpoint-chain-actions');
    const lists = panel.indexOf('className="endpoint-chain"');
    expect(actions).toBeGreaterThan(-1);
    expect(actions).toBeLessThan(lists);
    expect(panel).not.toContain('className="primary compact endpoint-chain-apply"');
    expect(panel).toContain("const next = toggleEndpointInChain(chain, endpointId);");
    expect(panel).toContain("setChain(next);");
    expect(panel).toContain("applyImmediately(next);");
    expect(panel).toContain('if (!session || next.join(",") === boundChainKey) return;');
    expect(source).toContain('type: "model_endpoint_chain_apply"');
    expect(panel).toContain('aria-pressed={inChain}');
    expect(panel).toContain('className={`endpoint-chain-checkbox ${inChain ? "selected" : ""}`}');
    expect(panel).not.toContain('t(working ? "endpoints.chainWorkingHint" : "endpoints.chainSelectionHint")');
    expect(panel).not.toContain('t("endpoints.chainPending")');
    expect(panel).not.toContain('"endpoints.chainSynced"');
    expect(panel).toContain('t("endpoints.selectedCount", { count: chain.length })');
    expect(panel).toContain('className="endpoint-chain-col available"');
    expect(panel).toContain('className="endpoint-chain-col selected"');
  });

  it("keeps the selected-count strip visible above the lists on desktop and mobile", () => {
    expect(styles).toContain(".endpoint-chain-actions { display: flex;");
    expect(styles).not.toContain(".endpoint-chain-apply");
    expect(styles).toContain(".endpoint-chain-checkbox.selected {");
    expect(styles).toContain("@media (max-width: 720px) { .endpoint-chain-actions { align-items: stretch; flex-direction: column; }");
    expect(styles).toContain(".endpoint-chain-col.available .endpoint-chain-row { padding: 5px 7px; }");
    expect(styles).toContain(".endpoint-chain-col.selected { border: 1px dashed var(--management-border); border-radius: 8px; padding: 8px; }");
    expect(zhCatalog.endpoints.chainSelectionHint).toBeUndefined();
    expect(zhCatalog.endpoints.chainWorkingHint).toBeUndefined();
    expect(enCatalog.endpoints.chainSelectionHint).toBeUndefined();
    expect(enCatalog.endpoints.chainPending).toBeUndefined();
  });

  it("separates the Host-observed current route from the next-Turn preference", () => {
    expect(panel).toContain("active_model_endpoint_id");
    expect(panel).toContain("model_endpoint_id ??");
    // The top-right check always follows the Host-observed route, matching the
    // header, instead of switching to the next preference when idle.
    expect(panel).toContain("const currentEndpointId =");
    expect(panel).toMatch(/active_model_endpoint_id \?\?[\s\S]{0,80}model_endpoint_id \?\?/);
    expect(panel).not.toContain("working\n    ? session?.runtime_profile?.active_model_endpoint_id");
    expect(zhCatalog.endpoints.chainCurrent).toBe("当前使用");
    expect(enCatalog.endpoints.chainCurrent).toBe("In use");
    // Neither column uses a check badge for the in-use route; the selected
    // chain marks it with a quiet blue border instead.
    expect(source).not.toContain('className="endpoint-chain-active-badge"');
    expect(source).toContain('current ? "current" : ""');
    expect(styles).not.toContain(".endpoint-chain-active-badge");
    expect(styles).toContain(".endpoint-chain-row.in-chain.current {");
    expect(styles).toContain("border-color: #3b82f6;");
    expect(styles).not.toContain("0 0 9px #3b82f633");
    expect(styles).toContain(".endpoint-chain-row { position: relative; }");
    expect(source).not.toContain("endpoints.chainPreferred");
    expect(source).not.toContain("endpoints.chainNextPreferred");
    expect(source).toContain("endpoint-chain-index");
    expect(panel).not.toContain('t(working ? "endpoints.chainNextApplied" : "endpoints.chainSynced")');
    expect(panel).not.toContain("effectiveEndpointForProfile");
  });

  it("rolls back undelivered chain edits and marks the pending draft", () => {
    expect(panel).toContain("onApply: (endpointIds: string[]) => boolean");
    expect(panel).toContain("const accepted = onApply(next);");
    expect(panel).toContain("if (!accepted) {");
    expect(panel).toContain("setChain(boundChain);");
    expect(panel).toContain("endpoints.chainApplying");
    expect(panel).toContain('aria-live="polite"');
    expect(zhCatalog.endpoints.chainApplying).toBe("正在应用…");
    expect(enCatalog.endpoints.chainApplying).toBe("Applying…");
  });

  it("keeps the availability refresh button and probing animation wired", () => {
    expect(panel).toContain("onClick={onRefreshAvailability}");
    expect(source).toContain('type: "model_endpoint_availability_refresh"');
    expect(panel).toContain("endpoint-menu-refresh");
    expect(panel).toContain("anyProbing");
    expect(styles).toContain(".endpoint-menu-refresh.probing svg { animation: spin 1.2s linear infinite; }");
  });

  it("marks rate-limited endpoints in the chain and the settings list", () => {
    const flagStart = source.indexOf("function EndpointChainRateFlag({");
    expect(source.slice(flagStart, source.indexOf("function SortableEndpointChainRow({", flagStart))).toContain("rate_limited_since_ms");
    expect(panel).toContain("<EndpointChainRateFlag endpoint={endpoint} />");
    const settings = source.slice(source.indexOf("function EndpointSettingsPane({"));
    expect(settings).toContain("<EndpointChainRateFlag endpoint={endpoint} />");
    expect(styles).toContain(".endpoint-rate-flag");
    expect(zhCatalog.endpoints.rateLimited).toContain("限流");
    expect(enCatalog.endpoints.rateLimited).toContain("Rate-limited");
  });

  it("uses quiet theme surfaces and stable foregrounds for selection and hover", () => {
    // Selection and hover are distinct: selection uses a soft tinted surface
    // with a slim accent, while hover only lifts the surface one step.
    expect(styles).toContain(".endpoint-chain-row { display: flex; align-items: center; gap: 6px; border: 1px solid var(--management-border);");
    expect(styles).toContain(".endpoint-chain-row.in-chain { border-color: var(--management-accent-border); background: var(--management-accent-surface); }");
    // Selection is the tinted surface plus a slim border only; the thick left
    // bar is retired as a default pattern.
    expect(styles).not.toContain("box-shadow: inset 3px 0 var(--management-accent);");
    expect(styles).toContain("button.endpoint-chain-row:hover:not(:disabled) { border-color: var(--management-accent-border); background: var(--management-card-hover); }");
    expect(styles).toContain("button.endpoint-chain-row.in-chain:hover:not(:disabled) { border-color: var(--management-accent); background: var(--management-accent-surface-hover); }");
    // Foregrounds never invert when the pointer enters or a row is selected.
    expect(styles).toContain(".endpoint-chain-row.in-chain .endpoint-chain-pick strong { color: var(--management-text); }");
    expect(styles).toContain('.endpoint-chain-pick strong { font-size: 12px; color: var(--management-text);');
    expect(styles).toContain('.endpoint-chain-row.in-chain .endpoint-chain-pick small { color: #9fbec9; }');
    expect(styles).toContain(':root[data-theme="light"] .endpoint-chain-row.in-chain .endpoint-chain-pick small { color: #41625c; }');
    // The light theme uses the same management tokens rather than a second,
    // disconnected blue selection palette.
    expect(styles).toContain(':root[data-theme="light"] .endpoint-chain-row.in-chain { border-color: var(--management-accent-border); background: var(--management-accent-surface);');
    expect(styles).toContain(':root[data-theme="light"] .endpoint-chain-pick strong { color: var(--management-text); }');
    expect(styles).not.toContain(".endpoint-chain-row.in-chain { border-color: #4a9c8a; background: #18333a; }");
    // Remove button hover must not silently blend into the row.
    expect(styles).toContain(".endpoint-chain-remove:hover { color: #f0967f;");
    // The in-use route is a quiet blue border, not a filled badge.
    expect(styles).toContain(".endpoint-chain-row.in-chain.current { border-color: #3b82f6; }");
    // Available rows stay flat at rest; their button surface only appears on hover/focus.
    expect(styles).toContain(".endpoint-chain-col.available .endpoint-chain-row,\n.endpoint-chain-col.available .endpoint-chain-row.in-chain {\n  border-color: transparent;\n  background: transparent;\n  box-shadow: none;\n}");
    expect(styles).toContain(':root[data-theme="light"] .endpoint-rate-flag { color: #8a5f1d; }');
  });


  it("sizes the desktop panel to its content without breaking the mobile layout", () => {
    // The desktop panel follows its content between a readable floor and a
    // 600px ceiling; it is no longer a fixed 640px slab with dead space.
    expect(styles).toContain('width: fit-content; min-width: min(480px, calc(100vw - 48px)); max-width: min(600px, calc(100vw - 48px));');
    expect(styles).toContain("@media (max-width: 720px) { .endpoint-chain { grid-template-columns: 1fr; } }");
  });

  it("marks the in-use chain row with a blue border instead of a check badge", () => {
    expect(styles).not.toContain(".endpoint-chain-active-badge");
    expect(styles).toContain(".endpoint-chain-row.in-chain.current { border-color: #3b82f6; }");
    expect(styles).toContain(':root[data-theme="light"] .endpoint-chain-row.in-chain.current { border-color: #3b82f6; }');
  });

  it("subscribes locale-reactive chain components to the catalog", () => {
    const flagStart = source.indexOf("function EndpointChainRateFlag({");
    const flagEnd = source.indexOf("function SortableEndpointChainRow({");
    expect(source.slice(flagStart, flagEnd)).toContain("useT();");
    const rowStart = source.indexOf("function SortableEndpointChainRow({");
    const rowEnd = source.indexOf("function ModelEndpointPanel({");
    expect(source.slice(rowStart, rowEnd)).toContain("useT();");
  });
});
