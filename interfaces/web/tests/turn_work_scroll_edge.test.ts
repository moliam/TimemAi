import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";

const styles = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8");

function ruleBody(selector: string) {
  const start = styles.indexOf(`${selector} {`);
  expect(start).toBeGreaterThanOrEqual(0);
  const end = styles.indexOf("}", start);
  expect(end).toBeGreaterThan(start);
  return styles.slice(start, end);
}

describe("thought/action scroll edge", () => {
  it("separates the expanded work stream from the answer below", () => {
    const body = ruleBody(".turn-work-panel:not(.has-live-usage)::after");
    expect(body).toContain("position: absolute;");
    expect(body).toContain("bottom: 0;");
    expect(body).toContain("height: 1px;");
    expect(body).toContain("background: rgba(255, 255, 255, 0.10);");
  });

  it("draws a left rail on the work scroll window", () => {
    const body = ruleBody(".turn-work-scroll");
    expect(body).toContain("border-left: 2px solid rgba(255, 255, 255, 0.16);");
    expect(body).toContain("padding: 7px 0 10px 14px;");
    const light = styles.indexOf(':root[data-theme="light"] .turn-work-scroll { border-left-color: rgba(0, 0, 0, 0.14); }');
    expect(light).toBeGreaterThanOrEqual(0);
    // 级联护栏：scroll 的 padding 被后置规则整体覆盖过两次（紧凑皮肤与 fade-size 规则），
    // 任何一处丢了左距都会让轨线贴住内容——三处必须同时带 padding-left。
    const fade = styles.indexOf('.turn-work-scroll { --work-edge-fade-size: 16px; padding: 2px 0 6px 14px; }');
    expect(fade).toBeGreaterThanOrEqual(0);
    const compact = styles.indexOf('max-height: min(630px, 75vh); padding: 0 0 0 14px;');
    expect(compact).toBeGreaterThanOrEqual(0);
  });
});
