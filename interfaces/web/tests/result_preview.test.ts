import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { activityFromTopic, coalesceActionLifecycle } from "../src/view_model";
import { RESULT_PREVIEW_TAIL_LINES, resultTailLines } from "../src/result_preview";

const main = readFileSync(new URL("../src/main.tsx", import.meta.url), "utf8");
const styles = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8");
const view = readFileSync(new URL("../src/view_model.ts", import.meta.url), "utf8");
const host = readFileSync(new URL("../../../core/agent/src/host.rs", import.meta.url), "utf8");
const lib = readFileSync(new URL("../../../core/agent/src/lib.rs", import.meta.url), "utf8");
const server = readFileSync(new URL("../../../applications/timem/src/server.rs", import.meta.url), "utf8");
const shellUi = readFileSync(new URL("../../../interfaces/shell/src/stream_ui.rs", import.meta.url), "utf8");
const en = readFileSync(new URL("../src/i18n/strings.en.ts", import.meta.url), "utf8");
const zh = readFileSync(new URL("../src/i18n/strings.zh.ts", import.meta.url), "utf8");

const TOPIC = { name: "core.action" } as const;

const actionEvent = (
  id: string,
  event: string,
  extra: Record<string, unknown> = {},
  seq = 0,
) => ({
  event_id: `${id}-${event}-${seq}`,
  source: "core_topic" as const,
  payload: {
    session_id: "s1",
    topic: { name: TOPIC.name, attributes: { event } },
    payload: { action: "run_bash", action_id: id, event, ...extra },
  },
  created_at_ms: 1_000 + seq,
  timeline_seq: seq + 1,
});

describe("resultTailLines", () => {
  it("slices the tail and caps the line count", () => {
    const lines = Array.from({ length: 30 }, (_, i) => `line-${i + 1}`);
    const tail = resultTailLines(lines.join("\n"));
    expect(tail).toHaveLength(RESULT_PREVIEW_TAIL_LINES);
    expect(tail[0]).toBe("line-15");
    expect(tail[RESULT_PREVIEW_TAIL_LINES - 1]).toBe("line-30");
  });

  it("keeps short output verbatim and normalizes CRLF", () => {
    expect(resultTailLines("a\r\nb")).toEqual(["a", "b"]);
    expect(resultTailLines("only")).toEqual(["only"]);
    expect(resultTailLines("x\n\n\n")).toEqual(["x"]);
  });
});

describe("coalesceActionLifecycle output events", () => {
  it("merges live previews into the running start row without new rows", () => {
    const start = actionEvent("t1", "start", { input: { cmd: "ci.sh" } }, 0);
    const out1 = actionEvent("t1", "output", { tool_output_preview: "booting" }, 1);
    const out2 = actionEvent("t1", "output", { tool_output_preview: "compiling" }, 2);
    // While running: one row, preview holds the latest snapshot.
    const running = coalesceActionLifecycle([start, out1, out2]);
    expect(running).toHaveLength(1);
    const runningPayload = running[0].payload as unknown as {
      payload: { tool_output_preview?: string };
    };
    expect(runningPayload.payload.tool_output_preview).toBe("compiling");
    // Finish replaces the row payload with the authoritative final tail.
    const finish = actionEvent("t1", "finish", { status: "completed", tool_output_preview: "done" }, 3);
    const settled = coalesceActionLifecycle([start, out1, out2, finish]);
    expect(settled).toHaveLength(1);
    const settledPayload = settled[0].payload as unknown as {
      payload: { tool_output_preview?: string; status?: string };
    };
    expect(settledPayload.payload.status).toBe("completed");
    expect(settledPayload.payload.tool_output_preview).toBe("done");
  });

  it("drops output events whose start row is missing", () => {
    const out = actionEvent("ghost", "output", { tool_output_preview: "x" }, 0);
    expect(coalesceActionLifecycle([out])).toHaveLength(0);
  });

  it("keeps distinct actions in parallel streams separated", () => {
    const a = actionEvent("a1", "start", {}, 0);
    const b = actionEvent("b1", "start", {}, 1);
    const aOut = actionEvent("a1", "output", { tool_output_preview: "A" }, 2);
    const bOut = actionEvent("b1", "output", { tool_output_preview: "B" }, 3);
    const visible = coalesceActionLifecycle([a, b, aOut, bOut]);
    expect(visible).toHaveLength(2);
    const pa = visible[0].payload as unknown as { payload: { tool_output_preview?: string } };
    const pb = visible[1].payload as unknown as { payload: { tool_output_preview?: string } };
    expect(pa.payload.tool_output_preview).toBe("A");
    expect(pb.payload.tool_output_preview).toBe("B");
  });
});

describe("activityFromTopic result preview mapping", () => {
  it("maps finish tool_output_preview into activity.result_preview", () => {
    const activity = activityFromTopic({
      session_id: "s1",
      topic: { name: TOPIC.name },
      state: { name: "running" },
      payload: {
        action: "run_bash",
        action_id: "t9",
        event: "finish",
        status: "completed",
        tool_output_preview: "line-1\nline-2",
        tool_output_bytes: 42,
      },
    } as never) as { result_preview?: string } | null;
    expect(activity?.result_preview).toBe("line-1\nline-2");
  });

  it("omits the preview when the payload carries none", () => {
    const activity = activityFromTopic({
      session_id: "s1",
      topic: { name: TOPIC.name },
      state: { name: "running" },
      payload: { action: "run_bash", action_id: "t9", event: "finish", status: "completed" },
    } as never) as { result_preview?: string } | null;
    expect(activity?.result_preview).toBeUndefined();
  });
});

describe("result preview surfaces and contracts", () => {
  it("renders the preview in both stream and archived tool rows", () => {
    expect(main.match(/<StreamToolResultPreview/g)).toHaveLength(2);
    // 归档视图：结束自动收起（瞬时窗口），徽章再展开。
    expect(main).toContain("resultPreview && (running || previewReopened)");
    expect(main).toContain("!running && resultPreview && !previewReopened");
    expect(main).toContain('className="stream-tool-result-reopen"');
    // 流式行（用户反馈）：自动展开（运行中/未收纳常驻持续刷新 tail）；无标题标签；
    // 收纳或手动收起后仅在工具行头行尾保留图标徽章（不单独占行）再展开。
    // 用户反馈：命令带 diff 框时优先显示 diff，结果预览不再默认展开（仅眼睛徽章可开）。
    // 收纳后预览要在收拢动画结束后才撤掉，否则过渡从更矮的高度起步。
    expect(main).toContain("const previewVisible = !!resultPreview && !previewDismissed && (previewReopened || (!hasEdits && (running || !settledAbsorbed)));");
    expect(main).toContain("{previewVisible && (");
    // 用户反馈：收起带来的位移不能让阅读位置跑掉——由流内局部滚动锚定补偿。
    expect(main).toContain("useStreamShrinkAnchor(runRef);");
    expect(main).toContain("!running && resultPreview && !previewVisible && (");
    expect(main).toContain('aria-label={t("tools.resultPreview")}');
    expect(main).not.toContain("stream-tool-result-preview-badge");
    expect(main).not.toContain("resultPreviewLiveHint");
    expect(main).toContain("a.result_preview === b.result_preview");
    // 仅观察语义：只读 log；文本允许复制。
    expect(main).toContain('role="log"');
    expect(main).toContain("resultTailLines");
    expect(main).toContain("redactSensitiveDisplayText(line) || line");
    // 限高后内部滚动要贴住最新输出；用户自己往上翻过就不再抢滚动。
    expect(main).toContain("node.scrollTop = node.scrollHeight;");
    expect(main).toContain("followsTail.current = node.scrollHeight - node.scrollTop - node.clientHeight <= 8;");
  });

  it("styles the transient read-only window in both themes", () => {
    expect(styles).toContain(".stream-tool-result-preview {");
    const start = styles.indexOf(".stream-tool-result-preview {");
    const block = styles.slice(start, styles.indexOf("}", start) + 1);
    // 用户反馈：无框色块（非虚线框）。
    expect(block).toContain("border: none;");
    expect(block).toContain("background:");
    expect(block).not.toContain("dashed");
    // 用户反馈：不只在最外层去虚线——head 的虚线下分隔线也要去掉，
    // 正文左侧的 2px 粗色条同样去掉，整个预览就只剩一块圆角色块。
    const headStart = styles.indexOf(".stream-tool-result-preview-head {");
    const head = styles.slice(headStart, styles.indexOf("}", headStart) + 1);
    expect(head).not.toContain("dashed");
    expect(head).toContain("border: 0;");
    const bodyStart = styles.indexOf(".stream-tool-result-preview-body {");
    const body = styles.slice(bodyStart, styles.indexOf("}", bodyStart) + 1);
    expect(body).not.toContain("border-left");
    expect(body).toContain("border: 0;");
    // 用户反馈：预览限高 4 行（内部滚动），收纳时撤掉的位移才足够小。
    expect(body).toContain("max-height: calc(4 * 1.55em + 14px);");
    expect(body).toContain("overflow: auto;");
    expect(styles).not.toContain(".stream-tool-result-preview.running .stream-tool-result-preview-body");
    expect(styles).toContain("@keyframes stream-result-pulse");
    for (const selector of [
      ':root[data-theme="light"] .stream-tool-result-preview {',
      ':root[data-theme="light"] .stream-tool-result-reopen {',
      ".stream-tool-result-preview-copy.copy-success",
    ]) {
      expect(styles).toContain(selector);
    }
  });

  it("keeps i18n catalogs in sync for both locales", () => {
    for (const key of [
      "resultPreview",
      "resultPreviewLiveHint",
      "resultPreviewCopy",
      "resultPreviewCopied",
      "resultPreviewCollapse",
      "resultPreviewAria",
    ]) {
      expect(en).toContain(`${key}:`);
      expect(zh).toContain(`${key}:`);
    }
  });

  it("bounds the projection end to end", () => {
    // Host event builder: capped live event with final preview contract.
    expect(host).toContain("pub fn action_output_topic_event");
    // Finish event carries the final bounded preview.
    expect(lib).toContain('event.payload["tool_output_preview"] = json!(preview);');
    // Hard caps: lines/bytes/interval/per-action count.
    expect(lib).toContain("TOOL_OUTPUT_PREVIEW_LINES: usize = 16");
    expect(lib).toContain("TOOL_OUTPUT_PREVIEW_BYTES: usize = 4 * 1024");
    expect(lib).toContain("OUTPUT_PREVIEW_MAX_EVENTS_PER_ACTION: u32 = 240");
    // Live-only events never reach chat history.
    expect(server).toContain("is_transient_action_output_event");
    // Shell fold ignores the live-only events.
    expect(shellUi).toContain('if action.event == "output"');
    // Client line cap mirrors the Host budget.
    expect(view).toContain('typeof payload.tool_output_preview === "string"');
  });
});
