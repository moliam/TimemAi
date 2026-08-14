const STORAGE_PREFIX = "timem-web-event-cursor:v1";

export function eventCursorStorageKey(scope: string) {
  return `${STORAGE_PREFIX}:${encodeURIComponent(scope)}`;
}

export function loadEventCursor(storage: Pick<Storage, "getItem">, scope: string) {
  try {
    const value = Number(storage.getItem(eventCursorStorageKey(scope)));
    return Number.isSafeInteger(value) && value >= 0 ? value : 0;
  } catch {
    return 0;
  }
}

export function saveEventCursor(storage: Pick<Storage, "setItem">, scope: string, cursor: number) {
  if (!scope || !Number.isSafeInteger(cursor) || cursor < 0) return false;
  try {
    storage.setItem(eventCursorStorageKey(scope), String(cursor));
    return true;
  } catch {
    return false;
  }
}

export function classifyEventSequence(cursor: number, incoming: number): "duplicate" | "next" | "gap" {
  if (!Number.isSafeInteger(incoming) || incoming <= cursor) return "duplicate";
  return incoming === cursor + 1 ? "next" : "gap";
}

export function resolveHelloEventCursor(
  previousScope: string,
  nextScope: string,
  restoredCursor: number,
  helloCursor: number | undefined,
  replayFloor: number | undefined = 0,
) {
  const serverCursor = Number.isSafeInteger(helloCursor) && (helloCursor ?? 0) >= 0 ? helloCursor ?? 0 : 0;
  const serverFloor = Number.isSafeInteger(replayFloor) && (replayFloor ?? 0) >= 0 ? replayFloor ?? 0 : 0;
  if (restoredCursor < serverFloor || restoredCursor > serverCursor) {
    return { cursor: serverCursor, reconnectForReplay: false };
  }
  if (previousScope === nextScope) return { cursor: restoredCursor, reconnectForReplay: false };
  if (restoredCursor === 0) return { cursor: serverCursor, reconnectForReplay: false };
  if (restoredCursor < serverCursor) return { cursor: restoredCursor, reconnectForReplay: true };
  // A cursor ahead of the same mem's server journal means the journal was
  // recreated. The snapshot is authoritative and becomes the new baseline.
  return { cursor: serverCursor, reconnectForReplay: false };
}
