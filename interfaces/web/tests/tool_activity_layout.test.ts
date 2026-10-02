import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";

const source = readFileSync(new URL("../src/main.tsx", import.meta.url), "utf8");
const styles = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8");

describe("tool activity row layout", () => {
  it("shows a leading dot only while running and keeps timing at the right edge", () => {
    const activity = source.slice(source.indexOf("function ToolActivity("), source.indexOf("function MemoIcon("));
    expect(activity).toContain('className="tool-activity-running-marker"');
    expect(activity).toContain('className="tool-activity-dot"');
    expect(activity).toContain('className="tool-activity-timing"');
    expect(activity).toContain('className="tool-activity-background">{t("tools.statusBg")}');
    expect(activity).toContain(') : status !== "completed" ? (');
    expect(activity).toContain('className="tool-activity-status"');
    expect(activity).toMatch(/tool-activity-running-marker[\s\S]*<b>[\s\S]*tool-activity-command[\s\S]*tool-activity-timing/);
  });

  it("uses the whole row as disclosure without a persistent expand glyph", () => {
    const activity = source.slice(source.indexOf("function ToolActivity("), source.indexOf("function MemoIcon("));
    expect(activity).not.toContain('tool-activity-icon tool-activity-chevron');
    expect(source).toContain('className="tool-activity-group-icon tool-activity-chevron"');
    expect(styles).toContain(".tool-activity.completed .tool-activity-command,");
    expect(styles).toContain("grid-template-columns: max-content minmax(0, 1fr) max-content;");
    expect(styles).toContain("grid-template-columns: 14px max-content minmax(0, 1fr) max-content;");
    expect(activity).toMatch(/tool-activity-running-marker[\s\S]*<b>[\s\S]*tool-activity-command/);
    expect(activity).toContain("event.currentTarget.contains(selection.anchorNode)");
  });
  it("keeps the top-level background status before the shrinkable tool counts", () => {
    expect(source).toContain('toolActivityGroupStatusLabel(summary, showResults)');
    expect(source).toContain('activeParts.push(t("tools.bgCount", { count: summary.backgroundRunningCount }));');
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

  it("removes completed group checks while retaining explicit failure counts", () => {
    expect(source).toContain('summary.status === "completed") return ""');
    expect(source).toContain('groupStatusLabel && <span className="tool-activity-group-status">');
    expect(source).toContain('return `✗(${summary.failedCount})`');
    expect(source).not.toContain('summary.failedCount > 1');
  });

  it("renders live wait-budget countdowns and clarifies timeout handoff", () => {
    expect(source).toContain('className="tool-activity-countdown"');
    expect(source).toContain("formatRemainingDuration(remainingWaitMs)");
    expect(source).toContain('t("tools.waitEndedRunning", { pid: activity.pid })');
  });

});

describe("stream tool status continuity", () => {
  it("keeps status, tool identity and command in order on a row-wide disclosure", () => {
    const row = source.slice(source.indexOf("const StreamToolRow ="), source.indexOf("function TurnAnswerDelivery"));
    expect(row).toMatch(/running \? \([\s\S]*className="stream-tool-status-slot"[\s\S]*className="stream-tool-dot"[\s\S]*status !== "completed" \? \([\s\S]*<ActionStatus[\s\S]*className="stream-tool-status"[\s\S]*<b>[\s\S]*stream-tool-command-preview/);
    expect(row).not.toContain("<ChevronRight");
    expect(row).toContain('role={hasExpandableDetail ? "button" : undefined}');
    expect(row).toContain('event.key !== "Enter" && event.key !== " "');
    expect(styles).toContain(".stream-tool-head.stream-tool-toggle:hover");
    expect(styles).toContain("min-width: 14px; height: 17px; line-height: 17px; align-self: center;");
    expect(row).toContain(') : status !== "completed" ? (');
    expect(styles).toContain(".stream-tool-head { display: flex; align-items: center;");
    expect(row).toContain('className="stream-tool-background">(bg)');
    expect(styles).toContain(".stream-tool-background { color: #98afbc; opacity: .65; }");
  });
  it("folds retired tools away entirely with a compact elapsed label", () => {
    const row = source.slice(source.indexOf("const StreamToolRow ="), source.indexOf("function TurnAnswerDelivery"));
    expect(row).toContain("stream-tool-elapsed");
    expect(row).toContain("formatToolElapsed(activity.elapsed_ms!)");
    expect(row).toContain("formatLiveElapsed(liveElapsedMs)");
    expect(row).not.toContain("summarized");
    expect(styles).toContain(".stream-tool-merged-item.merged { grid-template-rows: 0fr; opacity: 0; }");
    expect(styles).not.toContain(".stream-tool-run { position: relative; }");
    expect(styles).not.toContain("stream-tool-row::before");
    expect(styles).toContain(".stream-tool-elapsed { color: #98afbc; font-variant-numeric: tabular-nums; flex: none; }");
  });
});


describe("collapsed tool summary", () => {
  it("labels prior tools explicitly and aligns the disclosure with live rows", () => {
    // The visible "tools" word became a wrench glyph; the label stays
    // available to screen readers via the sr-only span.
    expect(source).toContain('<span className="sr-only">{t("tools.toolsLabel")}</span>');
    expect(source).toContain('className="stream-tool-run-glyph"');
    // The disclosure +/- sign leads, the wrench glyph follows.
    const summary = source.match(/<span className="stream-tool-run-sign".*?stream-tool-run-glyph"/s)?.[0];
    expect(summary).toBeDefined();
    expect(summary?.indexOf("stream-tool-run-sign")).toBeLessThan(
      summary?.indexOf("stream-tool-run-glyph") ?? -1,
    );
    expect(styles).toMatch(/\.stream-tool-run-toggle \{[^}]*font-weight: 400;/);
    expect(styles).toMatch(/\.stream-tool-run-toggle \{[^}]*padding: 2px 4px 2px 0;/);
  });
});

it("keeps automatic tool absorption free of page-wide height animation", () => {
  const rule = styles.match(/\.stream-tool-merged-item \{([^}]*)\}/)?.[1];
  expect(rule).toBeDefined();
  expect(rule).not.toContain("transition:");
  expect(styles).toContain(".stream-tool-count.incremented { animation:");
});

it("keeps the running dot static without pulsing animations", () => {
  expect(styles).not.toContain("stream-tool-breathe");
  expect(styles).not.toContain("stream-tool-glow");
  expect(styles).not.toMatch(/\.stream-tool-row\.running \.stream-tool-dot \{[^}]*animation/);
});

it("keeps live text/tool spacing compact without shrinking touch targets", () => {
  expect(styles).toContain("clamp(.125rem, calc(var(--content-size) * .125), .25rem)");
  expect(styles).toContain(".turn-stream-tools > .stream-thought-text { margin-bottom: 0; }");
  expect(styles).toMatch(/\.stream-tool-row \{[^}]*padding: 2px 0;/);
  expect(styles).toContain(".stream-tool-head.stream-tool-toggle, .stream-tool-run-toggle { min-height: 44px; }");
});

it("bundles IBM Plex Mono locally and scopes it to all tool rendering", () => {
  expect(styles).toContain('src: url("/fonts/IBMPlexMono-Latin-300.woff2") format("woff2")');
  expect(styles).toMatch(/\.stream-tool-count \{[^}]*font-family: "IBM Plex Mono"/);
  expect(styles).toMatch(/\.stream-tool-run,[\s\S]*\.tool-activity \{[\s\S]*font-family: "IBM Plex Mono"/);
  expect(styles).toContain('.stream-tool-head,\n.stream-tool-run-toggle,\n.tool-activity,\n.tool-activity b { font-size: 12.5px; }');
  expect(styles).toMatch(/\.stream-tool-count \{[^}]*font-size: calc\(var\(--content-size\) \* \.888889\); font-weight: 300;/);
  const font = readFileSync(new URL("../public/fonts/IBMPlexMono-Latin-300.woff2", import.meta.url));
  expect(font.readUInt32BE(0)).toBe(0x774f4632);
  expect(readFileSync(new URL("../public/fonts/IBMPlexMono-OFL.txt", import.meta.url), "utf8")).toContain("SIL OPEN FONT LICENSE");
});

it("uses the structured run_bash edit summary in ordinary and stream rows", () => {
  const stream = source.slice(source.indexOf("const StreamToolRow ="), source.indexOf("function TurnAnswerDelivery"));
  const ordinary = source.slice(source.indexOf("function ToolActivity("), source.indexOf("function MemoIcon("));
  expect(source).toContain('MemorySearchInvocation');
  expect(source).toContain('SelfToolIcon');
  expect(source).toContain('SelfToolInvocation');
  expect(stream).toContain('activity.run_bash_edit ? <RunBashEditIcon />');
  expect(stream).toContain('<RunBashEditInvocation edit={activity.run_bash_edit} />');
  expect(ordinary).toContain('activity.run_bash_edit ? <RunBashEditIcon />');
  expect(ordinary).toContain('<RunBashEditInvocation edit={activity.run_bash_edit} />');
  expect(styles).toContain('.tool-activity .bash-edit-preview { grid-column: 3; width: 100%; }');
  expect(styles).toContain('.stream-tool-head > .bash-edit-preview { flex: 1; }');
});


it("increases readable tool invocation summaries by half a pixel", () => {
  expect(styles).toContain('.tool-invocation-preview { font-size: 11px !important; line-height: 1.5; }');
  expect(styles).toContain('.bash-edit-preview { font-size: 11.5px !important; }');
  expect(source).toContain('className="stream-tool-command-preview tool-invocation-preview"');
  expect(source).toContain('className="tool-activity-command tool-invocation-preview"');
  expect(styles).toMatch(/\.file-tool-preview \{[^}]*font-size: 11px;/);
  expect(styles).toMatch(/\.bash-edit-preview \{[^}]*font-size: 11px;/);
});

it("uses structured memory and self-tool summaries in ordinary and stream rows", () => {
  const stream = source.slice(source.indexOf("const StreamToolRow ="), source.indexOf("function TurnAnswerDelivery"));
  const ordinary = source.slice(source.indexOf("function ToolActivity("), source.indexOf("function MemoIcon("));
  for (const section of [stream, ordinary]) {
    expect(section).toContain('<MemorySearchInvocation search={activity.memory_search} />');
    expect(section).toContain('<SelfToolIcon />');
    expect(section).toContain('<SelfToolInvocation operation={activity.self_tool} />');
  }
  expect(stream).toContain('!structuredInvocation ? toolInvocationPreview(activity) : ""');
  expect(ordinary).toContain('activity.tool_name === "sub_answer" || structuredInvocation ? undefined');
});

it("uses terminal glyphs for Bash and reduces default Lucide stroke weight", () => {
  expect(source).toContain('<SquareTerminal size={14} aria-hidden="true" />');
  expect(source).toContain('className="bash-tool-icon" title={toolName}');
  expect(styles).toContain(':where(svg.lucide[stroke-width="2"]) { stroke-width: 1.5; }');
  expect(source).toContain('strokeWidth={1.575}');
});

it("renders an Infinity reasoning notice only from Core upgrade projections", () => {
  expect(source).toContain('kind === "reasoning_upgrade"');
  expect(source).not.toContain('event.payload.reasoning_enabled === true');
  expect(source).toContain('<InfinityIcon size={13} />');
  expect(source).toContain('t("context.reasoningUpgrade", { from: event.payload.from, to: event.payload.to })');
});


it("uses one typography contract for system-level notices", () => {
  const activityView = source.slice(source.indexOf("function ActivityView("), source.indexOf("function ToolGenNotice("));
  const memo = source.slice(source.indexOf("function MemoNotice("), source.indexOf("function toolInvocationPreview("));
  const compact = source.slice(source.indexOf("function ContextCompactNotice("), source.indexOf("function DecisionModal("));
  expect(activityView).toContain('system-notice reasoning-notice');
  expect(activityView).toContain('className="system-notice-title"');
  expect(memo).toContain('system-notice memo-notice');
  expect(memo).toContain('className="system-notice-detail system-notice-long-detail"');
  expect(compact.match(/system-notice context-compact-notice/g)).toHaveLength(2);
  expect(source).toContain('className={`system-notice system-notice-row toolgen-notice');
  expect(source).toContain('className="system-notice-icon" aria-hidden="true"><Wrench size={13} />');
  expect(source).toContain('className="system-notice-row"');
  expect(styles).toContain('.system-notice .system-notice-row { grid-template-columns: 16px minmax(0, 1fr) max-content; align-items: center; column-gap: 6px; padding: 6px; }');
  expect(styles).toContain('.turn-work-item.system-notice { font-family: inherit; font-size: 10px; font-weight: 500; line-height: 1.5; }');
  expect(styles).toContain('.turn-work-item.system-notice,\n.toolgen-notice.system-notice { font-size: 10.5px; }');
  expect(styles).toContain('.system-notice .system-notice-icon { width: 16px; height: 20px; align-self: center;');
  expect(styles).toContain('.toolgen-notice.system-notice summary::after { grid-column: 3; margin: 0;');
  expect(styles).toContain('.toolgen-notice.system-notice.published summary::before { display: none; }');
  expect(styles).toContain('.system-notice .system-notice-title { min-width: 0; color: var(--system-notice-accent); font: inherit; font-weight: 650; }');
  expect(styles).toContain('.system-notice .system-notice-detail { color: var(--system-notice-text); font: inherit; font-weight: 500; font-variant-numeric: tabular-nums; }');
  expect(styles).toMatch(/@media \(max-width: 720px\) \{[\s\S]*\.system-notice \.system-notice-line \{ flex-wrap: wrap; white-space: normal; \}/);
  expect(styles).not.toContain('.memo-notice .memo-notice-line');
  expect(styles).not.toContain('.compact-notice .compact-notice-line');
});


it("narrows tool rendering from the right edge while preserving mobile width", () => {
  expect(styles).toContain("width: 90%;\n  max-width: 90%;\n  margin-right: auto;");
  expect(styles).toMatch(/@media \(max-width: 720px\) \{[\s\S]*\.stream-tool-run,[\s\S]*width: 100%; max-width: 100%;/);
});
