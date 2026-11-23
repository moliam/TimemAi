import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { fileEditsPresentation, parseDiffRows } from "../src/tool_presentation";

const main = readFileSync(new URL("../src/main.tsx", import.meta.url), "utf8");
const styles = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8");
const component = readFileSync(new URL("../src/file_edits_view.tsx", import.meta.url), "utf8");
const en = readFileSync(new URL("../src/i18n/strings.en.ts", import.meta.url), "utf8");
const zh = readFileSync(new URL("../src/i18n/strings.zh.ts", import.meta.url), "utf8");

const validEntry = {
  path: "src/main.rs",
  op: "modified",
  diff: "--- a/src/main.rs\n+++ b/src/main.rs\n@@ -1,3 +1,3 @@\n a\n-old\n+new\n c\n",
  added_lines: 1,
  removed_lines: 1,
  truncated: false,
  skip_reason: null,
};

const entry = (over: Partial<typeof validEntry>) => ({
  ...validEntry,
  ...over,
});

describe("fileEditsPresentation", () => {
  it("parses a valid edits payload", () => {
    const parsed = fileEditsPresentation([validEntry]);
    expect(parsed).toHaveLength(1);
    expect(parsed![0]).toEqual(validEntry);
  });

  it("accepts skipped entries without diff text", () => {
    const parsed = fileEditsPresentation([
      { path: "blob.bin", op: "skipped", diff: null, added_lines: 0, removed_lines: 0, truncated: false, skip_reason: "binary" },
    ]);
    expect(parsed![0].skip_reason).toBe("binary");
    expect(parsed![0].diff).toBeNull();
  });

  it("parses missing-skip entries and hides capture-side no-change cards", () => {
    const skipped = (reason: string, index: number) => ({
      path: `f-${index}-${reason}.txt`,
      op: "skipped",
      diff: null,
      added_lines: 0,
      removed_lines: 0,
      truncated: false,
      skip_reason: reason,
    });
    // The strict whitelist must accept "missing" (an unknown reason would
    // reject the whole payload), and the card itself stays hidden.
    const parsed = fileEditsPresentation([skipped("missing", 0), validEntry]);
    expect(parsed).toHaveLength(1);
    expect(parsed![0].op).toBe("modified");

    const filtered = fileEditsPresentation([
      { path: "same.txt", op: "unchanged", diff: null, added_lines: 0, removed_lines: 0, truncated: false, skip_reason: null },
      skipped("missing", 1),
      skipped("unreadable", 2),
      skipped("not_captured", 3),
      skipped("budget_exhausted", 4),
      skipped("binary", 5),
      skipped("too_large", 6),
      validEntry,
    ]);
    expect(filtered!.map(item => item.skip_reason ?? item.op)).toEqual([
      "binary",
      "too_large",
      "modified",
    ]);
  });

  it("rejects malformed payloads", () => {
    expect(fileEditsPresentation(undefined)).toBeUndefined();
    expect(fileEditsPresentation([])).toBeUndefined();
    expect(fileEditsPresentation({})).toBeUndefined();
    expect(fileEditsPresentation([{ ...validEntry, op: "renamed" }])).toBeUndefined();
    expect(fileEditsPresentation([{ ...validEntry, skip_reason: "suspicious" }])).toBeUndefined();
    expect(fileEditsPresentation([{ ...validEntry, added_lines: -1 }])).toBeUndefined();
    expect(fileEditsPresentation([{ ...validEntry, diff: "x".repeat(40_001) }])).toBeUndefined();
    expect(fileEditsPresentation([{ ...validEntry, extra: true }])).toBeUndefined();
    expect(fileEditsPresentation(Array.from({ length: 17 }, () => validEntry))).toBeUndefined();
  });
});

describe("parseDiffRows", () => {
  it("tracks line numbers across hunks and tags rows", () => {
    const rows = parseDiffRows(
      "--- a/f.txt\n+++ b/f.txt\n@@ -1,2 +1,2 @@\n a\n-b\n+c\n@@ -9,1 +9,1 @@\n x",
    );
    expect(rows.map(row => row.kind)).toEqual(["meta", "meta", "hunk", "ctx", "del", "add", "hunk", "ctx"]);
    expect(rows[3]).toMatchObject({ oldNo: 1, newNo: 1 });
    expect(rows[4]).toMatchObject({ oldNo: 2 });
    expect(rows[5]).toMatchObject({ newNo: 2 });
    expect(rows[6].text).toContain("@@ -9,1 +9,1 @@");
    expect(rows[7]).toMatchObject({ oldNo: 9, newNo: 9 });
  });

  it("degrades unknown and no-newline markers to meta rows", () => {
    const rows = parseDiffRows("@@ -0,0 +1,1 @@\n+hi\n\\ No newline at end of file\n");
    expect(rows.map(row => row.kind)).toEqual(["hunk", "add", "meta"]);
    expect(rows[2].text).toContain("No newline");
  });
});

describe("file edits surfaces", () => {
  it("renders the panel and chip in both tool card variants", () => {
    expect(main.match(/<FileEditsPanel edits=\{activity\.edits\} \/>/g)).toHaveLength(2);
    expect(main.match(/<FileEditsChip edits=\{activity\.edits\} \/>/g)).toHaveLength(2);
    expect(main).toContain("!!code || !!detail || !!activity.edits?.length");
    expect(main).toContain("!!detail || !!code || !!activity.edits?.length");
    expect(main).toContain("JSON.stringify(a.edits) === JSON.stringify(b.edits)");
  });

  it("styles both themes and keeps the guard classes", () => {
    for (const selector of [
      ".stream-tool-edits-chip",
      ".file-edit-section",
      ".file-edit-row-add",
      ".file-edit-row-del",
      ".file-edit-row-hunk",
      ":root[data-theme=\"light\"] .file-edit-row-add",
      ":root[data-theme=\"light\"] .file-edit-row-del",
    ]) {
      expect(styles).toContain(selector);
    }
    // 行号列按内容自适应，长行号不换行、不因窄容器折叠。
    expect(styles).toContain("grid-template-columns: max-content max-content 1.4ch minmax(0, 1fr)");
    const gutterStart = styles.indexOf(".file-edit-gutter {");
    const gutterEnd = styles.indexOf("}", gutterStart);
    const gutter = styles.slice(gutterStart, gutterEnd + 1);
    expect(gutter).toContain("min-width: max-content;");
    expect(gutter).toContain("white-space: nowrap;");
  });

  it("keeps the panel as the single framed box and counts nowhere but the header chip", () => {
    // 用户反馈：文件名列表不渲染为框，仅展开的 diff 正文保留内层框。
    // 面板内也不再渲染第二份「N 个文件 +x −y」：卡片头部 chip 已是唯一计数处，
    // 面板再报一遍会让同一组数字在一张卡里出现三次。
    expect(component).not.toContain('className="file-edits-panel-head"');
    expect(component).not.toContain('className="file-edits-panel-totals"');
    expect(component).not.toContain('t("tools.fileEditsPanelCount"');
    expect(en).not.toContain("fileEditsPanelCount:");
    expect(zh).not.toContain("fileEditsPanelCount:");
    const sectionStart = styles.indexOf(".file-edit-section {");
    const section = styles.slice(sectionStart, styles.indexOf("}", sectionStart) + 1);
    expect(section).not.toContain("border:");
    expect(section).not.toContain("background:");
    const opStart = styles.indexOf(".file-edit-op {");
    const op = styles.slice(opStart, styles.indexOf("}", opStart) + 1);
    expect(op).not.toContain("background");
    const diffStart = styles.indexOf(".file-edit-diff {");
    const diff = styles.slice(diffStart, styles.indexOf("}", diffStart) + 1);
    // 用户反馈：diff 框与文本同宽——内层框取消，滚动行为保留。
    expect(diff).not.toContain("border:");
    expect(diff).not.toContain("background:");
    expect(diff).not.toContain("border-radius");
    expect(diff).toContain("overflow: auto;");
    expect(diff).toContain("max-height: 420px;");
    const embeddedStart = styles.indexOf(".turn-edits-summary .file-edits-panel {");
    const embedded = styles.slice(embeddedStart, styles.indexOf("}", embeddedStart) + 1);
    expect(embedded).toContain("border: none;");
    expect(embedded).toContain("padding: 0;");
  });

  it("renders no second head or summed totals inside the panel", () => {
    const html = renderToString(createElement(FileEditsPanel, {
      edits: [
        { ...validEntry },
        { ...validEntry, path: "b.rs", added_lines: 2, removed_lines: 0 },
      ],
    })).replace(/<!-- -->/g, "");
    expect(html).not.toContain("file-edits-panel-head");
    expect(html).not.toContain("file-edits-panel-totals");
    // 每行自己的 +/- 保留：per-file 数字不是重复，是定位到哪个文件。
    expect(html).toContain("file-edit-additions");
    expect(html).toContain("file-edit-deletions");
  });

  it("keeps the diff tint across the full horizontal scroll range", () => {
    const start = styles.indexOf(".file-edit-diff-table {");
    const block = styles.slice(start, styles.indexOf("}", start) + 1);
    expect(block).toContain("width: max-content;");
    expect(block).toContain("min-width: 100%;");
  });

  it("keeps i18n catalogs in sync for both locales", () => {
    const keys = [
      "fileEditsChip",
      "fileEditsAria",
      "fileOpCreated",
      "fileOpModified",
      "fileOpDeleted",
      "fileOpUnchanged",
      "fileOpSkipped",
      "fileDiffOmitted",
      "copyDiff",
      "diffCopied",
      "copyFailed",
      "fileSkipBinary",
      "fileSkipTooLarge",
      "fileSkipUnreadable",
      "fileSkipNotCaptured",
      "fileSkipBudget",
      "collapseCommand",
      "turnEditsTitle",
      "turnEditsOverflow",
      "captureCount",
    ];
    for (const key of keys) {
      expect(en).toContain(`${key}:`);
      expect(zh).toContain(`${key}:`);
    }
    expect(component).not.toMatch(/[\u4e00-\u9fff]/);
  });
});

import { createElement } from "react";
import { renderToString } from "react-dom/server";
import { FileEditsChip, FileEditsPanel, TurnEditsSummaryBox, summarizeTurnEdits } from "../src/file_edits_view";

describe("file edits components render server-side", () => {
  it("chip summarizes count and stats", () => {
    const html = renderToString(createElement(FileEditsChip, {
      edits: [{ ...validEntry }, { ...validEntry, path: "b.txt", added_lines: 2, removed_lines: 3 }],
    }));
    expect(html).toContain("stream-tool-edits-chip");
    expect(html).toContain("2");
    expect(html).toContain("+3");
    expect(html).toContain("\u22124");
  });

  it("shows unchanged row line numbers once instead of one per side", () => {
    const diff = "--- a/f.txt\n+++ b/f.txt\n@@ -1,2 +1,2 @@\n mid\n+new\n";
    const html = renderToString(createElement(FileEditsPanel, {
      edits: [{ ...validEntry, path: "f.txt", diff }],
    }));
    expect(html.match(/file-edit-gutter">1<\/span>/g)).toHaveLength(1);
    expect(html.match(/file-edit-gutter">2<\/span>/g)).toHaveLength(1);
  });

  it("renders all captured diff rows and marks runtime truncation as omitted", () => {
    const rows = Array.from({ length: 24 }, (_, index) => ` kept-${index}`);
    const diff = `--- a/long.txt\n+++ b/long.txt\n@@ -1,24 +1,24 @@\n${rows.join("\n")}\n`;
    const html = renderToString(createElement(FileEditsPanel, {
      edits: [{ ...validEntry, path: "long.txt", diff, truncated: true }],
    }));
    expect(html).toContain("file-edit-row-ctx");
    expect(html).toContain("kept-0");
    expect(html).toContain("kept-23");
    expect(html).toContain("file-edit-notice");
    expect(html).toContain("\u3010\u5df2\u7701\u7565\u3011");
  });

  it("marks payload-slimmed edits even when no diff body remains", () => {
    const html = renderToString(createElement(FileEditsPanel, {
      edits: [{ ...validEntry, diff: null, truncated: true }],
    }));
    expect(html).not.toContain("file-edit-diff-table");
    expect(html).toContain("file-edit-notice");
    expect(html).toContain("\u3010\u5df2\u7701\u7565\u3011");
  });

  it("panel default-expands diff sections and keeps skipped notice-only", () => {
    const html = renderToString(createElement(FileEditsPanel, {
      edits: [
        { ...validEntry },
        { path: "blob.bin", op: "skipped", diff: null, added_lines: 0, removed_lines: 0, truncated: false, skip_reason: "binary" },
      ],
    }));
    expect(html).toContain("file-edits-panel");
    expect(html).toContain("file-edit-op-modified");
    expect(html).toContain("file-edit-op-skipped");
    expect(html).toContain("main.rs");
    // 用户反馈：有 diff 的文件段默认直接展开，diff 正文随首帧渲染。
    expect(html).toContain(`aria-expanded="true"`);
    expect(html).toContain("file-edit-row-add");
    expect(html).toContain("file-edit-row-del");
    expect(html).toContain(">old</span>");
    expect(html).toContain(">new</span>");
    // 无 diff 的 skipped 段没有可展开正文，仅显示原因提示。
    expect(html).toContain("file-edit-notice");
  });
});

describe("file edits auto-expand contract", () => {
  it("folds diff sections under auto-collapse while tool cards still auto-open", () => {
    expect(component).toContain("useState(() => !!edit.diff && !autoCollapse)");
    expect(main).toContain("const hasEdits = !!activity.edits?.length;");
    expect(main).toContain("const [expanded, setExpanded] = useState(() => hasEdits)");
    expect(main).toContain("if (hasEdits) setExpanded(true);");
    expect(main).toContain("const [open, setOpen] = useState(() => hasEdits)");
    expect(main).toContain("if (hasEdits) setOpen(true);");
    // 自动展开只呈现 diff 面板：命令保持单行缩略，完整代码/输出由手动展开控制。
    expect(main).toContain("const editsAutoOpen = hasEdits && !manuallyToggled;");
    expect(main).toContain('<div className="stream-tool-command-shell">');
    expect(main).toContain('<pre className={`stream-tool-command${commandPreview?.clipped ? " stream-tool-command-clipped" : ""}`}>{commandPreview?.text ?? code}</pre>');
    expect(main).toContain("(!open || editsAutoOpen) && invocationPreview");
    // 带修改证据的行豁免折叠组合并，diff 卡片始终完整可见。
    expect(main).toContain("const mergeableCount = completed.filter(activity => !activity.edits?.length).length;");
    expect(main).toContain("const merged = mergeableCount > 0 && !expanded && !interactionHeld;");
  });
});

describe("summarizeTurnEdits", () => {
  it("merges repeated paths keeping first-touch order and the final capture", () => {
    const result = summarizeTurnEdits([
      [entry({ path: "a.txt", added_lines: 1, removed_lines: 0 })],
      [entry({ path: "b.txt", op: "created" })],
      [entry({ path: "a.txt", added_lines: 5, removed_lines: 2 })],
    ]);
    expect(result.edits.map((edit) => edit.path)).toEqual(["a.txt", "b.txt"]);
    expect(result.edits[0]).toMatchObject({ added_lines: 5, removed_lines: 2, capture_count: 2 });
    expect(result.edits[1]).toMatchObject({ op: "created", capture_count: 1 });
    expect(result.overflowCount).toBe(0);
  });

  it("marks a path created during the turn as created even after modifications", () => {
    const result = summarizeTurnEdits([
      [entry({ path: "new.txt", op: "created" })],
      [entry({ path: "new.txt", op: "modified" })],
    ]);
    expect(result.edits[0]).toMatchObject({ op: "created", capture_count: 2 });
  });

  it("drops a path created mid-turn and deleted before the end", () => {
    const result = summarizeTurnEdits([
      [entry({ path: "gone.txt", op: "created" })],
      [entry({ path: "gone.txt", op: "deleted", diff: null, added_lines: 0, removed_lines: 4 })],
    ]);
    expect(result.edits).toEqual([]);
    expect(result.overflowCount).toBe(0);
  });

  it("reports a real deletion of a pre-existing file", () => {
    const result = summarizeTurnEdits([
      [entry({ path: "kept.txt" })],
      [entry({ path: "kept.txt", op: "deleted", diff: null, added_lines: 0, removed_lines: 3 })],
    ]);
    expect(result.edits[0]).toMatchObject({ op: "deleted", removed_lines: 3, capture_count: 2 });
  });

  it("folds a deletion followed by a recreation into a modification", () => {
    const result = summarizeTurnEdits([
      [entry({ path: "b.txt" })],
      [entry({ path: "b.txt", op: "deleted", diff: null, added_lines: 0, removed_lines: 2 })],
      [entry({ path: "b.txt", op: "created" })],
    ]);
    expect(result.edits[0]).toMatchObject({ op: "modified", capture_count: 3 });
  });

  it("keeps a creation when the file is deleted and created again mid-turn", () => {
    const result = summarizeTurnEdits([
      [entry({ path: "c.txt", op: "created" })],
      [entry({ path: "c.txt", op: "deleted", diff: null, added_lines: 0, removed_lines: 1 })],
      [entry({ path: "c.txt", op: "created" })],
    ]);
    expect(result.edits[0]).toMatchObject({ op: "created", capture_count: 3 });
  });

  it("still drops a vanished mid-turn creation when a later capture skips", () => {
    const result = summarizeTurnEdits([
      [entry({ path: "d.txt", op: "created" })],
      [entry({ path: "d.txt", op: "deleted", diff: null, added_lines: 0, removed_lines: 1 })],
      [
        entry({
          path: "d.txt",
          op: "skipped",
          skip_reason: "too_large",
          diff: null,
          added_lines: 0,
          removed_lines: 0,
        }),
      ],
    ]);
    expect(result.edits).toEqual([]);
  });

  it("keeps the last captured diff when a later capture is skipped", () => {
    const result = summarizeTurnEdits([
      [entry({ path: "keep.txt", diff: "--- a\n+++ b\n@@\n+x\n" })],
      [entry({ path: "keep.txt", op: "skipped", skip_reason: "binary", diff: null, added_lines: 0, removed_lines: 0 })],
    ]);
    expect(result.edits[0]).toMatchObject({ op: "modified", skip_reason: null });
    expect(result.edits[0].diff).toContain("+x");
    expect(result.edits[0].capture_count).toBe(2);
  });

  it("stays skipped when every capture is skipped", () => {
    const result = summarizeTurnEdits([
      [entry({ path: "bin.dat", op: "skipped", skip_reason: "binary", diff: null })],
      [entry({ path: "bin.dat", op: "skipped", skip_reason: "unreadable", diff: null })],
    ]);
    expect(result.edits[0]).toMatchObject({ op: "skipped", skip_reason: "unreadable", capture_count: 2 });
  });

  it("ignores malformed groups and entries", () => {
    const result = summarizeTurnEdits([
      undefined,
      null,
      "nope",
      [null, undefined, 3],
      [entry({ path: "" })],
      [entry({ path: "ok.txt" })],
    ] as never);
    expect(result.edits).toHaveLength(1);
    expect(result.edits[0]).toMatchObject({ path: "ok.txt", capture_count: 1 });
  });

  it("caps merged entries and reports the overflow count", () => {
    const groups = Array.from({ length: 70 }, (_, index) => [
      entry({ path: `f${index}.txt` }),
    ]);
    const result = summarizeTurnEdits(groups);
    expect(result.edits).toHaveLength(64);
    expect(result.overflowCount).toBe(6);
    expect(result.edits.at(-1)).toMatchObject({ path: "f63.txt" });
  });

  it("accepts a missing or empty input", () => {
    expect(summarizeTurnEdits(undefined)).toEqual({ edits: [], overflowCount: 0 });
    expect(summarizeTurnEdits([])).toEqual({ edits: [], overflowCount: 0 });
  });
});

describe("turn edits summary box renders server-side", () => {
  it("renders the title, chip, capture badge and overflow note", () => {
    const summary = summarizeTurnEdits([
      [entry({ path: "src/lib.rs", added_lines: 3, removed_lines: 1 })],
      [entry({ path: "src/lib.rs", added_lines: 6, removed_lines: 2 })],
      [entry({ path: "docs/x.md", op: "created" })],
    ]);
    const html = renderToString(createElement(TurnEditsSummaryBox, { summary })).replace(/<!-- -->/g, "");
    expect(html).toContain("turn-edits-summary");
    expect(html).toContain("file-edit-captures");
    expect(html).toContain("×2");
    expect(html).toContain("+6");
    expect(html).not.toContain("file-edit-notice");
  });

  it("reports the bounded overflow when the summary exceeds its budget", () => {
    const groups = Array.from({ length: 66 }, (_, index) => [
      entry({ path: `f${index}.txt` }),
    ]);
    const html = renderToString(
      createElement(TurnEditsSummaryBox, { summary: summarizeTurnEdits(groups) }),
    );
    expect(html).toContain("file-edit-notice");
  });
});

describe("file edits panel numbering", () => {
  it("labels each section head with its 1-based index before the op badge", () => {
    expect(component).toContain('<FileEditSection key={`${edit.path}:${index}`} edit={edit} index={index} />');
    expect(component).toContain("edit: TurnEditEntry; index: number");
    const headIndex = component.indexOf('<span className="file-edit-index">{index + 1}.</span>');
    const opIndex = component.indexOf("file-edit-op file-edit-op-");
    expect(headIndex).toBeGreaterThan(-1);
    expect(headIndex).toBeLessThan(opIndex);
    expect(styles).toContain(".file-edit-index { flex: none; min-width: 1.2em; text-align: right;");
    expect(styles).toContain(':root[data-theme="light"] .file-edit-index');
  });
});
