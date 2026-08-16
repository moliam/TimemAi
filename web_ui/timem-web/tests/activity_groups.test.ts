import { describe, expect, it } from "vitest";
import { Activity } from "../src/protocol";
import { summarizeToolActivities } from "../src/activity_groups";

function activity(tool_name: string, tool_status: string): Activity {
  return {
    id: `${tool_name}-${tool_status}`,
    sessionId: "session-1",
    tone: "action",
    title: tool_name,
    tool_name,
    tool_status,
    createdAt: 1,
  };
}

describe("tool activity grouping", () => {
  it("counts tools in first-seen order", () => {
    const summary = summarizeToolActivities([
      activity("run_bash", "completed"),
      activity("self_tool", "completed"),
      activity("run_bash", "completed"),
      activity("run_bash", "completed"),
    ]);
    expect(summary?.label).toBe("Bash ×3, Self Tool ×1");
    expect(summary?.activities).toHaveLength(4);
  });

  it("reports running while any tool remains active", () => {
    const summary = summarizeToolActivities([
      activity("run_bash", "completed"),
      activity("self_tool", "background_running"),
      activity("memmgr", "failed"),
    ]);
    expect(summary?.status).toBe("running");
  });

  it("reports failed after all tools settle if any failed", () => {
    expect(summarizeToolActivities([
      activity("run_bash", "completed"),
      activity("self_tool", "timeout"),
    ])?.status).toBe("failed");
  });

  it("reports completed when every tool completes", () => {
    expect(summarizeToolActivities([
      activity("run_bash", "completed"),
      activity("self_tool", "success"),
    ])?.status).toBe("completed");
  });

  it("ignores non-action activity", () => {
    const thought: Activity = {
      id: "thought",
      sessionId: "session-1",
      tone: "thinking",
      title: "",
      detail: "Investigating",
      createdAt: 1,
    };
    expect(summarizeToolActivities([thought])).toBeNull();
  });
});
