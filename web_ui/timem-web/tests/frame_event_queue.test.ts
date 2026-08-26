import { describe, expect, it } from "vitest";
import { createFrameEventQueue } from "../src/frame_event_queue";

function manualFrames() {
  const callbacks = new Map<number, () => void>();
  let next = 1;
  return {
    schedule(callback: () => void) {
      const id = next++;
      callbacks.set(id, callback);
      return id;
    },
    cancel(id: number) { callbacks.delete(id); },
    runOne() {
      const entry = callbacks.entries().next().value as [number, () => void] | undefined;
      if (!entry) return false;
      callbacks.delete(entry[0]);
      entry[1]();
      return true;
    },
    size: () => callbacks.size,
  };
}

describe("frame event queue", () => {
  it("preserves order and drains a flood over bounded animation frames", () => {
    const frames = manualFrames();
    const consumed: number[] = [];
    const batches: number[] = [];
    const queue = createFrameEventQueue<number>({
      consume(items) {
        batches.push(items.length);
        consumed.push(...items);
      },
      schedule: frames.schedule,
      cancel: frames.cancel,
      now: () => 0,
      maxBatch: 3,
    });
    for (let value = 0; value < 8; value += 1) queue.enqueue(value);
    expect(frames.size()).toBe(1);
    while (frames.runOne()) { /* drain */ }
    expect(batches).toEqual([3, 3, 2]);
    expect(consumed).toEqual([0, 1, 2, 3, 4, 5, 6, 7]);
    expect(queue.pending()).toBe(0);
  });

  it("yields when the frame time budget is exhausted", () => {
    const frames = manualFrames();
    const batches: number[][] = [];
    let clock = 0;
    const queue = createFrameEventQueue<number>({
      consume: (items) => batches.push([...items]),
      schedule: frames.schedule,
      cancel: frames.cancel,
      now: () => clock++,
      maxBatch: 50,
      budgetMs: 2,
    });
    [1, 2, 3, 4].forEach((item) => queue.enqueue(item));
    frames.runOne();
    expect(batches[0]).toEqual([1, 2]);
    expect(queue.pending()).toBe(2);
    frames.runOne();
    expect(batches.flat()).toEqual([1, 2, 3, 4]);
  });

  it("flushes live progress immediately without reordering earlier events", () => {
    const frames = manualFrames();
    const batches: number[][] = [];
    const queue = createFrameEventQueue<number>({
      consume: (items) => batches.push([...items]),
      schedule: frames.schedule,
      cancel: frames.cancel,
      now: () => 0,
    });
    for (let value = 1; value <= 30; value += 1) queue.enqueue(value);
    queue.enqueue(31, true);
    expect(batches).toEqual([
      Array.from({ length: 24 }, (_, index) => index + 1),
      Array.from({ length: 7 }, (_, index) => index + 25),
    ]);
    expect(frames.size()).toBe(0);
    expect(queue.pending()).toBe(0);
  });

  it("drops pending UI work after disposal", () => {
    const frames = manualFrames();
    const consumed: number[] = [];
    const queue = createFrameEventQueue<number>({
      consume: (items) => consumed.push(...items),
      schedule: frames.schedule,
      cancel: frames.cancel,
    });
    queue.enqueue(1);
    queue.dispose();
    queue.enqueue(2);
    expect(frames.size()).toBe(0);
    expect(consumed).toEqual([]);
  });
});
