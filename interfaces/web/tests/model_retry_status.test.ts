import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const styles = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8");
const source = readFileSync(new URL("../src/main.tsx", import.meta.url), "utf8");

describe("model retry status pill", () => {
  it("flips the detail panel upward when the pill sits near the composer", () => {
    // 用户反馈：贴底的错误胶囊向下展开会被输入框和阅读区一起裁掉——面板是绝对定位，
    // 不进入滚动高度，页面也滚不上来看不全。契约：量上下余量，下方不够就向上展开。
    expect(source).toContain('node.classList.add("is-above")');
    expect(source).toContain("const above = Math.max(0, box.top - view.top - gap);");
    expect(source).toContain("const below = Math.max(0, view.bottom - box.bottom - gap);");
    expect(source).toContain("if (below >= needed || below >= above) return;");
    expect(styles).toContain(
      ".model-retry-status.is-above .model-retry-detail { top: auto; bottom: calc(100% + 7px); }",
    );
  });

  it("keeps the flipped panel readable inside the remaining space", () => {
    expect(source).toContain(
      'detail.style.maxHeight = `${Math.round(Math.max(96, Math.min(240, above)))}px`;',
    );
    // 打开状态下滚动/缩放要重新判定，避免胶囊移到底部后还留在向下展开。
    expect(source).toContain('chat.addEventListener("scroll", onViewportChange, { passive: true });');
    expect(styles).toContain("width: min(420px, calc(100vw - 48px));");
  });
});
