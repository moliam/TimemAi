// 用户气泡时间戳：本地挂钟时间，属于数据呈现而非 Interface chrome 文案，
// 不进入 i18n 目录（与 model 输出、用户数据同等对待）。
// 分级显示：今天仅时:分；今年非今天补月/日；跨年再补年份。
const timeFormatter = new Intl.DateTimeFormat(undefined, {
  hour: "2-digit",
  minute: "2-digit",
});

const monthDayTimeFormatter = new Intl.DateTimeFormat(undefined, {
  month: "numeric",
  day: "numeric",
  hour: "2-digit",
  minute: "2-digit",
});

const yearMonthDayTimeFormatter = new Intl.DateTimeFormat(undefined, {
  year: "numeric",
  month: "numeric",
  day: "numeric",
  hour: "2-digit",
  minute: "2-digit",
});

/** Local wall-clock timestamp for a user entry; date parts appear once the entry leaves today. */
export function formatUserEntryTime(value: number, now: number = Date.now()): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "";
  const current = new Date(now);
  const sameDay =
    date.getFullYear() === current.getFullYear() &&
    date.getMonth() === current.getMonth() &&
    date.getDate() === current.getDate();
  if (sameDay) return timeFormatter.format(date);
  if (date.getFullYear() === current.getFullYear()) {
    return monthDayTimeFormatter.format(date);
  }
  return yearMonthDayTimeFormatter.format(date);
}
