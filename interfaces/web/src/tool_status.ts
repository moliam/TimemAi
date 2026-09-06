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

// Visual contract: terminal success/failure use ✓/✗, not redundant word labels.
export function humanizeToolStatus(status: string) {
  if (status === "completed") return "✓";
  if (status === "failed") return "✗";
  if (status === TOOL_STATUS_BACKGROUND_RUNNING) return "running (bg)";
  if (status === "timeout") return "timed out";
  return status.replaceAll("_", " ");
}

// 展开控件与单项结果统一：成功 ✓、失败 ✗，不能用字母 x 替换失败符号。
// 零失败不追加失败计数；控件的 + / − 只表达展开状态，不表达工具执行结果。
export function toolResultCountsLabel(succeeded: number, failed: number) {
  return `${succeeded} ✓${failed > 0 ? ` | ${failed} ✗` : ""}`;
}
