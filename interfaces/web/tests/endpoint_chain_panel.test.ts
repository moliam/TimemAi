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
    expect(panel).toContain('t(working ? "endpoints.chainWorkingHint" : "endpoints.chainSelectionHint")');
    expect(panel).toContain('t("endpoints.chainPending")');
    expect(panel).toContain('"endpoints.chainSynced"');
    expect(panel).toContain('t("endpoints.selectedCount", { count: chain.length })');
  });

  it("keeps the status strip visible above the lists on desktop and mobile", () => {
    expect(styles).toContain(".endpoint-chain-actions { display: flex;");
    expect(styles).not.toContain(".endpoint-chain-apply");
    expect(styles).toContain(".endpoint-chain-checkbox.selected {");
    expect(styles).toContain("@media (max-width: 720px) { .endpoint-chain-actions { align-items: stretch; flex-direction: column; }");
    expect(zhCatalog.endpoints.chainSelectionHint).toContain("点击即选择");
    expect(zhCatalog.endpoints.chainSelectionHint).toContain("立即生效");
    expect(enCatalog.endpoints.chainSelectionHint).toContain("immediately");
    expect(enCatalog.endpoints.chainPending).toBe("Updating chain…");
  });

  it("separates the Host-observed current route from the next-Turn preference", () => {
    expect(panel).toContain("active_model_endpoint_id");
    expect(panel).toContain("model_endpoint_id ?? null");
    expect(source).toContain('t(working ? "endpoints.chainCurrent" : "endpoints.chainPreferred")');
    expect(source).toContain('t("endpoints.chainNextPreferred")');
    expect(panel).toContain('t(working ? "endpoints.chainNextApplied" : "endpoints.chainSynced")');
    expect(panel).not.toContain("effectiveEndpointForProfile");
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
    // Active badge is a filled pill, not a ghost outline.
    expect(styles).toContain("background: rgba(47, 143, 118, .18); color: #7ee0c3;");
    expect(styles).toContain(':root[data-theme="light"] .endpoint-rate-flag { color: #8a5f1d; }');
  });


  it("widens the desktop panel without breaking the mobile layout", () => {
    expect(styles).toContain('@media (min-width: 721px) { .endpoint-menu:not(.editing) { width: min(600px, calc(100vw - 48px)); } }');
    expect(styles).toContain("@media (max-width: 720px) { .endpoint-chain { grid-template-columns: 1fr; } }");
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
