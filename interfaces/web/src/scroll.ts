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

/**
 * 自动工具收纳时的局部滚动锚定。收纳会把「折叠了多少像素」整段移走；只要用户
 * 没在跟底部、自己也没滚动，就把这段高度差反向补进 scrollTop，让组下方正在
 * 阅读的内容原地不动，而不是整屏上下跳。返回 null 表示本次不应补偿。
 */
export function foldAnchorScrollTop(input: {
  previousBottom: number;
  bottom: number;
  scrollTop: number;
  previousScrollTop: number;
  scrollHeight: number;
  clientHeight: number;
  threshold?: number;
}): number | null {
  const delta = input.bottom - input.previousBottom;
  if (Math.abs(delta) <= 1) return null;
  if (isNearScrollBottom(input, input.threshold)) return null;
  if (Math.abs(input.scrollTop - input.previousScrollTop) > 1) return null;
  return Math.max(0, input.scrollTop + delta);
}

export function isNearScrollBottom(metrics: ScrollMetrics & { clientHeight: number }, threshold = 72) {
  return metrics.scrollHeight - metrics.scrollTop - metrics.clientHeight <= threshold;
}

export function scrollEdgeFades(metrics: ScrollMetrics & { clientHeight: number }, epsilon = 1) {
  const threshold = Math.max(0, epsilon);
  const maxScrollTop = Math.max(0, metrics.scrollHeight - metrics.clientHeight);
  if (maxScrollTop <= threshold) return { top: false, bottom: false };
  const scrollTop = Math.max(0, Math.min(metrics.scrollTop, maxScrollTop));
  return {
    top: scrollTop > threshold,
    bottom: scrollTop < maxScrollTop - threshold,
  };
}

export function restoreSessionScrollTop(position: SessionScrollPosition | undefined, scrollHeight: number) {
  if (!position || position.followLatest) return scrollHeight;
  return Math.max(0, Math.min(position.scrollTop, scrollHeight));
}

export function canScrollInDirection(
  metrics: ScrollMetrics & { clientHeight: number },
  deltaY: number,
  epsilon = 1,
) {
  const maxScrollTop = Math.max(0, metrics.scrollHeight - metrics.clientHeight);
  if (deltaY < 0) return metrics.scrollTop > epsilon;
  if (deltaY > 0) return metrics.scrollTop < maxScrollTop - epsilon;
  return false;
}

export function wheelDeltaPixels(deltaY: number, deltaMode: number, clientHeight: number) {
  if (deltaMode === 1) return deltaY * 16;
  if (deltaMode === 2) return deltaY * Math.max(1, clientHeight);
  return deltaY;
}

export type UserMessageNavigationDirection = "previous" | "next";

export function adjacentUserMessageIndex(
  anchorTops: readonly number[],
  referenceTop: number,
  direction: UserMessageNavigationDirection,
  epsilon = 4,
) {
  if (direction === "previous") {
    for (let index = anchorTops.length - 1; index >= 0; index -= 1) {
      if (anchorTops[index] < referenceTop - epsilon) return index;
    }
    return -1;
  }
  return anchorTops.findIndex((top) => top > referenceTop + epsilon);
}

export type ViewportTurnAnchor = { turnId: string; viewportTop: number };

/**
 * First restored turn whose bottom edge reaches the viewport. Anchoring its
 * top pins the reading position while later turns hydrate and grow: a
 * hydration commit that expands content above the anchored turn is cancelled
 * by an equal scrollTop correction, and expansion below it needs no change.
 */
export function pickViewportTurnAnchor(
  turns: ReadonlyArray<{ turnId: string; top: number; bottom: number }>,
  viewportTop: number,
  epsilon = 1,
): ViewportTurnAnchor | null {
  for (const turn of turns) {
    if (turn.bottom > viewportTop + epsilon) {
      return { turnId: turn.turnId, viewportTop: turn.top - viewportTop };
    }
  }
  return null;
}

/**
 * ScrollTop correction that pins the anchored turn back to its captured
 * viewport offset after a commit moved content above it. Returns 0 when the
 * anchor already sits where the reader left it.
 */
export function anchorScrollDelta(
  anchor: ViewportTurnAnchor,
  elementViewportTop: number,
  epsilon = 1,
): number {
  const delta = elementViewportTop - anchor.viewportTop;
  return Math.abs(delta) <= epsilon ? 0 : delta;
}
