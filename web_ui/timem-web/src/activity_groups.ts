import { Activity } from "./protocol";
import { toolDisplayName } from "./view_model";

export type ToolActivityGroupStatus = "running" | "failed" | "completed";

export type ToolActivitySummary = {
  label: string;
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

  return {
    label: [...counts].map(([name, count]) => `${name} ×${count}`).join(", "),
    status,
    activities: tools,
  };
}
