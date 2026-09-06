import { describe, expect, it } from "vitest";
import { Activity } from "../src/protocol";
import {
  coalesceFreeTalkItems,
  computeStreamRetention,
  isRunningToolActivity,
  summarizeConsecutiveToolActivities,
  summarizeToolActivities,
} from "../src/activity_groups";

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
    expect(summary?.label).toBe("bash 3 | self tool 1");
    expect(summary?.counts).toEqual([
      { name: "bash", count: 3 },
      { name: "self tool", count: 1 },
    ]);
    expect(summary?.activities).toHaveLength(4);
  });

  it("counts polling Bash separately as Poll", () => {
    expect(summarizeToolActivities([
      { ...activity("run_bash", "running"), tool_mode: "poll" },
      activity("run_bash", "completed"),
    ])?.counts).toEqual([
      { name: "poll", count: 1 },
      { name: "bash", count: 1 },
    ]);
  });

  it("counts foreground, background, failed, and completed children independently", () => {
    const summary = summarizeToolActivities([
      activity("run_bash", "running"),
      activity("run_bash", "background_running"),
      activity("self_tool", "background_running"),
      activity("memmgr", "failed"),
      activity("self_tool", "completed"),
    ]);
    expect(summary).toMatchObject({
      status: "running",
      foregroundRunningCount: 1,
      backgroundRunningCount: 2,
      failedCount: 1,
      completedCount: 1,
    });
  });

  it("keeps the group running until every background child exits", () => {
    const oneStillRunning = summarizeToolActivities([
      activity("run_bash", "background_finished"),
      activity("run_bash", "background_running"),
    ]);
    expect(oneStillRunning).toMatchObject({
      status: "running",
      backgroundRunningCount: 1,
      completedCount: 1,
    });

    const allExited = summarizeToolActivities([
      activity("run_bash", "background_finished"),
      activity("run_bash", "completed"),
    ]);
    expect(allExited).toMatchObject({
      status: "completed",
      backgroundRunningCount: 0,
      completedCount: 2,
    });
  });

  it("reports failed only after all tools settle and preserves the failure count", () => {
    expect(summarizeToolActivities([
      activity("run_bash", "completed"),
      activity("self_tool", "timeout"),
      activity("memmgr", "cancelled"),
    ])).toMatchObject({ status: "failed", failedCount: 2 });
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
 { startIndex: 1, label: "bash 1 | self tool 1", activities: 2 },
 { startIndex: 4, label: "bash 1", activities: 1 },
 { startIndex: 6, label: "memmgr 1 | bash 1", activities: 2 },
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
 expect(runs[0].summary.label).toBe("bash 1 | self tool 1");
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

describe("running tool activity gate", () => {
  const base = {
    id: "a1",
    sessionId: "s1",
    title: "run_bash",
    createdAt: 1,
  };
  it("marks running foreground/background actions as live", () => {
    expect(
      isRunningToolActivity({ ...base, tone: "action", tool_status: "running" }),
    ).toBe(true);
    expect(
      isRunningToolActivity({
        ...base,
        tone: "action",
        tool_status: "background_running",
      }),
    ).toBe(true);
  });

  it("keeps terminal actions, non-actions and toolgen inside the frame", () => {
    expect(
      isRunningToolActivity({ ...base, tone: "action", tool_status: "finish" }),
    ).toBe(false);
    expect(
      isRunningToolActivity({ ...base, tone: "action", tool_status: "timeout" }),
    ).toBe(false);
    expect(
      isRunningToolActivity({ ...base, tone: "thinking" }),
    ).toBe(false);
    expect(isRunningToolActivity(null)).toBe(false);
    expect(
      isRunningToolActivity({
        ...base,
        tone: "action",
        tool_status: "running",
        kind: "toolgen",
      }),
    ).toBe(false);
  });
});

describe("stream retention split", () => {
  const thought = (id: string): Activity => ({
    id,
    sessionId: "s",
    tone: "thinking",
    kind: "free_talk",
    title: "",
    detail: id,
    createdAt: 1,
  });
  const tool = (id: string, tool_status: string): Activity => ({
    ...activity(id, tool_status),
    id,
  });

  it("retains the latest thought run plus trailing tools", () => {
    const items = [
      thought("t1"),
      tool("a1", "completed"),
      thought("t2"),
      tool("a2", "completed"),
      tool("a3", "running"),
    ];
    const ret = computeStreamRetention(items);
    expect(ret.retained.map((a) => a.id)).toEqual(["t2", "a2", "a3"]);
    expect(ret.thought?.id).toBe("t2");
    expect(ret.isRetained(items[0])).toBe(false);
    expect(ret.isRetained(items[4])).toBe(true);
    expect(ret.isRetained(null)).toBe(false);
  });

  it("falls back to running tools when no thought exists yet", () => {
    const ret = computeStreamRetention([
      tool("a1", "completed"),
      tool("a2", "running"),
      tool("a3", "background_running"),
    ]);
    expect(ret.retained.map((a) => a.id)).toEqual(["a2", "a3"]);
    expect(ret.thought).toBeNull();
  });

  it("moves old tools into the frame on a thoughtless response, retaining thought", () => {
    const items = [thought("t1"), tool("a1", "completed"), null, tool("a2", "completed")];
    const ret = computeStreamRetention(items, 2);
    expect(ret.retained.map(a => a.id)).toEqual(["t1", "a2"]);
    expect(ret.isRetained(items[1])).toBe(false);
    expect(ret.thought?.id).toBe("t1");
  });

  it("keeps current completed tools without any thought, using response boundaries", () => {
    const items = [null, tool("a1", "completed"), null, tool("a2", "completed")];
    expect(computeStreamRetention(items, 2).retained.map(a => a.id)).toEqual(["a2"]);
  });

  it("keeps everything framed when the list is empty", () => {
    expect(computeStreamRetention([]).retained).toEqual([]);
  });

  it("collapses consecutive free-talk snapshots into the latest one", () => {
    const items = [
      { key: "s1", activity: thought("t1") },
      { key: "s2", activity: thought("t2") },
      { key: "n1", activity: null },
      { key: "s3", activity: thought("t3") },
      { key: "a1", activity: tool("a1", "completed") },
    ];
    expect(coalesceFreeTalkItems(items).map((item) => item.key)).toEqual([
      "s2",
      "n1",
      "s3",
      "a1",
    ]);
  });
});
