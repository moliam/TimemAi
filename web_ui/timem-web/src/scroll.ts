export type ScrollMetrics = {
  scrollTop: number;
  scrollHeight: number;
};

export type SessionScrollPosition = {
  scrollTop: number;
  followLatest: boolean;
};

export function preservePrependScrollTop(previous: ScrollMetrics, nextScrollHeight: number) {
  return Math.max(0, previous.scrollTop + Math.max(0, nextScrollHeight - previous.scrollHeight));
}

export function isNearScrollBottom(metrics: ScrollMetrics & { clientHeight: number }, threshold = 72) {
  return metrics.scrollHeight - metrics.scrollTop - metrics.clientHeight <= threshold;
}

export function restoreSessionScrollTop(position: SessionScrollPosition | undefined, scrollHeight: number) {
  if (!position || position.followLatest) return scrollHeight;
  return Math.max(0, Math.min(position.scrollTop, scrollHeight));
}
