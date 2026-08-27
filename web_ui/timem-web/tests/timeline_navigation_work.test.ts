import { describe, expect, it, vi } from "vitest";
import { createFrameTask } from "../src/frame_task";
import { requestTimelineNavigationWork } from "../src/timeline_navigation_work";

function workSpies() {
  return {
    navigation: { request: vi.fn() },
    geometry: { request: vi.fn() },
    layout: { request: vi.fn() },
  };
}

describe("timeline navigation work", () => {
  it("keeps a scroll storm away from geometry and floating-layout measurement", () => {
    const work = workSpies();
    for (let index = 0; index < 50_000; index += 1) {
      requestTimelineNavigationWork("scroll", work);
    }
    expect(work.navigation.request).toHaveBeenCalledTimes(50_000);
    expect(work.geometry.request).not.toHaveBeenCalled();
    expect(work.layout.request).not.toHaveBeenCalled();
  });

  it("refreshes offsets for content changes and all geometry on layout invalidation", () => {
    const work = workSpies();
    requestTimelineNavigationWork("content", work);
    expect(work.geometry.request).toHaveBeenCalledTimes(1);
    expect(work.layout.request).not.toHaveBeenCalled();

    requestTimelineNavigationWork("layout", work);
    expect(work.geometry.request).toHaveBeenCalledTimes(2);
    expect(work.layout.request).toHaveBeenCalledTimes(1);
  });

  it("coalesces a scroll storm into one navigation callback per frame", () => {
    const scheduled: Array<() => void> = [];
    const navigationRun = vi.fn();
    const geometryRun = vi.fn();
    const layoutRun = vi.fn();
    const task = (run: () => void) => createFrameTask({
      run,
      schedule: (callback) => { scheduled.push(callback); return scheduled.length; },
      cancel: () => undefined,
    });
    const work = {
      navigation: task(navigationRun),
      geometry: task(geometryRun),
      layout: task(layoutRun),
    };

    for (let index = 0; index < 50_000; index += 1) {
      requestTimelineNavigationWork("scroll", work);
    }
    expect(scheduled).toHaveLength(1);
    scheduled.shift()?.();
    expect(navigationRun).toHaveBeenCalledTimes(1);
    expect(geometryRun).not.toHaveBeenCalled();
    expect(layoutRun).not.toHaveBeenCalled();
  });
});
