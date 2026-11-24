import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const source = readFileSync(new URL("../src/main.tsx", import.meta.url), "utf8");
const styles = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8");

describe("fixed composer scroll ownership", () => {
  it("keeps the complete input surface outside the scrolling message viewport", () => {
    expect(source).toContain('className="chat-scroll aui-thread-viewport"');
    // The composer must stay a sibling after the scrolling viewport: the gap
    // anchor is the viewport's last child, and the composer follows its close.
    expect(source).toMatch(
      /<div\s+className="chat-scroll-shrink-gap"[\s\S]*?\/>\s*<\/div>\s*<div className="composer-wrap aui-thread-footer">/,
    );
    const viewportEnd = source.indexOf('className="chat-scroll aui-thread-viewport"');
    const composerStart = source.indexOf(
      '<div className="composer-wrap aui-thread-footer">',
    );
    const navigationStart = source.indexOf(
      '<nav\n        ref={userMessageNavigationRef}',
    );
    expect(viewportEnd).toBeGreaterThan(0);
    expect(composerStart).toBeGreaterThan(viewportEnd);
    expect(navigationStart).toBeGreaterThan(composerStart);
    expect(source).not.toContain(
      '<ThreadPrimitive.ViewportFooter className="composer-wrap aui-thread-footer">',
    );
  });

  it("gives the message timeline the flexible scroll slot and the composer a bounded sibling slot", () => {
    expect(styles).toMatch(
      /\.aui-thread \{[^}]*min-height: 0;[^}]*display: flex;[^}]*flex-direction: column;[^}]*overflow: hidden;/,
    );
    expect(styles).toMatch(
      /\.chat-scroll \{[^}]*flex: 1 1 auto;[^}]*min-height: 0;[^}]*overflow-y: auto;/,
    );
    expect(styles).toMatch(
      /\.composer-wrap \{[^}]*position: relative;[^}]*flex: none;[^}]*max-height: min\(72dvh, 720px\);[^}]*display: flex;[^}]*flex-direction: column;[^}]*overflow: hidden;/,
    );
    expect(styles).not.toMatch(/\.composer-wrap \{[^}]*position: sticky;/);
    expect(styles).toMatch(/\.composer \{[^}]*flex: none;/);
    expect(styles).toMatch(
      /\.attachment-strip \{[^}]*flex: none;[^}]*max-height: min\(22dvh, 144px\);[^}]*overflow-y: auto;/,
    );
    // One collapse level only: the item list itself is capped near four rows
    // and scrolls; no second row-level collapsed/expanded queue state exists.
    expect(styles).toMatch(
      /\.queued-message-items \{[^}]*max-height: 224px;[^}]*overflow-y: auto;[^}]*overscroll-behavior: contain;/,
    );
    expect(styles).not.toMatch(/\.queued-message-list\.(?:collapsed|expanded)/);
    // 用户反馈：待发送队列与输入框外框同宽、同左缘（聊天内容列左缘），不再
    // 右移让出导航按钮列的 gutter；对应变量也已移除。
    expect(styles).toMatch(/\.queued-message-list \{[^}]*width: 100%;/);
    expect(styles).not.toMatch(/composer-nav-gutter/);
  });

  it("widens the chat measure and keeps composer, cwd, and turn widths in step", () => {
    expect(styles).toMatch(/\.chat-scroll \{[^}]*max\(26px, calc\(\(100% - 980px\)\/2 - 22px\)\)/);
    expect(styles).toMatch(/\.chat-scroll \{[^}]*max\(26px, calc\(\(100% - 980px\)\/2 \+ 22px\)\)/);
    expect(styles).toMatch(/\.composer-wrap \{[^}]*max\(26px, calc\(\(100% - 980px\)\/2 - 22px\)\)/);
    expect(styles).toMatch(/\.message \{[^}]*max-width: 900px;/);
    expect(styles).toMatch(/\.working-indicator \{[^}]*max-width: 900px;/);
    // 用户反馈：阅读列不再收窄到 900/768 居中，而是铺满输入框的宽度，并与之左对齐
    // （左浮动导航按钮列已挪进输入框左侧的 .composer-row）。
    expect(styles).toMatch(/\.turn-interaction \{[^}]*max-width: none;/);
    expect(styles).not.toMatch(/\.turn-interaction \{[^}]*max-width: (?:900|768)px;/);
    expect(styles).toMatch(/\.composer-cwd \{[^}]*max-width: 876px;/);
  });

  it("keeps the message navigation beside the composer instead of floating over the column", () => {
    // 用户反馈：上下跳转/working 按钮挪到输入框左侧，阅读列因此可以左对齐输入框并更宽。
    expect(source).toContain('<div className="composer-row">');
    expect(source).toContain('<nav\n        ref={userMessageNavigationRef}');
    expect(styles).toMatch(/\.composer-row \{[^}]*display: grid;[^}]*grid-template-columns: auto minmax\(0, 1fr\);/);
    // 用户反馈：导航列要与输入框顶部对齐，且整体比输入框矮（3×24 + 2×6 = 84px
    // 对输入框约 94px），所以从底部对齐改成顶部对齐并统一缩小一档。
    expect(styles).toMatch(/\.user-message-navigation \{[^}]*order: 1;[^}]*align-self: start;/);
    expect(styles).toMatch(/\.user-message-navigation button \{[^}]*width: 24px;[^}]*height: 24px;/);
    expect(styles).not.toMatch(/\.user-message-navigation \{[^}]*position: absolute;/);
    // 输入框左右内边距一致；正文左内边距让出按钮列（+22px）后与输入框左边缘对齐。
    expect(styles).toMatch(/\.composer-wrap \{[^}]*padding: 16px max\(26px, calc\(\(100% - 980px\)\/2 - 22px\)\) 20px max\(26px, calc\(\(100% - 980px\)\/2 - 22px\)\);/);
    expect(styles).toMatch(/\.chat-scroll \{[^}]*padding: 34px max\(26px, calc\(\(100% - 980px\)\/2 - 22px\)\) 24px max\(26px, calc\(\(100% - 980px\)\/2 \+ 22px\)\);/);
  });

  it("uses the formal interface font for the CWD and debug paths", () => {
    expect(source).toContain('className="composer-cwd-inline"');
    expect(source).toContain('className="composer-cwd-inline composer-debug-inline composer-debug-link"');
    const pathRule = styles.match(/\.composer-cwd-inline span \{([^}]*)\}/)?.[1];
    expect(pathRule).toContain("font-family: var(--ui-font)");
    expect(pathRule).not.toMatch(/SFMono|Consolas|monospace/);
  });

  it("keeps multiline wheel handling attached across conditional composer remounts", () => {
    const attachment = source.slice(
      source.indexOf("const attachComposerTextarea = useCallback"),
      source.indexOf("const [composerExpanded"),
    );
    expect(source).toContain("attachRef={attachComposerTextarea}");
    expect(source).toContain("ref={attachRef}");
    expect(attachment).toContain('textarea.addEventListener("wheel"');
    expect(attachment).toContain("passive: false");
    expect(attachment).toContain("canScrollInDirection(textarea, deltaY)");
    expect(attachment).toContain("event.preventDefault()");
    expect(attachment).toContain("textarea.scrollTop += deltaY");
    expect(attachment).toContain('textarea.removeEventListener("wheel"');
  });
});
