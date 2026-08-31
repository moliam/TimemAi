export function classifyEventSequence(cursor: number, incoming: number): "duplicate" | "next" | "gap" {
  if (!Number.isSafeInteger(incoming) || incoming <= cursor) return "duplicate";
  return incoming === cursor + 1 ? "next" : "gap";
}

/**
 * A Hello snapshot is a complete authoritative replacement. Its cursor is the
 * exact baseline for the new connection; no cursor from an older connection
 * may leak across it. Invalid protocol values fail closed to the legacy zero
 * baseline and will be caught if a sequenced event cannot follow it.
 */
export function snapshotEventBaseline(cursor: number | undefined): number {
  return Number.isSafeInteger(cursor) && (cursor ?? 0) >= 0 ? (cursor ?? 0) : 0;
}
