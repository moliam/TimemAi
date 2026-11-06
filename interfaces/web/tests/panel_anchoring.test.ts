import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const source = readFileSync(new URL("../src/main.tsx", import.meta.url), "utf8");
const styles = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8");

describe("floating panel anchoring", () => {
  it("opens the MCP panel against its header trigger, not at a fixed left inset", () => {
    expect(styles).toContain(
      ".mcp-panel { position: absolute; z-index: 10; top: 62px; right: var(--mcp-anchor-right, 24px); left: auto;",
    );
    expect(source).toContain(
      "const [mcpPanelAnchorRight, setMcpPanelAnchorRight] = useState<number | null>(null);",
    );
    expect(source).toContain("useLayoutEffect(() => {\n    if (!showMcp) return;");
    expect(source).toContain("const boundary =");
    expect(source).toContain('"--mcp-anchor-right": `${anchorRight}px`');
    // Mobile keeps the inset takeover; the custom property only steers the
    // desktop dropdown's horizontal anchor.
    expect(styles).toContain("  .mcp-panel {\n    position: fixed;\n    inset: 62px 12px auto;");
  });

  it("keeps the header reasoning select content-sized instead of stretching the row", () => {
    expect(styles).toContain(
      ".header-model-popover-select { justify-self: start; min-width: 64px; max-width: 100%;",
    );
  });

  it("opens timers and session info as anchored cards instead of right sidebars", () => {
    expect(styles).toContain(".timer-panel,\n.mailbox-panel {\n  position: absolute;");
    expect(styles).toContain("right: var(--session-popover-right, 24px);");
    expect(styles).toContain("background: #171e1c;");
    expect(source).toContain('className="timer-panel session-popover"');
    expect(source).toContain('className="mailbox-panel session-popover"');
    expect(source).toContain('"--session-popover-right": `${anchorRight}px`');
    expect(source).toContain("const [timersPanelAnchorRight, setTimersPanelAnchorRight] = useState<");
    expect(source).toContain("const [mailboxPanelAnchorRight, setMailboxPanelAnchorRight] = useState<");
    expect(source).toContain("const closeTimersPanel = useCallback((restoreFocus = true) => {");
    expect(source).toContain("timersButtonRef.current?.contains(target) ||");
    expect(source).toContain("mailboxButtonRef.current?.contains(target) ||");
    expect(source).not.toContain("timer-panel toolrepo-side-panel session-side-panel");
    expect(source).not.toContain("mailbox-panel toolrepo-side-panel session-side-panel");
    expect(source).not.toContain('{showTimers && (\n          <button\n            type="button"\n            className="side-panel-backdrop"');
    expect(source).not.toContain('{showMailbox && (\n          <button\n            type="button"\n            className="side-panel-backdrop"');
  });

  it("uses the direct-conversation icon for session info", () => {
    expect(source).toContain("MessagesSquare,");
    expect(source).toContain("<MessagesSquare size={17} />");
    expect(source).toContain("<MessagesSquare size={15} />");
    expect(source).not.toContain("<Info size={17} />");
    expect(source).not.toContain("<Info size={15} />");
  });
});
