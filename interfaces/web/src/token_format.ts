export function formatTokens(value: number | undefined) {
  if (!value) return value === 0 ? "0" : undefined;
  if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(1)}M`;
  return value >= 1_000
    ? `${(value / 1_000).toFixed(value >= 10_000 ? 0 : 1)}K`
    : String(value);
}

// Three-tier precision for aggregate cache detail labels:
// <1M -> 0.1K steps, 1M..1G -> 0.1M (100K) steps, >=1G -> 0.001G (1M) steps.
export function formatTokensCoarse(value: number | undefined) {
  if (!value) return value === 0 ? "0" : undefined;
  if (value >= 1_000_000_000) return `${(value / 1_000_000_000).toFixed(3)}G`;
  if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(1)}M`;
  if (value >= 1_000) return `${(value / 1_000).toFixed(1)}K`;
  return String(value);
}
