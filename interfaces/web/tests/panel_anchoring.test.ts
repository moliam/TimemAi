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
});
