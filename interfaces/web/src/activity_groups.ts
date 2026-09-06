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
  /** Membership test by object identity; nulls are never retained. */
  isRetained: (activity: Activity | null | undefined) => boolean;
};

/**
 * Stream UI Mode retention rule: the latest model-thought run (consecutive
 * free-talk snapshots plus every tool activity after it) stays in the live
 * stream area until the next thought run arrives, at which point the previous
 * run migrates into the Thought/Action frame. Without any thought yet, only
 * running tools stay in the stream. Terminal turns keep everything framed.
 */
export function computeStreamRetention(
  activities: readonly (Activity | null)[],
): StreamRetention {
  const dense = activities.filter((a): a is Activity => a !== null);
  let lastThoughtIndex = -1;
  for (let i = dense.length - 1; i >= 0; i -= 1) {
    if (dense[i].kind === "free_talk") { lastThoughtIndex = i; break; }
  }
  let runStart = lastThoughtIndex;
  while (runStart > 0 && dense[runStart - 1].kind === "free_talk") runStart -= 1;

  const retainedSet = new Set<Activity>();
  let thought: Activity | null = null;
  if (lastThoughtIndex >= 0) {
    for (let i = runStart; i < dense.length; i += 1) retainedSet.add(dense[i]);
    thought = dense[lastThoughtIndex];
  } else {
    for (const activity of dense)
      if (isRunningToolActivity(activity)) retainedSet.add(activity);
  }
  const retained = dense.filter((a) => retainedSet.has(a));
  return {
    retained,
    thought,
    isRetained: (activity) => !!activity && retainedSet.has(activity),
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
