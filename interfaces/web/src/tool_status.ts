import { t } from "./i18n";

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
// Unknown statuses fall back to the raw wire status; only the two self-made
// words are localized vocabulary, read at render time.
export function humanizeToolStatus(status: string) {
  if (status === "completed") return "✓";
  if (status === "failed") return "✗";
  if (status === TOOL_STATUS_BACKGROUND_RUNNING) return t("tools.statusBg");
  if (status === "timeout") return t("tools.statusTimeout");
  return status.replaceAll("_", " ");
}

// 展开控件与单项结果统一：成功 ✓、失败 ✗，不能用字母 x 替换失败符号。
// 零失败不追加失败计数；控件的 + / − 只表达展开状态，不表达工具执行结果。
// Stream tool rows show wall-clock cost: 0.1s precision under a minute, then
// compact m/s (5m6s); hours drop the seconds. Kept separate from wait timers,
// whose whole-second form serves pending budgets and completion facts.
export function formatToolElapsed(elapsedMs: number) {
  const seconds = Math.max(0, Math.round(elapsedMs / 100)) / 10;
  if (seconds < 60) return `${seconds === 0 ? "0" : seconds.toFixed(1)}s`;
  const total = Math.round(seconds);
  const minutes = Math.floor(total / 60);
  if (minutes < 60) return `${minutes}m${total % 60}s`;
  const hours = Math.floor(minutes / 60);
  return `${hours}h${minutes % 60}m`;
}

export function toolResultCountsLabel(succeeded: number, failed: number) {
  return `${succeeded} ✓${failed > 0 ? ` | ${failed} ✗` : ""}`;
}
