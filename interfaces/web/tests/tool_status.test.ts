import { describe, expect, it } from "vitest";
import { formatToolElapsed, humanizeToolStatus, toolResultCountsLabel, isToolActivityFailed, isToolActivityRunning } from "../src/tool_status";
import { setLocale } from "../src/i18n";

setLocale("zh");

describe("stream tool elapsed label", () => {
  it("keeps 0.1s precision under a minute and compacts beyond it", () => {
    expect(formatToolElapsed(0)).toBe("0s");
    expect(formatToolElapsed(100)).toBe("0.1s");
    expect(formatToolElapsed(1300)).toBe("1.3s");
    expect(formatToolElapsed(12300)).toBe("12.3s");
    expect(formatToolElapsed(59600)).toBe("59.6s");
    expect(formatToolElapsed(60000)).toBe("1m0s");
    expect(formatToolElapsed(306000)).toBe("5m6s");
    expect(formatToolElapsed(3720000)).toBe("1h2m");
    expect(formatToolElapsed(-5)).toBe("0s");
  });
});

describe("tool result labels", () => {
  it("hides zero failures without a trailing separator", () => {
    expect(toolResultCountsLabel(0, 0)).toBe("0 ✓");
    expect(toolResultCountsLabel(2, 0)).toBe("2 ✓");
  });
  it("shows nonzero failures", () => {
    expect(toolResultCountsLabel(2, 1)).toBe("2 ✓ | 1 ✗");
    expect(toolResultCountsLabel(0, 2)).toBe("0 ✓ | 2 ✗");
  });
  it("uses ✓ and ✗ for terminal result labels", () => {
    expect(humanizeToolStatus("completed")).toBe("✓");
    expect(humanizeToolStatus("failed")).toBe("✗");
    expect(humanizeToolStatus("running")).toBe("running");
    expect(humanizeToolStatus("background_running")).toBe("后台运行");
    expect(humanizeToolStatus("timeout")).toBe("已超时");
  });
  it.each(["error", "timeout", "cancelled", "cancelled_by_user"])("preserves failure semantics for %s", status => {
    expect(isToolActivityFailed(status)).toBe(true);
    expect(isToolActivityRunning(status)).toBe(false);
    expect(humanizeToolStatus(status)).toBe(status === "timeout" ? "已超时" : status.replaceAll("_", " "));
  });
  it("does not reinterpret unknown statuses", () => {
    expect(humanizeToolStatus("future_state")).toBe("future state");
    expect(isToolActivityFailed("future_state")).toBe(false);
    expect(isToolActivityRunning("future_state")).toBe(false);
  });
});
