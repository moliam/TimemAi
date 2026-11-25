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

/**
 * 贴底跟随时，一次提交后滚动位置上移的两种可能：浏览器因上方内容变矮把
 * scrollTop 夹小（应补偿），或用户自己往上滚（必须立刻脱钩、不能拉回）。
 * 用 scrollHeight 是否变矮区分：用户上滚不会让内容变矮。
 */
export type PinnedScrollChange = "none" | "user-scroll-up" | "layout-shrink";

export function classifyPinnedScrollChange(
  previous: ScrollMetrics,
  current: ScrollMetrics,
  epsilon = 1,
): PinnedScrollChange {
  const movedUp = previous.scrollTop - current.scrollTop;
  if (movedUp <= epsilon) return "none";
  return previous.scrollHeight - current.scrollHeight > epsilon
    ? "layout-shrink"
    : "user-scroll-up";
}

/**
 * 是否继续贴底跟随。用户主动上滚会一直保持脱钩，哪怕仍在“底部附近”的阈值内；
 * 只有真正回到底部（或显式点“跳到最新”）才恢复，避免流式内容把慢滚拉回去。
 */
export function resolveFollowLatest(params: {
  holdOff: boolean;
  nearBottom: boolean;
  distanceToBottom: number;
  bottomEpsilon?: number;
}): { follow: boolean; holdOff: boolean } {
  const bottomEpsilon = params.bottomEpsilon ?? 2;
  const holdOff = params.holdOff && params.distanceToBottom > bottomEpsilon;
  return { follow: !holdOff && params.nearBottom, holdOff };
}
