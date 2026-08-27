export type FrameTaskOptions = {
  run: () => void;
  schedule?: (callback: () => void) => number;
  cancel?: (handle: number) => void;
};

/** Coalesces any number of invalidations into at most one callback per frame. */
export function createFrameTask({
  run,
  schedule = (callback) => window.requestAnimationFrame(callback),
  cancel = (handle) => window.cancelAnimationFrame(handle),
}: FrameTaskOptions) {
  let callback = run;
  let handle: number | undefined;
  let disposed = false;

  return {
    request() {
      if (disposed || handle !== undefined) return;
      handle = schedule(() => {
        handle = undefined;
        if (!disposed) callback();
      });
    },
    update(next: () => void) {
      callback = next;
    },
    pending() {
      return handle !== undefined;
    },
    dispose() {
      disposed = true;
      if (handle !== undefined) cancel(handle);
      handle = undefined;
    },
  };
}

export type FrameTask = ReturnType<typeof createFrameTask>;
