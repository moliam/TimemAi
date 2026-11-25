// 输入框左侧上下箭头的鼠标悬停历史面板：数据只从权威 Turn 投影派生，
// 顺序必须与 `[data-user-message-anchor]` 一一对应（每个可见用户气泡一条），
// 这样点击列表项就能直接复用现有的 anchor 滚动逻辑，不做第二套导航状态。
import type { WebTurn, WebTurnUserEntry } from "./protocol";

/** 面板里单条预览的最大字符数；完整文本留在 title 中。 */
export const USER_MESSAGE_HISTORY_PREVIEW_CHARS = 96;

const HIDDEN_USER_ENTRY_KINDS = new Set(["approval", "resume_directly"]);
const SYSTEM_USER_ENTRY_KINDS = new Set([
  "mailbox",
  "mailbox_status",
  "timer",
  "resource_observation",
]);

export function isVisibleUserEntry(entry: WebTurnUserEntry): boolean {
  return !HIDDEN_USER_ENTRY_KINDS.has(entry.kind);
}

/** 系统触发（mailbox/timer/资源观察）的用户气泡不是用户本人输入。 */
export function isSystemTriggeredUserEntry(entry: WebTurnUserEntry): boolean {
  return (
    SYSTEM_USER_ENTRY_KINDS.has(entry.kind) || entry.text.startsWith("[mailbox] ")
  );
}

export type UserMessageHistoryItem = {
  turnId: string;
  /** 完整文本，用于 title 与无障碍说明。 */
  text: string;
  /** 单行折叠后的预览文本。 */
  preview: string;
  createdAtMs: number;
  /** 该气泡内的可见条目数（补充输入会让它大于 1）。 */
  entryCount: number;
};

/**
 * 与 `TimemThread` 渲染用户气泡的条件保持一致：approval/resume_directly
 * 不渲染，所以也不能占用一条历史记录，否则点击索引会错位。
 */
export function userMessageHistoryItems(
  turns: readonly WebTurn[],
): UserMessageHistoryItem[] {
  const items: UserMessageHistoryItem[] = [];
  for (const turn of turns) {
    const entries = turn.user_entries.filter(isVisibleUserEntry);
    if (entries.length === 0) continue;
    const humanEntries = entries.filter(
      (entry) => !isSystemTriggeredUserEntry(entry),
    );
    // 混有真人输入时只展示真人部分；纯系统触发的轮次退化为通知文本，
    // 仍保留一条可跳转的目标，避免索引与 anchor 数不一致。
    const primary = humanEntries.length > 0 ? humanEntries : entries;
    const text = primary
      .map((entry) => entry.text.trim())
      .filter(Boolean)
      .join("\n\n");
    items.push({
      turnId: turn.turn_id,
      text,
      preview: previewUserMessageText(text),
      createdAtMs: primary[0]?.created_at_ms || turn.created_at_ms,
      entryCount: entries.length,
    });
  }
  return items;
}

/** 压平换行与连续空白，按字符（而非 UTF-16 码元）截断。 */
export function previewUserMessageText(
  text: string,
  maxChars: number = USER_MESSAGE_HISTORY_PREVIEW_CHARS,
): string {
  const flattened = text.replace(/\s+/gu, " ").trim();
  if (!flattened) return "";
  const chars = Array.from(flattened);
  if (chars.length <= maxChars) return flattened;
  return `${chars.slice(0, maxChars).join("")}…`;
}
