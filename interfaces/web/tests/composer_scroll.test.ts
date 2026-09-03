import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const source = readFileSync(new URL("../src/main.tsx", import.meta.url), "utf8");
const styles = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8");

describe("fixed composer scroll ownership", () => {
  it("keeps the complete input surface outside the scrolling message viewport", () => {
    expect(source).toMatch(
      /<ThreadPrimitive\.Viewport[\s\S]*<\/ThreadPrimitive\.Viewport>\s*<div className="composer-wrap aui-thread-footer">/,
    );
    const viewportEnd = source.indexOf("</ThreadPrimitive.Viewport>");
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
    expect(styles).toMatch(
      /\.queued-message-list\.expanded \{[^}]*grid-template-rows: auto minmax\(0, 1fr\);[^}]*overflow: hidden;/,
    );
    expect(styles).toMatch(
      /\.queued-message-list\.expanded \.queued-message-items \{[^}]*overflow-y: auto;[^}]*overscroll-behavior: contain;/,
    );
  });

  it("keeps multiline wheel handling attached across conditional composer remounts", () => {
    const attachment = source.slice(
      source.indexOf("const attachComposerTextarea = useCallback"),
      source.indexOf("const [composerExpanded"),
    );
    expect(source).toContain("ref={attachComposerTextarea}");
    expect(attachment).toContain('textarea.addEventListener("wheel"');
    expect(attachment).toContain("passive: false");
    expect(attachment).toContain("canScrollInDirection(textarea, deltaY)");
    expect(attachment).toContain("event.preventDefault()");
    expect(attachment).toContain("textarea.scrollTop += deltaY");
    expect(attachment).toContain('textarea.removeEventListener("wheel"');
  });
});
