// Content gate for the heavy markdown enhancement chunk (highlight.js +
// KaTeX: several hundred KB of JS plus their vendor CSS). The input is the
// output of normalizeMarkdownMath, where currency dollars are already
// escaped, so any remaining $...$/$$ is a real math candidate. Escaped
// dollars are stripped again so leftover prose stays off the chunk. False
// positives (e.g. dollars inside inline code) merely load the chunk early;
// false negatives would show formula source text, so the patterns stay broad.
const CODE_FENCE = /(^|\n) {0,3}(`{3,}|~{3,})/;
const MATH_DELIMITER = /\$\$|\$[^\n$]+\$|\\\(|\\\[|\[\/(?:math|inline)\]/;

export function markdownNeedsHeavyEnhancement(normalizedMarkdown: string): boolean {
  if (CODE_FENCE.test(normalizedMarkdown)) return true;
  return MATH_DELIMITER.test(normalizedMarkdown.replace(/\\\$/g, ""));
}
