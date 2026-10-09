export type FrameTaskOptions = {
  run: () => void;
  fallbackMs?: number;
  schedule?: (callback: () => void) => number;
  cancel?: (handle: number) => void;
};

/** Coalesces invalidations by frame, with an optional pending-only fallback. */
export function createFrameTask({
  run,
  fallbackMs,
  schedule = (callback) => window.requestAnimationFrame(callback),
  cancel = (handle) => window.cancelAnimationFrame(handle),
}: FrameTaskOptions) {
  let callback = run;
  let handle: number | undefined;
  let disposed = false;
  let fallback: ReturnType<typeof setTimeout> | undefined;
  let generation = 0;
  const clearPending = () => {
    if (handle !== undefined) cancel(handle);
    if (fallback !== undefined) clearTimeout(fallback);
    handle = undefined;
    fallback = undefined;
  };

  return {
    request() {
      if (disposed || handle !== undefined) return;
      const requestedGeneration = ++generation;
      const flush = () => {
        if (disposed || requestedGeneration !== generation) return;
        // Invalidate the losing callback, including one already dispatched.
        ++generation;
        clearPending();
        callback();
      };
      handle = schedule(flush);
      // Opt in only for layout correctness that must not depend on a display
      // frame. One timer while invalidated; no polling while idle.
      if (fallbackMs !== undefined) fallback = setTimeout(flush, fallbackMs);
    },
    update(next: () => void) {
      callback = next;
    },
    pending() {
      return handle !== undefined;
    },
    dispose() {
      disposed = true;
      ++generation;
      clearPending();
    },
  };
}

export type FrameTask = ReturnType<typeof createFrameTask>;
