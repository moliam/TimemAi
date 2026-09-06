import { describe, expect, it } from "vitest";
import { humanizeToolStatus, toolResultCountsLabel, isToolActivityFailed, isToolActivityRunning } from "../src/tool_status";

describe("tool result labels", () => {
  it("hides zero failures without a trailing separator", () => {
    expect(toolResultCountsLabel(0, 0)).toBe("0 Succ");
    expect(toolResultCountsLabel(2, 0)).toBe("2 Succ");
  });
  it("shows nonzero failures", () => {
    expect(toolResultCountsLabel(2, 1)).toBe("2 Succ | 1 Failed");
    expect(toolResultCountsLabel(0, 2)).toBe("0 Succ | 2 Failed");
  });
  it("uses Succ and Failed for terminal result labels", () => {
    expect(humanizeToolStatus("completed")).toBe("Succ");
    expect(humanizeToolStatus("failed")).toBe("Failed");
    expect(humanizeToolStatus("running")).toBe("running");
    expect(humanizeToolStatus("background_running")).toBe("running (bg)");
    expect(humanizeToolStatus("timeout")).toBe("timed out");
  });
  it.each(["error", "timeout", "cancelled", "cancelled_by_user"])("preserves failure semantics for %s", status => {
    expect(isToolActivityFailed(status)).toBe(true);
    expect(isToolActivityRunning(status)).toBe(false);
    expect(humanizeToolStatus(status)).toBe(status === "timeout" ? "timed out" : status.replaceAll("_", " "));
  });
  it("does not reinterpret unknown statuses", () => {
    expect(humanizeToolStatus("future_state")).toBe("future state");
    expect(isToolActivityFailed("future_state")).toBe(false);
    expect(isToolActivityRunning("future_state")).toBe(false);
  });
});
