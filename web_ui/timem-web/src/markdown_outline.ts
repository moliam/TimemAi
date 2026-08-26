export type MarkdownOutlineItem = {
  id: string;
  level: number;
  title: string;
};

const MAX_OUTLINE_LEVEL = 3;

export function markdownHeadingSlug(title: string) {
  const normalized = title
    .toLocaleLowerCase()
    .replace(/[`*_~\[\]{}()<>]/g, "")
    .replace(/[^\p{Letter}\p{Number}]+/gu, "-")
    .replace(/^-+|-+$/g, "");
  return normalized || "section";
}

export function markdownHeadingId(title: string, occurrences: Map<string, number>) {
  const slug = markdownHeadingSlug(title);
  const count = (occurrences.get(slug) ?? 0) + 1;
  occurrences.set(slug, count);
  return count === 1 ? slug : `${slug}-${count}`;
}

export function markdownHeadingText(source: string) {
  return source
    .replace(/!\[([^\]]*)\]\([^)]*\)/g, "$1")
    .replace(/\[([^\]]+)\]\([^)]*\)/g, "$1")
    .replace(/<([^>]+)>/g, "$1")
    .replace(/(`+)(.*?)\1/g, "$2")
    .replace(/\\([\\`*_[\]{}()#+\-.!>~])/g, "$1")
    .replace(/[*_~]+/g, "")
    .trim();
}

export function extractMarkdownOutline(markdown: string): MarkdownOutlineItem[] {
  const items: MarkdownOutlineItem[] = [];
  const occurrences = new Map<string, number>();
  let fence: { marker: "`" | "~"; length: number } | undefined;

  for (const line of markdown.split(/\r?\n/)) {
    const fenceMatch = line.match(/^\s{0,3}(`{3,}|~{3,})/);
    if (fenceMatch) {
      const marker = fenceMatch[1][0] as "`" | "~";
      const length = fenceMatch[1].length;
      if (!fence) fence = { marker, length };
      else if (fence.marker === marker && length >= fence.length) fence = undefined;
      continue;
    }
    if (fence) continue;

    const match = line.match(/^\s{0,3}(#{1,6})\s+(.+?)\s*#*\s*$/);
    if (!match) continue;
    const level = match[1].length;
    if (level > MAX_OUTLINE_LEVEL) continue;
    const title = markdownHeadingText(match[2].replace(/\s+#+\s*$/, ""));
    if (!title) continue;
    items.push({ id: markdownHeadingId(title, occurrences), level, title });
  }
  return items;
}

export function finalAnswerNeedsOutline(answerHeight: number, viewportHeight: number, sectionCount: number) {
  return sectionCount >= 2 && viewportHeight > 0 && answerHeight > viewportHeight * 2;
}
