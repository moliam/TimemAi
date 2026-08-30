export function formatTokens(value: number | undefined) {
  if (!value) return value === 0 ? "0" : undefined;
  if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(1)}M`;
  return value >= 1_000
    ? `${(value / 1_000).toFixed(value >= 10_000 ? 0 : 1)}K`
    : String(value);
}
