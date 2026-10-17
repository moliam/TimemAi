// High-frequency composer typing stays local to the composer field. The
// scheduler throttles write-through into thread-level draft state so one
// keystroke no longer rerenders the whole chat UI, while capping how far the
// persisted copy can lag behind the live text.

export interface ComposerWriteThroughOptions {
  commit: (text: string) => void;
  now: () => number;
  schedule: (callback: () => void, delayMs: number) => number;
  cancel: (id: number) => void;
  intervalMs: number;
}

export interface ComposerWriteThrough {
  onInput(text: string): void;
  flush(): void;
  reset(): void;
  dispose(): void;
}

export function createComposerWriteThrough(
  options: ComposerWriteThroughOptions,
): ComposerWriteThrough {
  let pendingId: number | null = null;
  let latest: string | null = null;
  let lastCommitAtMs = 0;

  const fire = () => {
    pendingId = null;
    if (latest === null) return;
    const text = latest;
    latest = null;
    lastCommitAtMs = options.now();
    options.commit(text);
  };

  const reset = () => {
    if (pendingId !== null) {
      options.cancel(pendingId);
      pendingId = null;
    }
    latest = null;
  };

  return {
    // The pending timer is deliberately not reset by newer keystrokes, so
    // continuous typing still commits at most intervalMs behind.
    onInput(text) {
      latest = text;
      if (pendingId !== null) return;
      const waitMs = Math.max(
        0,
        options.intervalMs - (options.now() - lastCommitAtMs),
      );
      pendingId = options.schedule(fire, waitMs);
    },
    // Commit the newest uncommitted text immediately (submit, teardown).
    flush() {
      if (pendingId !== null) {
        options.cancel(pendingId);
        pendingId = null;
      }
      if (latest === null) return;
      fire();
    },
    // Drop pending typing without committing: an authoritative external value
    // is replacing the draft (submit cleared it, rejection refilled it).
    reset,
    dispose: reset,
  };
}
