// 用户气泡时间戳：本地挂钟时间，属于数据呈现而非 Interface chrome 文案，
// 不进入 i18n 目录（与 model 输出、用户数据同等对待）。
const formatter = new Intl.DateTimeFormat(undefined, {
  hour: "2-digit",
  minute: "2-digit",
});

/** Local wall-clock hour:minute for a user entry timestamp. */
export function formatUserEntryTime(value: number): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "";
  return formatter.format(date);
}
