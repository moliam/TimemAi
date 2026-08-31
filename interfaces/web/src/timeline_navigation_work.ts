export type TimelineNavigationRequest = { request(): void } | null | undefined;

export type TimelineNavigationWork = {
  navigation: TimelineNavigationRequest;
  geometry: TimelineNavigationRequest;
  layout: TimelineNavigationRequest;
};

export type TimelineNavigationInvalidation = "scroll" | "content" | "layout";

/**
 * Keeps the performance trade-off explicit:
 * - scroll may only derive navigation state from cached offsets;
 * - content changes may rebuild cached offsets;
 * - activation/size/outline layout changes may also remeasure floating layout.
 */
export function requestTimelineNavigationWork(
  invalidation: TimelineNavigationInvalidation,
  work: TimelineNavigationWork,
) {
  if (invalidation === "scroll") {
    work.navigation?.request();
    return;
  }
  work.geometry?.request();
  if (invalidation === "layout") work.layout?.request();
}
