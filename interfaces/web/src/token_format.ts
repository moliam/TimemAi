export function formatTokens(value: number | undefined) {
  if (!value) return value === 0 ? "0" : undefined;
  if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(1)}M`;
  return value >= 1_000
    ? `${(value / 1_000).toFixed(value >= 10_000 ? 0 : 1)}K`
    : String(value);
}

// Three-tier precision for aggregate cache detail labels:
// <1M -> 0.1K steps, 1M..1G -> 0.1M (100K) steps, >=1G -> 0.001G (1M) steps.
// Resident instruction files: byte size for display, plus a rough token
// estimate. 4 bytes per token is the usual English-text ratio; it is a display
// heuristic only and never feeds request budgeting.
export function formatKilobytes(bytes: number | undefined) {
  if (bytes === undefined || !Number.isFinite(bytes) || bytes < 0) return undefined;
  const kb = bytes / 1024;
  return kb >= 100 ? `${Math.round(kb)} KB` : `${kb.toFixed(1)} KB`;
}

export function estimateTokensFromBytes(bytes: number | undefined) {
  if (bytes === undefined || !Number.isFinite(bytes) || bytes <= 0) return 0;
  return Math.round(bytes / 4);
}

export function formatTokensCoarse(value: number | undefined) {
  if (!value) return value === 0 ? "0" : undefined;
  if (value >= 1_000_000_000) return `${(value / 1_000_000_000).toFixed(3)}G`;
  if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(1)}M`;
  if (value >= 1_000) return `${(value / 1_000).toFixed(1)}K`;
  return String(value);
}

// Turn completion status bar: "24K(Think 0.6K)" — the think annotation only
// appears when the turn actually reported reasoning tokens.
export function formatOutputWithThink(
  output: number | undefined,
  think: number | undefined,
) {
  const outputText = formatTokens(output);
  if (!output || !outputText) return undefined;
  const thinkText = think ? formatTokens(think) : undefined;
  return thinkText ? `${outputText}(think ${thinkText})` : outputText;
}
