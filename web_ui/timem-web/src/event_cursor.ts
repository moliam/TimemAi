export function classifyEventSequence(cursor: number, incoming: number): "duplicate" | "next" | "gap" {
  if (!Number.isSafeInteger(incoming) || incoming <= cursor) return "duplicate";
  return incoming === cursor + 1 ? "next" : "gap";
}
