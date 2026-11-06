/** Maximum characters rendered in the inline thinking window. The strip
 * itself may clip narrower; it always renders the newest suffix. */
export const THINKING_INLINE_CHARS = 96;

/** Reasoning deltas may contain line breaks and repeated spacing; the inline
 * strip needs one dense line so its width cannot jump on every delta. */
export function collapseThinkingText(text: string): string {
  return text.replace(/\s+/g, " ").trim();
}

/** Keep the newest characters. The leading ellipsis only appears when the
 * retained tail itself was compacted, never when CSS clips the strip. */
export function compactThinkingTail(
  text: string,
  maxChars = THINKING_INLINE_CHARS,
): string {
  const collapsed = collapseThinkingText(text);
  if (!collapsed) return "";
  const chars = Array.from(collapsed);
  return chars.length > maxChars
    ? `…${chars.slice(-maxChars).join("")}`
    : collapsed;
}
