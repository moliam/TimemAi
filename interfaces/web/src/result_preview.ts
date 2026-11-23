// Result preview tail contract shared by the tool row and its tests: the
// read-only preview box always shows at most the last
// RESULT_PREVIEW_TAIL_LINES lines of the tool's own result. The Host already
// bounds the payload (tail lines + bytes); this slice only re-enforces the
// line contract on the client.
export const RESULT_PREVIEW_TAIL_LINES = 16;

export function resultTailLines(
  text: string,
  max: number = RESULT_PREVIEW_TAIL_LINES,
): string[] {
  const normalized = text.replace(/\r\n?/g, "\n");
  const lines = normalized.split("\n");
  while (lines.length > 1 && lines[lines.length - 1].trim() === "") lines.pop();
  return lines.slice(-max);
}
