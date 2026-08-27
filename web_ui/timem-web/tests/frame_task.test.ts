import { describe, expect, it, vi } from "vitest";
import { createFrameTask } from "../src/frame_task";

describe("frame task", () => {
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
