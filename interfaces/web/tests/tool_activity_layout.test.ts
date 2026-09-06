import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";

const source = readFileSync(new URL("../src/main.tsx", import.meta.url), "utf8");
const styles = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8");

describe("tool activity row layout", () => {
  it("keeps status and duration in one grid cell", () => {
    expect(source).toContain('className="tool-activity-meta"');
    expect(source).toMatch(
      /className="tool-activity-meta"[\s\S]*className="tool-activity-status"[\s\S]*className="tool-activity-duration"/,
    );
  });

  it("uses the leading icon as the expand control without a redundant trailing chevron", () => {
    expect(source).toContain('className="tool-activity-icon tool-activity-chevron"');
    expect(source).toContain('className="tool-activity-group-icon tool-activity-chevron"');
    expect(source).not.toContain('<ChevronRight className="tool-activity-chevron" size={14} />');
    expect(styles).toContain(".tool-activity-command { min-width: 0; grid-column: 4; justify-self: start;");
    expect(styles).toContain("grid-template-columns: 16px max-content max-content minmax(0, 1fr);");
    expect(styles).not.toContain("grid-template-columns: 16px max-content max-content minmax(0, 1fr) 14px;");
  });
  it("keeps the top-level background status before the shrinkable tool counts", () => {
    expect(source).toContain('toolActivityGroupStatusLabel(summary)');
    expect(source).toContain('activeParts.push(`bg ${summary.backgroundRunningCount}`)');
    expect(source).toMatch(
      /className="tool-activity-group-status"[\s\S]*className="tool-activity-group-counts"/,
    );
    expect(styles).toContain(
      "grid-template-columns: 16px max-content minmax(0, 1fr)",
    );
    expect(styles).toContain(
      ".tool-activity-group-counts { min-width: 0;",
    );
  });

  it("uses compact aligned terminal labels and always includes the failure count", () => {
    expect(source).toContain('summary.status === "completed") return "✓"');
    expect(source).toContain('return `✗(${summary.failedCount})`');
    expect(source).not.toContain('summary.failedCount > 1');
  });

  it("renders live wait-budget countdowns and clarifies timeout handoff", () => {
    expect(source).toContain('className="tool-activity-countdown"');
    expect(source).toContain("formatRemainingDuration(remainingWaitMs)");
    expect(source).toContain("wait ended · process still running · pid");
  });

});

describe("stream tool status continuity", () => {
  it("keeps the dot and terminal status in one leading cell before the name", () => {
    const row = source.slice(source.indexOf("const StreamToolRow ="), source.indexOf("function TurnAnswerDelivery"));
    expect(row).toMatch(/className="stream-tool-status-slot"[\s\S]*className="stream-tool-dot"[\s\S]*<ActionStatus[\s\S]*<b>\{toolName\}<\/b>/);
    expect(styles).toContain("min-width: 14px; align-self: center;");
    expect(row).toContain('className="stream-tool-background">(bg)');
    expect(styles).toContain(".stream-tool-background { color: #98afbc; opacity: .65; }");
  });
});


describe("collapsed tool summary", () => {
  it("labels prior tools explicitly and aligns the disclosure with live rows", () => {
    expect(source).toContain('<span>tools</span>');
    expect(styles).toMatch(/\.stream-tool-run-toggle \{[^}]*font-weight: 400;/);
    expect(styles).toMatch(/\.stream-tool-run-toggle \{[^}]*padding: 7px 4px;/);
  });
});

it("keeps automatic tool absorption free of page-wide height animation", () => {
  const rule = styles.match(/\.stream-tool-merged-item \{([^}]*)\}/)?.[1];
  expect(rule).toBeDefined();
  expect(rule).not.toContain("transition:");
  expect(styles).toContain(".stream-tool-count.incremented { animation:");
});

it("animates running dots without changing layout dimensions", () => {
  expect(styles).toContain("animation: stream-tool-breathe 1.2s ease-in-out infinite");
  expect(styles).toContain("transform: scale(.65)");
  expect(styles).toContain("transform: scale(1)");
});
