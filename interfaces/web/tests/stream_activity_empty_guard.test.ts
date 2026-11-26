import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";

const source = readFileSync(new URL("../src/main.tsx", import.meta.url), "utf8");

describe("empty stream activity guard", () => {
  it("renders no stream-only container while the activity stream is empty", () => {
    const start = source.indexOf("function StreamActivityPresentation(");
    expect(start).toBeGreaterThanOrEqual(0);
    const sectionStart = source.indexOf(
      'return <section className="turn-stream-tools"',
      start,
    );
    expect(sectionStart).toBeGreaterThan(start);
    // 非流式工作面板复用流式呈现（fold the work stream）：空 entries 必须在
    // 无条件渲染 section 之前返回 null，否则 working turn 会泄漏
    // provisional UI，non-stream 验收断言会失败。
    const guardStart = source.indexOf(
      "if (uniqueEntries.length === 0) return null;",
      start,
    );
    expect(guardStart).toBeGreaterThan(start);
    expect(guardStart).toBeLessThan(sectionStart);
  });
});
