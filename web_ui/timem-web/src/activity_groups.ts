import { Activity } from "./protocol";
import { toolDisplayName } from "./view_model";

export type ToolActivityGroupStatus = "running" | "failed" | "completed";

export type ToolActivityCount = {
  name: string;
  count: number;
};

export type ToolActivitySummary = {
  label: string;
  counts: ToolActivityCount[];
  status: ToolActivityGroupStatus;
  activities: Activity[];
};

const RUNNING_STATUSES = new Set(["running", "background_running"]);
const FAILED_STATUSES = new Set(["error", "failed", "timeout", "cancelled", "cancelled_by_user"]);

export function summarizeToolActivities(activities: Activity[]): ToolActivitySummary | null {
  const tools = activities.filter((activity) => activity.tone === "action");
  if (tools.length === 0) return null;

  const counts = new Map<string, number>();
  for (const activity of tools) {
    const name = toolDisplayName(activity.tool_name || activity.title || "Tool");
    counts.set(name, (counts.get(name) ?? 0) + 1);
  }

  const statuses = tools.map((activity) => activity.tool_status || "running");
  const status = statuses.some((value) => RUNNING_STATUSES.has(value))
    ? "running"
    : statuses.some((value) => FAILED_STATUSES.has(value))
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
