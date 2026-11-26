import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";

const main = readFileSync(new URL("../src/main.tsx", import.meta.url), "utf8");
const styles = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8");

// 用户反馈：工具胶囊是 remark 的收尾，不是独立一行——紧邻动作组的纯文本条目
// 并入该组作领先文本，胶囊在句末内联显示，同一按钮负责展开/收起工具行。
describe("stream tool lead capsule", () => {
  it("merges the adjacent remark into the following tool group", () => {
    expect(main).toContain("next.lead = source.entries[source.entries.length - 1];");
    expect(main).toContain("source.entries = source.entries.slice(0, -1);");
    // 用户反馈（修正）：remark 优先于 thought 预览——单次从近到远扫描，
    // 命中最近 remark 即用，否则用扫描中最近的 thought。
    expect(main).toContain("if (remarkSource === -1 && isRemarkGroup(candidate)) remarkSource = back;");
    expect(main).toContain("if (thoughtSource === -1 && isThoughtGroup(candidate)) thoughtSource = back;");
    expect(main).toContain("const leadSource = remarkSource !== -1 ? remarkSource : thoughtSource;");
    // 缺陷看护：已消费的空文本组必须截断回溯——否则 [remark1][remark2][动作][间隔][动作]
    // 会让 remark1（或 thought1）被更靠后的动作组重复消费，同段文本渲染两遍。
    expect(main).toContain("if (candidate.consumed) break;");
    expect(main).toContain("if (source.entries.length === 0) source.consumed = true;");
    expect(main).toContain("const visibleGroups = renderedGroups.filter(group => group.entries.length > 0);");
    // 时序语义不变：并入组内的 free_talk 章节仍算「回复位置」。
    expect(main).toContain("const leadChapter = group.lead?.activity?.kind === \"free_talk\"");
  });

  it("renders the capsule inline at the end of the lead text", () => {
    expect(main).toContain('className={`stream-thought-text stream-tool-lead${leadInline ? " inline" : ""}`}');
    expect(main).toContain("{leadInline ? toggleButton : null}");
    expect(main).toContain("{leadInline ? null : toggleButton}");
    // 单行纯文本才内联；含换行/列表/代码/强调标记的富文本回退块级。
    expect(main).toContain("const leadInline = !!lead && !/[\\n*_\\[\\]`|]|^\\s*(?:[-+]|\\d+\\.)\\s/.test(lead.text.trim());");
  });

  it("keeps the inline capsule as a pill that does not reflow the standalone row", () => {
    expect(styles).toContain(".stream-tool-lead.inline .stream-tool-lead-text { white-space: pre-wrap; }");
    expect(styles).toContain(".stream-tool-lead.inline .stream-tool-run-toggle { display: inline-flex; margin-left: 6px; vertical-align: 1px; }");
    // 无领先文本时保持原契约：独立一行、靠右。
    expect(styles).toMatch(/\.stream-tool-run-toggle \{[^}]*display: flex;[^}]*margin-left: auto;/s);
  });
});
