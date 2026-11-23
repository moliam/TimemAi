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

  it("drops legacy skipped entries entirely", () => {
    // The runtime no longer emits "skipped": a non-text capture arrives as
    // modified/created/deleted with a kind tag. Old stored payloads keep the
    // marker, and those rows must not surface as a "跳过" card.
    const parsed = fileEditsPresentation([
      { path: "blob.bin", op: "skipped", diff: null, added_lines: 0, removed_lines: 0, truncated: false, skip_reason: "binary" },
    ]);
    expect(parsed).toEqual([]);
  });

  it("keeps real edits and drops no-op rows", () => {
    const tagged = (reason: string, path: string) => ({
      path,
      op: "modified",
      diff: null,
      added_lines: 0,
      removed_lines: 0,
      truncated: false,
      skip_reason: reason,
    });
    const filtered = fileEditsPresentation([
      { path: "same.txt", op: "unchanged", diff: null, added_lines: 0, removed_lines: 0, truncated: false, skip_reason: null },
      { path: "legacy.txt", op: "skipped", diff: null, added_lines: 0, removed_lines: 0, truncated: false, skip_reason: "missing" },
      tagged("binary", "target/debug/timem"),
      tagged("directory", "target/"),
      validEntry,
    ]);
    expect(filtered!.map(item => item.skip_reason ?? item.op)).toEqual([
      "binary",
      "directory",
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
    // 用户反馈：点 diff 缩略框要展开 diff——流式行里的缩略框自己是控件，
    // 归档行保持纯静态摘要。
    expect(main.match(/<FileEditsChip edits=\{activity\.edits\} onActivate=\{revealDiffOnly\} \/>/g)).toHaveLength(1);
    expect(main.match(/<FileEditsChip edits=\{activity\.edits\} \/>/g)).toHaveLength(1);
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

  it("keeps the panel frameless with a fill only and counts nowhere but the header chip", () => {
    // 用户反馈：文件名列表不渲染为框，仅展开的 diff 正文保留内层框。
    // 面板内也不再渲染第二份「N 个文件 +x −y」：卡片头部 chip 已是唯一计数处，
    // 面板再报一遍会让同一组数字在一张卡里出现三次。
    expect(component).not.toContain('className="file-edits-panel-head"');
    expect(component).not.toContain('className="file-edits-panel-totals"');
    expect(component).not.toContain('t("tools.fileEditsPanelCount"');
    expect(en).not.toContain("fileEditsPanelCount:");
    expect(zh).not.toContain("fileEditsPanelCount:");
    const panelStart = styles.indexOf(".file-edits-panel {");
    const panel = styles.slice(panelStart, styles.indexOf("}", panelStart) + 1);
    // 用户反馈：一行文件摘要外面不要边框，只留底色。
    expect(panel).toContain("border: 0;");
    expect(panel).toContain("background:");
    // 用户反馈：文件名行的「修改」贴到卡片最左边了。面板左右内边距归零换取与正文
    // 同宽后，头部行自己内缩（diff 行仍铺满），两个容器都要遵守。
    expect(styles).toMatch(/\.stream-tool-row \.file-edit-head,\s*\.turn-edits-summary \.file-edit-head \{\s*padding-left: 10px;\s*padding-right: 10px;/);
    const lightPanelStart = styles.indexOf(':root[data-theme="light"] .file-edits-panel {');
    const lightPanel = styles.slice(lightPanelStart, styles.indexOf("}", lightPanelStart) + 1);
    expect(lightPanel).not.toContain("border");
    const sectionStart = styles.indexOf(".file-edit-section {");
    const section = styles.slice(sectionStart, styles.indexOf("}", sectionStart) + 1);
    expect(section).not.toContain("border:");
    expect(section).not.toContain("background:");
    // 用户反馈：新建 / 修改 / 删除用不同颜色的无框胶囊；底色要浅，别压得那么重。
    // 淡底 + 同色系浅字（对比度 7:1 上下）：新建蓝、修改取 diff 里 + 行的绿、删除红。
    const opStart = styles.indexOf(".file-edit-op {");
    const op = styles.slice(opStart, styles.indexOf("}", opStart) + 1);
    expect(op).toContain("border: 0;");
    expect(op).toContain("border-radius: 999px;");
    expect(op).toContain("padding: 0 7px;");
    // 不再是白字实底：底色浅了以后白字会读不清，文字色改由各 op 自己声明。
    expect(op).not.toContain("color: #fff");
    expect(op).not.toContain("border: 1px");
    for (const [name, tint, ink] of [
      ["created", "rgba(56, 132, 208, 0.20)", "#8ec4ef"],
      ["modified", "rgba(64, 180, 120, 0.18)", "#8fdcac"],
      ["deleted", "rgba(214, 92, 84, 0.20)", "#f0a49d"],
    ] as const) {
      const start = styles.indexOf(`.file-edit-op-${name} {`);
      const block = styles.slice(start, styles.indexOf("}", start) + 1);
      expect(block).toContain(`background: ${tint};`);
      expect(block).toContain(`color: ${ink};`);
    }
    // 明色主题：同一套淡底再浅一档，文字用同色系深色（对比度 6.2~6.9:1）。
    for (const [name, tint, ink] of [
      ["created", "#e6f0fb", "#1b5a92"],
      ["modified", "#e6f4ec", "#14603f"],
      ["deleted", "#fce9e7", "#96271f"],
    ] as const) {
      const start = styles.indexOf(`:root[data-theme="light"] .file-edit-op-${name} {`);
      const block = styles.slice(start, styles.indexOf("}", start) + 1);
      expect(block).toContain(`background: ${tint};`);
      expect(block).toContain(`color: ${ink};`);
      expect(block).not.toContain("border");
    }
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
      "fileKindBinary",
      "fileKindDirectory",
      "fileKindTooLarge",
      "fileKindUnreadable",
      "fileKindNoBaseline",
      "fileKindDiffOmitted",
      "collapseCommand",
      "turnEditsTitle",
      "turnEditsOverflow",
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
import { FileEditsChip, FileEditsPanel, TurnEditsSummaryBox, turnFileChangesSummary } from "../src/file_edits_view";

describe("file edits components render server-side", () => {
  it("chip summarizes count and stats", () => {
    const html = renderToString(createElement(FileEditsChip, {
      edits: [{ ...validEntry }, { ...validEntry, path: "b.txt", added_lines: 2, removed_lines: 3 }],
    }));
    expect(html).toContain("stream-tool-edits-chip");
    expect(html).toContain("2");
    expect(html).toContain("+3");
    expect(html).toContain("\u22124");
    // 静态摘要（归档视图）不是按钮，不会吞掉行头的展开行为。
    expect(html).not.toContain('role="button"');
  });

  it("chip is its own activator when it can reveal the diff", () => {
    const html = renderToString(createElement(FileEditsChip, {
      edits: [{ ...validEntry }],
      onActivate: () => {},
    }));
    expect(html).toContain("stream-tool-edits-chip-action");
    expect(html).toContain('role="button"');
    expect(html).toContain("tabindex=\"0\"");
  });

  it("hides the file ordinal when the panel lists a single file", () => {
    // 用户反馈：只有一个文件时不要「1.」这种序号，多文件才需要序号定位。
    const single = renderToString(createElement(FileEditsPanel, {
      edits: [{ ...validEntry, path: "only.txt" }],
    }));
    expect(single).not.toContain("file-edit-index");
    const multi = renderToString(createElement(FileEditsPanel, {
      edits: [{ ...validEntry, path: "a.txt" }, { ...validEntry, path: "b.txt" }],
    }));
    expect(multi.match(/file-edit-index/g)).toHaveLength(2);
    expect(multi).toMatch(/file-edit-index">1</);
    expect(multi).toMatch(/file-edit-index">2</);
  });

  it("renders the diff copy control as an icon with an accessible label", () => {
    // 用户反馈：把文字「复制差异」换成小图标，diff 行头保持单行紧凑。
    const html = renderToString(createElement(FileEditsPanel, {
      edits: [{ ...validEntry, path: "f.txt" }],
    }));
    const start = html.indexOf('class="file-edit-copy');
    expect(start).toBeGreaterThan(-1);
    const control = html.slice(start, html.indexOf("</span>", start));
    expect(control).toContain('aria-label="复制差异"');
    expect(control).toContain("<svg");
    expect(control).not.toContain("复制差异<");
    expect(styles).toMatch(/\.file-edit-copy \{[^}]*width: 18px;[^}]*height: 18px;/s);
    expect(styles).not.toMatch(/\.file-edit-copy \{[^}]*font-size:/s);
    // 用户反馈：小图标不要边框，只留字形 + 悬停底色。
    expect(styles).toMatch(/\.file-edit-copy \{[^}]*border: 0;/s);
    expect(styles).not.toContain(':root[data-theme="light"] .file-edit-copy { border-color');
    expect(styles).not.toMatch(/\.file-edit-copy \{[^}]*border: 1px solid/s);
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

  it("labels payload-slimmed edits with the omitted tag, not a truncation notice", () => {
    const html = renderToString(createElement(FileEditsPanel, {
      edits: [{
        ...validEntry,
        op: "deleted",
        diff: null,
        added_lines: 0,
        removed_lines: 1,
        truncated: true,
        skip_reason: "budget_exhausted",
      }],
    }));
    expect(html).not.toContain("file-edit-diff-table");
    // 用户反馈：只删一行却写「超出限制、已截断」自相矛盾——预算省略只用类型标签说明。
    expect(html).toContain("file-edit-kind");
    expect(html).not.toContain("file-edit-notice");
  });

  it("panel default-expands diff sections and tags non-text sections", () => {
    const html = renderToString(createElement(FileEditsPanel, {
      edits: [
        { ...validEntry },
        { path: "blob.bin", op: "modified", diff: null, added_lines: 0, removed_lines: 0, truncated: false, skip_reason: "binary", size_bytes: 4096 },
      ],
    }));
    expect(html).toContain("file-edits-panel");
    expect(html).toContain("file-edit-op-modified");
    expect(html).toContain("main.rs");
    // 用户反馈：有 diff 的文件段默认直接展开，diff 正文随首帧渲染。
    expect(html).toContain(`aria-expanded="true"`);
    expect(html).toContain("file-edit-row-add");
    expect(html).toContain("file-edit-row-del");
    expect(html).toContain(">old</span>");
    expect(html).toContain(">new</span>");
    // 无 diff 的段只带类型标签（op 仍是「修改」），不再有「跳过」或整行提示。
    expect(html).toContain("file-edit-kind");
    expect(html).not.toContain("file-edit-op-skipped");
    expect(html).not.toContain("file-edit-notice");
  });
});

describe("file edits auto-expand contract", () => {
  it("folds diff sections under auto-collapse while tool cards still auto-open", () => {
    expect(component).toContain("useState(() => !!edit.diff && (!autoCollapse || expandDuringWork))");
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

describe("turnFileChangesSummary", () => {
  // 净变化（首触 op vs 末态 op、中途创建又删除丢弃、按首触顺序、64 个上限）现在由
  // Host 折叠并随 turn 下发，Rust 侧 server::tests 覆盖；这里只校验客户端如何接受
  // 并渲染这份权威投影。
  it("parses the host summary as-is; rows carry no capture count", () => {
    const summary = turnFileChangesSummary({
      edits: [validEntry, { ...validEntry, path: "docs/x.md", op: "created" }],
      overflow_count: 0,
    });
    expect(summary?.edits.map((edit) => edit.path)).toEqual(["src/main.rs", "docs/x.md"]);
    expect(summary?.overflowCount).toBe(0);
    // 用户反馈：总结不记录修改次数，capture_count 不再是合法字段。
    expect(turnFileChangesSummary({ edits: [{ ...validEntry, capture_count: 2 }] })).toBeUndefined();
  });

  it("reports the host overflow count", () => {
    const summary = turnFileChangesSummary({ edits: [validEntry], overflow_count: 3 });
    expect(summary?.overflowCount).toBe(3);
  });

  it("rejects a malformed summary instead of rendering half a fact", () => {
    expect(turnFileChangesSummary(undefined)).toBeUndefined();
    expect(turnFileChangesSummary({ edits: "nope" })).toBeUndefined();
    expect(turnFileChangesSummary({ edits: [{ ...validEntry, op: "moved" }] })).toBeUndefined();
    expect(turnFileChangesSummary({ edits: [{ ...validEntry, extra_key: 1 }] })).toBeUndefined();
  });

  it("accepts the host's 64 path budget but not more", () => {
    const edits = Array.from({ length: 64 }, (_, index) => ({ ...validEntry, path: `f${index}.txt` }));
    expect(turnFileChangesSummary({ edits })?.edits).toHaveLength(64);
    expect(turnFileChangesSummary({ edits: [...edits, validEntry] })).toBeUndefined();
  });

  it("shows the capture size for a non-text edit instead of a diff", () => {
    const binary = {
      path: "assets/logo.png",
      op: "modified" as const,
      diff: null,
      added_lines: 0,
      removed_lines: 0,
      truncated: false,
      skip_reason: "binary",
      size_bytes: 4096,
    };
    const html = renderToString(
      createElement(FileEditsPanel, { edits: [binary] }),
    ).replace(/<!-- -->/g, "");
    // 大小并入类型标签的 title，行内不再单独显示。
    expect(html).toContain('title="Binary · 4.0 KB"');
    expect(html).not.toContain("file-edit-diff-table");
  });
});

describe("turn edits summary box renders server-side", () => {
  it("renders the title, chip and overflow note without a capture badge", () => {
    const summary = {
      edits: [
        entry({ path: "src/lib.rs", added_lines: 6, removed_lines: 2 }),
        entry({ path: "docs/x.md", op: "created" }),
      ],
      overflowCount: 0,
    };
    const html = renderToString(createElement(TurnEditsSummaryBox, { summary })).replace(/<!-- -->/g, "");
    expect(html).toContain("turn-edits-summary");
    expect(html).not.toContain("file-edit-captures");
    expect(html).not.toContain("×2");
    expect(html).toContain("+6");
    expect(html).not.toContain("file-edit-notice");
  });

  it("reports the bounded overflow when the summary exceeds its budget", () => {
    const summary = {
      edits: [entry({ path: "f0.txt" })],
      overflowCount: 6,
    };
    const html = renderToString(
      createElement(TurnEditsSummaryBox, { summary }),
    );
    expect(html).toContain("file-edit-notice");
  });
});

describe("file edits kind tags", () => {
  it("labels non-text captures with a kind tag instead of a skipped op", () => {
    const html = renderToString(createElement(FileEditsPanel, {
      edits: [
        {
          path: "target/debug/timem",
          op: "modified",
          diff: null,
          added_lines: 0,
          removed_lines: 0,
          truncated: false,
          skip_reason: "binary",
          size_bytes: 260_000_000,
        },
        {
          path: "target/",
          op: "created",
          diff: null,
          added_lines: 0,
          removed_lines: 0,
          truncated: false,
          skip_reason: "directory",
        },
      ],
    })).replace(/<!-- -->/g, "");
    expect(html).toContain("file-edit-op-modified");
    expect(html).toContain("file-edit-op-created");
    expect(html).toContain("file-edit-kind");
    // Binary / size ride on the tag; the folder tag needs no size.
    expect(html).toContain('title="Binary · 248 MB"');
    expect(html).not.toContain("file-edit-op-skipped");
    expect(html).not.toContain("file-edit-notice");
  });
});

describe("file edits panel numbering", () => {
  it("labels each section head with its 1-based index before the op badge, only for multiple files", () => {
    expect(component).toContain('<FileEditSection key={`${edit.path}:${index}`} edit={edit} index={index} showIndex={showIndex} />');
    expect(component).toContain("const showIndex = edits.length > 1;");
    expect(component).toContain("edit: TurnEditEntry; index: number; showIndex: boolean");
    const headIndex = component.indexOf('{showIndex && <span className="file-edit-index">{index + 1}.</span>}');
    const opIndex = component.indexOf("file-edit-op file-edit-op-");
    expect(headIndex).toBeGreaterThan(-1);
    expect(headIndex).toBeLessThan(opIndex);
    expect(styles).toContain(".file-edit-index { flex: none; min-width: 1.2em; text-align: right;");
    expect(styles).toContain(':root[data-theme="light"] .file-edit-index');
  });
});
