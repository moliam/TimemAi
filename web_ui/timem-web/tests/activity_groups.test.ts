import { describe, expect, it } from "vitest";
import { Activity } from "../src/protocol";
import { summarizeConsecutiveToolActivities, summarizeToolActivities } from "../src/activity_groups";

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

  it("splits tool summaries at each visible thought update", () => {
 const thought = (id: string): Activity => ({
 id,
 sessionId: "session-1",
 tone: "thinking",
 title: "",
 detail: id,
 createdAt: 1,
 });
 const runs = summarizeConsecutiveToolActivities([
 thought("thought-1"),
 activity("run_bash", "completed"),
 activity("self_tool", "completed"),
 thought("thought-2"),
 activity("run_bash", "completed"),
 thought("thought-3"),
 activity("memmgr", "completed"),
 activity("run_bash", "completed"),
 ]);
 expect(runs.map(({ startIndex, summary }) => ({
 startIndex,
 label: summary.label,
 activities: summary.activities.length,
 }))).toEqual([
 { startIndex: 1, label: "Bash ×1, Self Tool ×1", activities: 2 },
 { startIndex: 4, label: "Bash ×1", activities: 1 },
 { startIndex: 6, label: "MemMgr ×1, Bash ×1", activities: 2 },
 ]);
 });
 it("does not split adjacent tool activity on invisible events", () => {
 const runs = summarizeConsecutiveToolActivities([
 activity("run_bash", "completed"),
 null,
 activity("self_tool", "completed"),
 ]);
 expect(runs).toHaveLength(1);
 expect(runs[0].startIndex).toBe(0);
 expect(runs[0].summary.label).toBe("Bash ×1, Self Tool ×1");
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
