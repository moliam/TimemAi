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
import { FileEditsChip, FileEditsPanel } from "../src/file_edits_view";

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
  it("opens diff sections and tool cards by default when edits exist", () => {
    expect(component).toContain("useState(() => !!edit.diff)");
    expect(main).toContain("const hasEdits = !!activity.edits?.length;");
    expect(main).toContain("const [expanded, setExpanded] = useState(() => hasEdits)");
    expect(main).toContain("if (hasEdits) setExpanded(true);");
    expect(main).toContain("const [open, setOpen] = useState(() => hasEdits)");
    expect(main).toContain("if (hasEdits) setOpen(true);");
    // 自动展开只呈现 diff 面板：命令保持单行缩略，完整代码/输出由手动展开控制。
    expect(main).toContain("const editsAutoOpen = hasEdits && !manuallyToggled;");
    expect(main).toContain('{!editsAutoOpen && code && <pre className="stream-tool-command">{code}</pre>}');
    expect(main).toContain("(!open || editsAutoOpen) && invocationPreview");
    // 带修改证据的行豁免折叠组合并，diff 卡片始终完整可见。
    expect(main).toContain("const mergeableCount = completed.filter(activity => !activity.edits?.length).length;");
    expect(main).toContain("const merged = mergeableCount > 0 && !expanded && !interactionHeld;");
  });
});
