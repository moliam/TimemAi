export type FrameScheduler = (callback: () => void) => number;
export type FrameCanceller = (handle: number) => void;

export interface FrameEventQueueOptions<T> {
  consume: (items: readonly T[]) => void;
  schedule?: FrameScheduler;
  cancel?: FrameCanceller;
  now?: () => number;
  maxBatch?: number;
  budgetMs?: number;
}

export interface FrameEventQueue<T> {
  enqueue(item: T, flushImmediately?: boolean): void;
  dispose(): void;
  pending(): number;
}

export function createFrameEventQueue<T>({
  consume,
  schedule = (callback) => requestAnimationFrame(callback),
  cancel = (handle) => cancelAnimationFrame(handle),
  now = () => performance.now(),
  maxBatch = 24,
  budgetMs = 6,
}: FrameEventQueueOptions<T>): FrameEventQueue<T> {
  const queue: T[] = [];
  let scheduled: number | null = null;
  let disposed = false;

  const requestFlush = () => {
    if (disposed || scheduled !== null) return;
    scheduled = schedule(flush);
  };

  const flush = () => {
    scheduled = null;
    if (disposed || queue.length === 0) return;
    const started = now();
    const batch: T[] = [];
    while (queue.length > 0 && batch.length < maxBatch) {
      batch.push(queue.shift()!);
      if (batch.length > 0 && now() - started >= budgetMs) break;
    }
    consume(batch);
    if (queue.length > 0) requestFlush();
  };

  return {
    enqueue(item, flushImmediately = false) {
      if (disposed) return;
      queue.push(item);
      if (flushImmediately) {
        if (scheduled !== null) cancel(scheduled);
        scheduled = null;
        while (!disposed && queue.length > 0) {
          flush();
          if (scheduled !== null) cancel(scheduled);
          scheduled = null;
        }
      } else {
        requestFlush();
      }
    },
    dispose() {
      disposed = true;
      queue.length = 0;
      if (scheduled !== null) cancel(scheduled);
      scheduled = null;
    },
    pending: () => queue.length,
  };
}
