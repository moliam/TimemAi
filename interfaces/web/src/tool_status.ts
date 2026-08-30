export const TOOL_STATUS_RUNNING = "running";
export const TOOL_STATUS_BACKGROUND_RUNNING = "background_running";

const RUNNING_TOOL_STATUSES = new Set([
  TOOL_STATUS_RUNNING,
  TOOL_STATUS_BACKGROUND_RUNNING,
]);
const FAILED_TOOL_STATUSES = new Set([
  "error",
  "failed",
  "timeout",
  "cancelled",
  "cancelled_by_user",
]);

export function isToolActivityRunning(status: string) {
  return RUNNING_TOOL_STATUSES.has(status);
}

export function isToolActivityFailed(status: string) {
  return FAILED_TOOL_STATUSES.has(status);
}

export function humanizeToolStatus(status: string) {
  if (status === TOOL_STATUS_BACKGROUND_RUNNING) return "running (bg)";
  if (status === "timeout") return "timed out";
  return status.replaceAll("_", " ");
}
