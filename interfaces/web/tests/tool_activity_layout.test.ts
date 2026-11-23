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
    expect(activity).toContain('failed={isToolActivityFailed(status)}');
    expect(activity).not.toContain('<ActionStatus');
    expect(activity).toMatch(/tool-activity-running-marker[\s\S]*<b>[\s\S]*tool-activity-command[\s\S]*tool-activity-timing/);
  });


  it("uses one identity icon cell for success and failure in ordinary and stream rows", () => {
    const stream = source.slice(source.indexOf("const StreamToolRow ="), source.indexOf("function TurnAnswerDelivery"));
    const ordinary = source.slice(source.indexOf("function ToolActivity("), source.indexOf("function MemoIcon("));
    const icon = source.slice(source.indexOf("function ToolActivityIcon("), source.indexOf("const StreamToolRow ="));
    expect(icon).toContain('<CircleX size={14} aria-hidden="true" />');
    expect(icon).toContain('className="tool-failure-icon"');
    for (const section of [stream, ordinary]) {
      expect(section).toContain('<ToolActivityIcon activity={activity} toolName={toolName} failed={isToolActivityFailed(status)} />');
      expect(section).not.toContain('<ActionStatus');
    }
    expect(styles).toContain('.tool-activity.settled .tool-activity-timing { grid-column: 3; }');
    expect(styles).toContain('.tool-activity.settled :is(.file-tool-preview, .bash-edit-preview, .memory-search-preview, .self-tool-preview, .timer-tool-preview) { grid-column: 2; }');
    expect(styles).toContain('.tool-failure-icon { color: #d08181; }');
  });

  it("uses the whole row as disclosure without a persistent expand glyph", () => {
    const activity = source.slice(source.indexOf("function ToolActivity("), source.indexOf("function MemoIcon("));
    expect(activity).not.toContain('tool-activity-icon tool-activity-chevron');
    expect(source).toContain('className="tool-activity-group-icon tool-activity-chevron"');
    expect(styles).toContain(".tool-activity.settled .tool-activity-command,");
    expect(styles).toContain("grid-template-columns: max-content minmax(0, 1fr) max-content;");
    expect(styles).toContain(".tool-activity.running.tool-activity-static { grid-template-columns: 14px max-content minmax(0, 1fr) max-content; }");
    expect(styles).not.toContain(".tool-activity.terminal-status.tool-activity-static { grid-template-columns: 14px");
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

  it("renders live wait-budget countdowns without a second terminal status label", () => {
    expect(source).toContain('className="tool-activity-countdown"');
    expect(source).toContain("formatRemainingDuration(remainingWaitMs)");
    expect(source).not.toContain('className="tool-activity-status"');
  });

});

describe("stream tool status continuity", () => {
  it("keeps status, tool identity and command in order on a row-wide disclosure", () => {
    const row = source.slice(source.indexOf("const StreamToolRow ="), source.indexOf("function TurnAnswerDelivery"));
    expect(row).toMatch(/running && \([\s\S]*className="stream-tool-status-slot"[\s\S]*className="stream-tool-dot"[\s\S]*<b><ToolActivityIcon[\s\S]*stream-tool-command-preview/);
    expect(row).toContain('failed={isToolActivityFailed(status)}');
    expect(row).not.toContain('<ActionStatus');
    expect(row).not.toContain("<ChevronRight");
    expect(row).toContain('role={hasExpandableDetail ? "button" : undefined}');
    expect(row).toContain('event.key !== "Enter" && event.key !== " "');
    expect(styles).toContain(".stream-tool-head.stream-tool-toggle:hover");
    expect(styles).toContain("min-width: 14px; height: 17px; line-height: 17px; align-self: center;");
    expect(row).not.toContain('status !== "completed"');
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
    // 用户反馈：徽章不许盖住任何内容（含正文末行/卡片/尾行统计），且展开时不许跑位。
    // 因此徽章是组内第一行、流内靠右对齐；容器绝不归零高度，展开只向下长高。
    expect(styles).toMatch(/\.stream-tool-run-toggle \{[^}]*display: flex;[^}]*width: fit-content;[^}]*margin-left: auto;/s);
    expect(styles).not.toMatch(/\.stream-tool-run\.merged[^{]*\{[^}]*height: 0/);
    expect(styles).not.toMatch(/\.stream-tool-run-toggle \{[^}]*position: absolute/);
    // 零高度悬浮只允许出现在「每一行都确实收起」的组里；混合组仍有可见行
    // （文件修改卡、运行中行）时必须保持正常流高度，否则可见行会溢出叠印正文。
    expect(source).toContain("const folded = merged && completedIds.size === activities.length;");
    expect(source).toContain('className={`stream-tool-run${folded ? " merged" : ""}`}');
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
    // 用户反馈：徽章用无框色块（不要描边），只用底色把徽章和正文分开。
    expect(styles).toMatch(/\.stream-tool-run-toggle \{[^}]*border: 0;/);
    expect(styles).toMatch(/\.stream-tool-run-toggle \{[^}]*background: rgba\(255, 255, 255, 0\.08\);/);
    expect(styles).toMatch(/\.stream-tool-run-toggle \{[^}]*border-radius: 999px;/);
    expect(styles).toMatch(/\.stream-tool-run-toggle \{[^}]*padding: 2px 9px 2px 5px;/);
    expect(styles).not.toMatch(/\.stream-tool-run-toggle \{[^}]*border: 1px solid/);
  });
});

it("collapses tool rows continuously and never animates the viewport", () => {
  // 用户反馈：收起时界面上下跳——要的是连续性，不是瞬时位移。行用 220ms 平滑收拢，
  // 其他内容被连续推上去；内容（预览/命令缩略）等收拢结束再切换，否则过渡会从
  // 更矮的高度起步，等于没有动画。视口本身绝不能有过渡或平滑滚动。
  const rule = styles.match(/\.stream-tool-merged-item \{([^}]*)\}/)?.[1];
  expect(rule).toBeDefined();
  expect(rule).toContain("transition: grid-template-rows .22s");
  expect(source).toContain("const STREAM_COLLAPSE_MS = 220;");
  expect(source).toContain("setSettledAbsorbed(true), STREAM_COLLAPSE_MS");
  // 用户反馈：贴底跟随时收拢仍整页下移。旧方案（useStreamShrinkAnchor /
  // foldAnchorScrollTop 逐帧反向补 scrollTop）在贴底场景永远无效——浏览器先把
  // scrollTop 夹到新的 max，补偿已经晚了；两者连同测试一起删除。
  // 现契约：收拢开始前按整块当前高度在底部一次预留等高空槽，变化期间总高度不缩水，
  // 于是贴底状态下不会被夹、上方内容不动；新内容到达后逐帧吃掉空槽。
  expect(source).not.toContain("useStreamShrinkAnchor");
  expect(source).not.toContain("foldAnchorScrollTop");
  expect(source).toContain("function reserveStreamScrollGap(");
  expect(source).toContain("reserveStreamScrollGap(run.getBoundingClientRect().height, viewport);");
  expect(source).toContain("function consumeStreamScrollGap(");
  expect(source).toContain("streamScrollGapHost.style.height");
  expect(styles).toContain(".chat-scroll-shrink-gap {");
  // 跟随语义：只跟随「变高」，目标是最新内容底边（空槽不算内容）。
  expect(source).toContain("const target = streamContentScrollTop(viewport);");
  expect(source).toContain("if (target > viewport.scrollTop) viewport.scrollTop = target;");
  expect(styles).toContain(".stream-tool-count.incremented { animation:");
});

it("badge keeps one flat capsule: smaller font, sign merged into the pill background", () => {
  // 用户反馈：徽章字体再小一点；左侧 +/− 不要单独底色，并进胶囊底色。
  const toggle = styles.match(/\.stream-tool-run-toggle \{([^}]*)\}/)?.[1];
  expect(toggle).toBeDefined();
  expect(toggle).toContain("font-size: calc(var(--content-size) * .85);");
  const sign = styles.match(/\.stream-tool-run-sign \{([^}]*)\}/)?.[1];
  expect(sign).toBeDefined();
  expect(sign).not.toContain("background");
  expect(sign).not.toContain("border-radius");
  expect(styles).not.toContain(':root[data-theme="light"] .stream-tool-run-sign');
  // 计数跟随徽章字号，不再用独立的 content-size 比例。
  const count = styles.match(/\.stream-tool-count \{([^}]*)\}/)?.[1];
  expect(count).toBeDefined();
  expect(count).toContain("font-size: 1em;");
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

it("shows the folded turn change summary beside the archived frame", () => {
  const turnScope = source.slice(source.indexOf("const turnEditsSummary = useMemo("), source.indexOf("{persistentToolGenItems.length > 0 && ("));

  // Aggregation comes from the authoritative lifecycle items of the turn.
  expect(turnScope).toContain("summarizeTurnEdits(");
  expect(turnScope).toContain("lifecycleItems.map((item) => item.activity?.edits ?? undefined)");
  // The turn summary appears whenever the finished turn captured file edits,
  // independent of whether the process area is archived or kept expanded.
  expect(turnScope).toContain("showWorkFrame && !isWorking && turnEditsSummary.edits.length > 0 && (");
  expect(turnScope).not.toContain("!workStreamVisible && turnEditsSummary");
  expect(turnScope).toContain("turnEditsSummary.edits.length > 0 && (");
  expect(turnScope).toContain("<TurnEditsSummaryBox summary={turnEditsSummary} />");
  expect(styles).toContain(".turn-edits-summary");
  expect(styles).toContain(".turn-edits-summary-head");
  expect(styles).toContain(".file-edit-captures");
});

it("bundles IBM Plex Mono locally and scopes it to all tool rendering", () => {
  expect(styles).toContain('src: url("/fonts/IBMPlexMono-Latin-300.woff2") format("woff2")');
  expect(styles).toMatch(/\.stream-tool-count \{[^}]*font-family: "IBM Plex Mono"/);
  expect(styles).toMatch(/\.stream-tool-run,[\s\S]*\.tool-activity \{[\s\S]*font-family: "IBM Plex Mono"/);
  expect(styles).toContain('.stream-tool-head,\n.tool-activity,\n.tool-activity b { font-size: 12.5px; }');
  // 折叠徽章有自己更小的字号，不再继承工具行的 12.5px。
  expect(styles).toMatch(/\.stream-tool-run-toggle \{[^}]*font-size: calc\(var\(--content-size\) \* \.85\);/);
  // 计数按徽章字号缩放（1em），不再单独用 content-size 比例，避免比徽章还大。
  expect(styles).toMatch(/\.stream-tool-count \{[^}]*font-size: 1em; font-weight: 300;/);
  const font = readFileSync(new URL("../public/fonts/IBMPlexMono-Latin-300.woff2", import.meta.url));
  expect(font.readUInt32BE(0)).toBe(0x774f4632);
  expect(readFileSync(new URL("../public/fonts/IBMPlexMono-OFL.txt", import.meta.url), "utf8")).toContain("SIL OPEN FONT LICENSE");
});

it("uses the structured run_bash edit summary in ordinary and stream rows", () => {
  const icon = source.slice(source.indexOf("function ToolActivityIcon("), source.indexOf("const StreamToolRow ="));
  const stream = source.slice(source.indexOf("const StreamToolRow ="), source.indexOf("function TurnAnswerDelivery"));
  const ordinary = source.slice(source.indexOf("function ToolActivity("), source.indexOf("function MemoIcon("));
  expect(source).toContain('MemorySearchInvocation');
  expect(source).toContain('SelfToolIcon');
  expect(source).toContain('SelfToolInvocation');
  expect(icon).toContain('if (activity.run_bash_edit) return <RunBashEditIcon />');
  expect(stream).toContain('<RunBashEditInvocation edit={activity.run_bash_edit} />');
  expect(ordinary).toContain('<RunBashEditInvocation edit={activity.run_bash_edit} />');
  for (const section of [stream, ordinary])
    expect(section).toContain('<ToolActivityIcon activity={activity} toolName={toolName} failed={isToolActivityFailed(status)} />');
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
  const icon = source.slice(source.indexOf("function ToolActivityIcon("), source.indexOf("const StreamToolRow ="));
  const stream = source.slice(source.indexOf("const StreamToolRow ="), source.indexOf("function TurnAnswerDelivery"));
  const ordinary = source.slice(source.indexOf("function ToolActivity("), source.indexOf("function MemoIcon("));
  expect(icon).toContain('if (activity.memory_search) return <MemorySearchIcon />');
  expect(icon).toContain('if (activity.self_tool) return <SelfToolIcon />');
  for (const section of [stream, ordinary]) {
    expect(section).toContain('<MemorySearchInvocation search={activity.memory_search} />');
    expect(section).toContain('<SelfToolInvocation operation={activity.self_tool} />');
    expect(section).toContain('<ToolActivityIcon activity={activity} toolName={toolName} failed={isToolActivityFailed(status)} />');
  }
  expect(stream).toContain('const invocationPreview = !structuredInvocation ? toolInvocationPreview(activity) : undefined;');
  expect(ordinary).toContain('activity.tool_name === "sub_answer" || structuredInvocation ? undefined');
});

it("shows generic tool arguments as a collapsed preview or expanded detail, never both", () => {
  const stream = source.slice(source.indexOf("const StreamToolRow ="), source.indexOf("function TurnAnswerDelivery"));
  const ordinary = source.slice(source.indexOf("function ToolActivity("), source.indexOf("function MemoIcon("));

  expect(stream).toContain("const code = activity.code?.trim();");
  expect(stream).toContain("const invocationPreview = !structuredInvocation ? toolInvocationPreview(activity) : undefined;");
  expect(stream).toContain("const hasExpandableDetail = !!code || !!detail || !!activity.edits?.length;");
  // Edits auto-open is the documented exception: preview stays visible while
  // the fold shows only the diff panel, never the command detail.
  expect(stream).toContain(": (!open || editsAutoOpen) && invocationPreview && <span");
  expect(stream).toContain('{!editsAutoOpen && code && (');
  expect(stream).toContain('<div className="stream-tool-command-shell">');
  expect(stream).toContain('const commandCap = running || !settledAbsorbed ? COMMAND_PREVIEW_MAX_LINES : 1;');
  expect(stream).toContain('<pre className={`stream-tool-command${commandPreview?.clipped ? " stream-tool-command-clipped" : ""}`}>{commandPreview?.text ?? code}</pre>');
  expect(stream).toContain('{!editsAutoOpen && detail && <div className="stream-tool-detail">{detail}</div>}');
  expect(stream).not.toContain("const command =");
  expect(stream).not.toContain("activity.code?.trim() || (!structuredInvocation");

  expect(ordinary).toContain(": (!open || editsAutoOpen) && invocationPreview && (");
  expect(ordinary).toContain("{!editsAutoOpen && detail && (");
  expect(ordinary).toContain("{!editsAutoOpen && code && (");
  expect(ordinary).not.toContain(": invocationPreview && (");
});

it("gives expanded edit cards a dedicated command collapse back to the diff-only state", () => {
  const stream = source.slice(source.indexOf("const StreamToolRow ="), source.indexOf("function TurnAnswerDelivery"));
  const ordinary = source.slice(source.indexOf("function ToolActivity("), source.indexOf("function MemoIcon("));

  // Row click from the diff-only auto state expands the command view.
  expect(stream).toContain("if (editsAutoOpen) {");
  expect(ordinary).toContain("if (editsAutoOpen) {");
  // The expanded command block carries its own collapse button in both cards.
  // 流式卡片同时把这个入口交给 diff 缩略框：点缩略框回到 diff 视图，
  // 归档卡片（ordinary）仍只有命令区的收起按钮。
  expect(stream).toContain("const revealDiffOnly = () => {");
  expect(ordinary).toContain("const collapseCommand = () => {");
  expect(stream).toContain('className="stream-tool-code-collapse" onClick={revealDiffOnly}');
  expect(stream).toContain("onActivate={revealDiffOnly}");
  expect(ordinary).toContain('className="tool-activity-code-collapse" onClick={collapseCommand}');
  // The dedicated collapse keeps the fold open and restores the auto state.
  expect(stream).toContain("setManuallyToggled(false);");
  expect(ordinary).toContain("setManuallyToggled(false);");
  expect(styles).toContain(".stream-tool-command-shell");
  expect(styles).toContain(".tool-activity-code-shell");
  expect(styles).toContain(".stream-tool-code-collapse");
  expect(styles).toContain(".tool-activity-code-collapse");
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
  const compact = source.slice(source.indexOf("function ContextCompressNotice("), source.indexOf("function DecisionModal("));
  expect(activityView).toContain('system-notice reasoning-notice');
  expect(activityView).toContain('className="system-notice-title"');
  expect(memo).toContain('system-notice memo-notice');
  expect(memo).toContain('className="system-notice-detail system-notice-long-detail"');
  expect(compact.match(/system-notice context-compress-notice/g)).toHaveLength(2);
  const requestedCompact = compact.slice(compact.indexOf('compact_phase === "requested"'), compact.indexOf("const breakdown"));
  expect(requestedCompact).toContain('<strong className="system-notice-title">{t("context.compressing")}</strong>');
  expect(requestedCompact).not.toContain('t("context.dynamic")');
  expect(requestedCompact).not.toContain('className="system-notice-detail"');
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

it("renders a blocked task_finished summary as the same free-talk activity class before Continue memo", () => {
  const activityView = source.slice(source.indexOf("function ActivityView("), source.indexOf("function ToolGenNotice("));
  const eventAdapter = source.slice(source.indexOf("function activitiesFromTurnEvent("), source.indexOf("function activityFromTurnEvent("));
  const lifecycleItems = source.slice(source.indexOf("const lifecycleItems = useMemo("), source.indexOf("const supplementItems = useMemo("));
  expect(activityView).toContain('activity.kind === "free_talk" ? " free-talk" : ""');
  expect(eventAdapter).toContain("const activities = activitiesFromTopic(");
  expect(eventAdapter).toContain("return activities.map((activity, index) => ({");
  expect(lifecycleItems).toContain("lifecycleEvents.flatMap((event) =>");
  expect(lifecycleItems).toContain("activitiesFromTurnEvent(");
  expect(source).not.toContain("finish_summary");
  expect(source).not.toContain("memo-finish-summary");
  expect(styles).not.toContain("memo-finish-summary");
});

it("narrows tool rendering from the right edge while preserving mobile width", () => {
  expect(styles).toContain("width: 90%;\n  max-width: 90%;\n  margin-right: auto;");
  expect(styles).toMatch(/@media \(max-width: 720px\) \{[\s\S]*\.stream-tool-run,[\s\S]*width: 100%; max-width: 100%;/);
});

it("renders timer lifecycle rows as standalone system-status lines", () => {
  // Ordinary mode: timer activities bypass the collapsed tool group.
  expect(source).toMatch(/if \(activity\.timer\)\s*\n?\s*return \(\s*\n\s*<ToolActivity[\s\S]*?timer-row-/);
  // Grouping never absorbs timer rows in either mode.
  const grouping = readFileSync(new URL("../src/activity_groups.ts", import.meta.url), "utf8");
  expect(grouping).toContain('activity.tone === "action" && !activity.timer');
  expect(source).toContain('if (previous && foldable(entry) && foldable(previous[0])) previous.push(entry);');
  expect(source).toMatch(/if \(activity\.timer\)\s*\n?\s*return \(\s*\n\s*<div key=\{key\} className="stream-tool-merged-item">/);
  expect(styles).toContain(', .timer-tool-preview) { grid-column: 2; }');
  expect(styles).toContain('.timer-tool-icon { color: #8bc8ba; }');
});


it("renders pipe-delimited result size before elapsed in both tool views", () => {
  const stream = source.slice(source.indexOf("const StreamToolRow ="), source.indexOf("function TurnAnswerDelivery"));
  const ordinary = source.slice(source.indexOf("function ToolActivity("), source.indexOf("function MemoIcon("));
  expect(stream).toContain('className="stream-tool-meta"');
  expect(ordinary).toContain('className="tool-activity-meta"');
  expect(stream).toContain('className="stream-tool-sep" aria-hidden="true">|</span>');
  expect(ordinary).toContain('className="tool-activity-sep" aria-hidden="true">|</span>');
  const streamMeta = stream.indexOf('className="stream-tool-meta"');
  const streamBytes = stream.indexOf('className="stream-tool-bytes"');
  const streamElapsed = stream.indexOf('className="stream-tool-elapsed"');
  expect(streamMeta).toBeLessThan(streamBytes);
  expect(streamBytes).toBeLessThan(streamElapsed);
  const ordinaryMeta = ordinary.indexOf('className="tool-activity-meta"');
  const ordinaryBytes = ordinary.indexOf('className="tool-activity-bytes"');
  const ordinaryDuration = ordinary.indexOf('className="tool-activity-duration"');
  expect(ordinaryMeta).toBeLessThan(ordinaryBytes);
  expect(ordinaryBytes).toBeLessThan(ordinaryDuration);
  expect(source).toContain("function formatResultBytes(");
  expect(source).toContain("return `${bytes}B`;");
  expect(source).toContain("return `${Math.ceil(bytes / 1024)}KB`;");
  expect(styles).toContain(".stream-tool-meta {");
  expect(styles).toContain(".tool-activity-meta,");
  expect(styles).toContain(".tool-activity-duration-slot { display: none; }");
  expect(styles).toContain(".tool-activity summary .tool-activity-sep { display: none; }");
});

describe("command multi-line preview one-way shrink", () => {
  const helper = source.slice(source.indexOf("const COMMAND_PREVIEW_MAX_LINES"), source.indexOf("function activitiesFromTurnEvent("));
  it("caps live commands at five lines with a trailing ellipsis and normalizes newlines", () => {
    expect(helper).toContain("const COMMAND_PREVIEW_MAX_LINES = 5;");
    expect(helper).toContain("code.replace(/\\r\\n?/g, \"\\n\").split(\"\\n\")");
    expect(helper).toContain('[...lines.slice(0, maxLines), "…"].join("\\n")');
    expect(helper).toContain("clipped: true");
    expect(helper).toContain("function commandHasMoreLines(activity: Activity): boolean");
  });
  it("stream rows shrink permanently once absorbed and archive rows settle single-line", () => {
    const stream = source.slice(source.indexOf("const StreamToolRow ="), source.indexOf("function TurnAnswerDelivery"));
    const ordinary = source.slice(source.indexOf("function ToolActivity("), source.indexOf("function MemoIcon("));
    expect(stream).toContain("absorbed?: boolean");
    expect(stream).toContain("const commandCap = running || !settledAbsorbed ? COMMAND_PREVIEW_MAX_LINES : 1;");
    expect(stream).toContain("if (previous.absorbed !== next.absorbed) return false;");
    expect(source).toContain("absorbed={!isToolActivityRunning(activity.tool_status || TOOL_STATUS_RUNNING) && (superseded || handoffIds.has(activity.id))}");
    expect(ordinary).toContain("const commandCap = running ? COMMAND_PREVIEW_MAX_LINES : 1;");
    expect(ordinary).not.toContain("fencedCode");
  });
  it("head previews hint multi-line commands and clipped bodies fade at the bottom", () => {
    expect(source).toContain('{invocationPreview.replace(/\\s+/g, " ")}{multilineCommand ? " …" : ""}</span>');
    expect(source).toContain('{invocationPreview}{multilineCommand ? " …" : ""}');
    expect(styles).toMatch(/\.stream-tool-command \{[^}]*position: relative;/s);
    expect(styles).toContain(".stream-tool-command-clipped::after");
    expect(styles).toContain('linear-gradient(180deg, transparent, #262626)');
    expect(styles).toContain(':root[data-theme="light"] .stream-tool-command-clipped::after');
  });
});
