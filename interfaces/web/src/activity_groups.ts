import { Activity } from "./protocol";
import { toolActivityDisplayName } from "./view_model";
import {
  isToolActivityFailed,
  isToolActivityRunning,
  TOOL_STATUS_RUNNING,
} from "./tool_status";

/** True while a tool call has started but not reached a terminal status. */
export function isRunningToolActivity(
  activity: Activity | null | undefined,
): boolean {
  return (
    !!activity &&
    activity.tone === "action" &&
    activity.kind !== "toolgen" &&
    isToolActivityRunning(activity.tool_status || TOOL_STATUS_RUNNING)
  );
}

export type ToolActivityGroupStatus = "running" | "failed" | "completed";

export type ToolActivityCount = {
  name: string;
  count: number;
};

export type ToolActivitySummary = {
  label: string;
  counts: ToolActivityCount[];
  status: ToolActivityGroupStatus;
  foregroundRunningCount: number;
  backgroundRunningCount: number;
  failedCount: number;
  completedCount: number;
  activities: Activity[];
};

export function summarizeToolActivities(activities: Activity[]): ToolActivitySummary | null {
  const tools = activities.filter((activity) => activity.tone === "action");
  if (tools.length === 0) return null;

  const counts = new Map<string, number>();
  for (const activity of tools) {
    const name = toolActivityDisplayName(activity.tool_name || activity.title || "Tool", activity.tool_mode);
    counts.set(name, (counts.get(name) ?? 0) + 1);
  }

  const statuses = tools.map((activity) => activity.tool_status || "running");
  const foregroundRunningCount = statuses.filter(
    (status) => status === "running",
  ).length;
  const backgroundRunningCount = statuses.filter(
    (status) => status === "background_running",
  ).length;
  const failedCount = statuses.filter(isToolActivityFailed).length;
  const completedCount = tools.length
    - foregroundRunningCount
    - backgroundRunningCount
    - failedCount;
  const status = statuses.some(isToolActivityRunning)
    ? "running"
    : failedCount > 0
      ? "failed"
      : "completed";

  const countItems = [...counts].map(([name, count]) => ({
    name: name.toLocaleLowerCase(),
    count,
  }));
  return {
    label: countItems.map(({ name, count }) => `${name} ${count}`).join(" | "),
    counts: countItems,
    status,
    foregroundRunningCount,
    backgroundRunningCount,
    failedCount,
    completedCount,
    activities: tools,
  };
}


export type ToolActivityRun = {
  startIndex: number;
  summary: ToolActivitySummary;
};

/**
 * Groups each consecutive run of tool activities independently.
 *
 * Visible non-tool activities, such as free-talk/thought updates, close the
 * current run. Null entries are ignored because they represent events that do
 * not render in the work stream and should not split adjacent tool lifecycle
 * events.
 */
export function summarizeConsecutiveToolActivities(
  activities: readonly (Activity | null)[],
): ToolActivityRun[] {
  const runs: ToolActivityRun[] = [];
  let startIndex = -1;
  let tools: Activity[] = [];

  const flush = () => {
    if (startIndex < 0 || tools.length === 0) return;
    const summary = summarizeToolActivities(tools);
    if (summary) runs.push({ startIndex, summary });
    startIndex = -1;
    tools = [];
  };

  activities.forEach((activity, index) => {
    if (activity === null) return;
    if (activity.tone === "action") {
      if (startIndex < 0) startIndex = index;
      tools.push(activity);
      return;
    }
    flush();
  });
  flush();

  return runs;
}

export type StreamRetention = {
  /** Non-null activities kept in the live stream area, in order. */
  retained: Activity[];
  /** Latest thought snapshot kept in the stream; null when none exists. */
  thought: Activity | null;
  /** Historical frame membership is independent of the live thought fallback. */
  isFramed: (activity: Activity | null | undefined) => boolean;
  /** Membership test by object identity; nulls are never retained. */
  isRetained: (activity: Activity | null | undefined) => boolean;
};

/** All rounds remain in place until the caller archives the confirmed final answer. */
export function computeStreamRetention(
  activities: readonly (Activity | null)[],
): StreamRetention {
  const retained = activities.filter((activity): activity is Activity => !!activity &&
    (activity.kind === "free_talk" || activity.tone === "action" || activity.kind === "user_supplement"));
  const retainedSet = new Set(retained);
  return {
    retained,
    thought: [...retained].reverse().find(activity => activity.kind === "free_talk") ?? null,
    isFramed: activity => !!activity && !retainedSet.has(activity),
    isRetained: activity => !!activity && retainedSet.has(activity),
  };
}

/**
 * Collapses each consecutive run of free-talk snapshots (null activities are
 * transparent) into its latest item, so both the frame and the stream show
 * one growing thought instead of stacking redundant snapshots.
 */
export function coalesceFreeTalkItems<T extends { activity: Activity | null }>(
  items: readonly T[],
): T[] {
  const out: T[] = [];
  let run: T[] = [];
  const flush = () => {
    if (run.length === 0) return;
    const thoughts = run.filter((item) => item.activity !== null);
    out.push(...run.filter((item) => item.activity === null));
    if (thoughts.length > 0) out.push(thoughts[thoughts.length - 1]);
    run = [];
  };
  for (const item of items) {
    if (item.activity?.kind === "free_talk") run.push(item);
    else { flush(); out.push(item); }
  }
  flush();
  return out;
}
