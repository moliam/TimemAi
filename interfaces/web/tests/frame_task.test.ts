import { afterEach, describe, expect, it, vi } from "vitest";
import { createFrameTask } from "../src/frame_task";

describe("frame task", () => {
  afterEach(() => vi.useRealTimers());

  it("runs pending layout work without frames and cancels the losing frame", () => {
    vi.useFakeTimers();
    const frames: Array<() => void> = [];
    const cancel = vi.fn();
    const callback = vi.fn();
    const task = createFrameTask({
      run: callback,
      schedule: frame => { frames.push(frame); return frames.length; },
      cancel,
      fallbackMs: 100,
    });
    for (let i = 0; i < 100; i++) task.request();
    expect(frames).toHaveLength(1);
    expect(vi.getTimerCount()).toBe(1);
    vi.advanceTimersByTime(100);
    expect(callback).toHaveBeenCalledTimes(1);
    expect(cancel).toHaveBeenCalledWith(1);
    expect(task.pending()).toBe(false);
    expect(vi.getTimerCount()).toBe(0);
    // Even an already-delivered losing callback cannot run this task twice,
    // or consume a subsequent request.
    task.request();
    frames[0]();
    expect(callback).toHaveBeenCalledTimes(1);
    expect(task.pending()).toBe(true);
    frames[1]();
    expect(callback).toHaveBeenCalledTimes(2);
    expect(vi.getTimerCount()).toBe(0);
    task.dispose();
  });

  it("cancels fallback on frame completion and disposal without idle polling", () => {
    vi.useFakeTimers();
    const frames: Array<() => void> = [];
    const callback = vi.fn();
    const task = createFrameTask({
      run: callback,
      schedule: frame => { frames.push(frame); return frames.length; },
      cancel: vi.fn(),
      fallbackMs: 100,
    });
    expect(vi.getTimerCount()).toBe(0);
    task.request();
    frames[0]();
    expect(vi.getTimerCount()).toBe(0);
    task.request();
    task.dispose();
    frames[1]();
    vi.advanceTimersByTime(1000);
    expect(callback).toHaveBeenCalledTimes(1);
    expect(vi.getTimerCount()).toBe(0);
    task.request();
    expect(task.pending()).toBe(false);
  });

  it("coalesces repeated requests into one callback per frame", () => {
    const frames: Array<() => void> = [];
    const callback = vi.fn();
    const task = createFrameTask({
      run: callback,
      schedule: (frame) => { frames.push(frame); return frames.length; },
      cancel: vi.fn(),
    });

    task.request();
    task.request();
    task.request();
    expect(frames).toHaveLength(1);
    frames.shift()?.();
    expect(callback).toHaveBeenCalledTimes(1);

    task.request();
    expect(frames).toHaveLength(1);
  });

  it("runs the latest callback and cancels pending work on dispose", () => {
    const frames: Array<() => void> = [];
    const cancel = vi.fn();
    const first = vi.fn();
    const second = vi.fn();
    const task = createFrameTask({
      run: first,
      schedule: (frame) => { frames.push(frame); return 42; },
      cancel,
    });

    task.request();
    task.update(second);
    frames.shift()?.();
    expect(first).not.toHaveBeenCalled();
    expect(second).toHaveBeenCalledTimes(1);

    task.request();
    task.dispose();
    expect(cancel).toHaveBeenCalledWith(42);
    expect(task.pending()).toBe(false);
  });
});
