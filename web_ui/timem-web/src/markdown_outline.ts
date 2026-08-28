export type MarkdownOutlineItem = {
  id: string;
  level: number;
  title: string;
};

const MAX_OUTLINE_LEVEL = 3;
export const MAX_OUTLINE_SOURCE_CHARS = 512 * 1024;
export const MAX_OUTLINE_HEADINGS = 128;
export const MAX_OUTLINE_LINE_CHARS = 8 * 1024;
export const MAX_OUTLINE_HEADING_SOURCE_CHARS = 2 * 1024;
export const MAX_OUTLINE_TITLE_CHARS = 160;

function boundedTitle(value: string) {
  if (value.length <= MAX_OUTLINE_TITLE_CHARS) return value;
  return `${value.slice(0, MAX_OUTLINE_TITLE_CHARS - 1).trimEnd()}…`;
}

export function markdownHeadingSlug(title: string) {
  const normalized = boundedTitle(title)
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

function appendBounded(output: string[], value: string, length: { value: number }) {
  if (!value || length.value >= MAX_OUTLINE_TITLE_CHARS) return;
  const remaining = MAX_OUTLINE_TITLE_CHARS - length.value;
  const next = value.length <= remaining ? value : value.slice(0, remaining);
  output.push(next);
  length.value += next.length;
}

/**
 * Extracts display text from one bounded heading source with a forward-only scan.
 * It intentionally implements only the inline constructs needed for the outline;
 * malformed or deeply nested Markdown is treated as plain bounded text.
 */
export function markdownHeadingText(source: string) {
  const input = source.slice(0, MAX_OUTLINE_HEADING_SOURCE_CHARS);
  const output: string[] = [];
  const outputLength = { value: 0 };
  let index = 0;

  while (index < input.length && outputLength.value < MAX_OUTLINE_TITLE_CHARS) {
    const char = input[index];
    if (char === "\\" && index + 1 < input.length) {
      appendBounded(output, input[index + 1], outputLength);
      index += 2;
      continue;
    }
    if (char === "!" && input[index + 1] === "[") {
      index += 1;
      continue;
    }
    if (char === "[") {
      const close = input.indexOf("]", index + 1);
      if (close >= 0) {
        appendBounded(output, input.slice(index + 1, close), outputLength);
        index = close + 1;
        if (input[index] === "(") {
          const destinationClose = input.indexOf(")", index + 1);
          index = destinationClose >= 0 ? destinationClose + 1 : index + 1;
        }
        continue;
      }
    }
    if (char === "<") {
      const close = input.indexOf(">", index + 1);
      if (close >= 0) {
        appendBounded(output, input.slice(index + 1, close), outputLength);
        index = close + 1;
        continue;
      }
    }
    if (char === "`") {
      let runEnd = index + 1;
      while (runEnd < input.length && input[runEnd] === "`") runEnd += 1;
      const marker = input.slice(index, runEnd);
      const close = input.indexOf(marker, runEnd);
      if (close >= 0) {
        appendBounded(output, input.slice(runEnd, close), outputLength);
        index = close + marker.length;
        continue;
      }
      index = runEnd;
      continue;
    }
    if (char === "*" || char === "_" || char === "~") {
      index += 1;
      continue;
    }
    appendBounded(output, char, outputLength);
    index += 1;
  }

  const title = output.join("").trim();
  return source.length > MAX_OUTLINE_HEADING_SOURCE_CHARS || title.length >= MAX_OUTLINE_TITLE_CHARS
    ? boundedTitle(title)
    : title;
}

function fenceAt(line: string) {
  let index = 0;
  while (index < line.length && index < 3 && line[index] === " ") index += 1;
  const marker = line[index];
  if (marker !== "`" && marker !== "~") return undefined;
  let end = index;
  while (end < line.length && line[end] === marker) end += 1;
  const length = end - index;
  return length >= 3 ? { marker: marker as "`" | "~", length } : undefined;
}

function headingAt(line: string) {
  let index = 0;
  while (index < line.length && index < 3 && line[index] === " ") index += 1;
  const markerStart = index;
  while (index < line.length && line[index] === "#" && index - markerStart < 6) index += 1;
  const level = index - markerStart;
  if (level < 1 || index >= line.length || (line[index] !== " " && line[index] !== "\t")) return undefined;
  while (index < line.length && (line[index] === " " || line[index] === "\t")) index += 1;
  let end = line.length;
  while (end > index && (line[end - 1] === " " || line[end - 1] === "\t")) end -= 1;
  let hashStart = end;
  while (hashStart > index && line[hashStart - 1] === "#") hashStart -= 1;
  if (hashStart < end && hashStart > index && (line[hashStart - 1] === " " || line[hashStart - 1] === "\t")) {
    end = hashStart - 1;
    while (end > index && (line[end - 1] === " " || line[end - 1] === "\t")) end -= 1;
  }
  return { level, source: line.slice(index, end) };
}

export function extractMarkdownOutline(markdown: string): MarkdownOutlineItem[] {
  if (!markdown || markdown.length > MAX_OUTLINE_SOURCE_CHARS) return [];
  const items: MarkdownOutlineItem[] = [];
  const occurrences = new Map<string, number>();
  let fence: { marker: "`" | "~"; length: number } | undefined;
  let offset = 0;

  while (offset <= markdown.length) {
    let lineEnd = markdown.indexOf("\n", offset);
    if (lineEnd < 0) lineEnd = markdown.length;
    const rawLength = lineEnd - offset;
    const lineLength = rawLength > 0 && markdown[lineEnd - 1] === "\r" ? rawLength - 1 : rawLength;
    if (lineLength <= MAX_OUTLINE_LINE_CHARS) {
      const line = markdown.slice(offset, offset + lineLength);
      const nextFence = fenceAt(line);
      if (nextFence) {
        if (!fence) fence = nextFence;
        else if (fence.marker === nextFence.marker && nextFence.length >= fence.length) fence = undefined;
      } else if (!fence) {
        const heading = headingAt(line);
        if (heading && heading.level <= MAX_OUTLINE_LEVEL) {
          const title = markdownHeadingText(heading.source);
          if (title) {
            if (items.length >= MAX_OUTLINE_HEADINGS) return [];
            items.push({ id: markdownHeadingId(title, occurrences), level: heading.level, title });
          }
        }
      }
    }
    if (lineEnd === markdown.length) break;
    offset = lineEnd + 1;
  }
  return items;
}

export function finalAnswerNeedsOutline(answerHeight: number, viewportHeight: number, sectionCount: number) {
  return sectionCount >= 2 && viewportHeight > 0 && answerHeight > viewportHeight;
}


export const MARKDOWN_OUTLINE_START_ID = "__start";

export function markdownOutlineFitsBesideContent(
  availableSpace: number,
  outlineWidth: number,
  gap: number,
  edgeGuard: number,
) {
  return Number.isFinite(availableSpace)
    && availableSpace >= Math.max(0, outlineWidth) + Math.max(0, gap) + Math.max(0, edgeGuard);
}

export function markdownOutlineActiveId(
  items: readonly MarkdownOutlineItem[],
  headingTops: ReadonlyMap<string, number>,
  threshold: number,
) {
  let activeId = MARKDOWN_OUTLINE_START_ID;
  if (!Number.isFinite(threshold)) return activeId;
  for (const item of items) {
    const top = headingTops.get(item.id);
    if (top === undefined || !Number.isFinite(top)) continue;
    if (top <= threshold) activeId = item.id;
    else break;
  }
  return activeId;
}

export function markdownOutlineRailScrollTop(
  currentScrollTop: number,
  viewportHeight: number,
  contentHeight: number,
  targetTop: number,
  targetHeight: number,
  edgePadding: number,
) {
  const values = [currentScrollTop, viewportHeight, contentHeight, targetTop, targetHeight, edgePadding];
  if (!values.every(Number.isFinite)) return Math.max(0, Number.isFinite(currentScrollTop) ? currentScrollTop : 0);
  const viewport = Math.max(0, viewportHeight);
  const content = Math.max(viewport, contentHeight);
  const height = Math.max(0, targetHeight);
  const padding = Math.min(Math.max(0, edgePadding), viewport / 2);
  const scrollTop = Math.max(0, currentScrollTop);
  const visibleTop = scrollTop + padding;
  const visibleBottom = scrollTop + viewport - padding;
  let next = scrollTop;
  if (targetTop < visibleTop) next = targetTop - padding;
  else if (targetTop + height > visibleBottom) next = targetTop + height - viewport + padding;
  return Math.min(Math.max(0, next), Math.max(0, content - viewport));
}

export function markdownOutlineTargetScrollTop(
  currentScrollTop: number,
  targetViewportTop: number,
  viewportTop: number,
  offset: number,
) {
  const values = [currentScrollTop, targetViewportTop, viewportTop, offset];
  if (!values.every(Number.isFinite)) return Math.max(0, Number.isFinite(currentScrollTop) ? currentScrollTop : 0);
  return Math.max(0, currentScrollTop + targetViewportTop - viewportTop - Math.max(0, offset));
}

export type MarkdownFloatingNavigationOverlap = "none" | "partial" | "full";

export function markdownFloatingNavigationLayout(
  contentLeft: number,
  navigationWidth: number,
  bodyGap: number,
  viewportWidth: number,
  edgeInset: number,
  outlineLeft?: number,
  outlineRight?: number,
) {
  const finite = [contentLeft, navigationWidth, bodyGap, viewportWidth, edgeInset].every(Number.isFinite);
  if (!finite) return { left: 0, overlap: "none" as MarkdownFloatingNavigationOverlap };
  const width = Math.max(0, navigationWidth);
  const inset = Math.max(0, edgeInset);
  const maximumVisibleLeft = Math.max(inset, viewportWidth - width - inset);
  // Stay as close to the reading column as possible while preserving a body gap.
  // When an expanded outline consumes that space, this naturally degrades from
  // no overlap to partial overlap and only then to full overlap.
  const left = Math.min(maximumVisibleLeft, Math.max(inset, contentLeft - Math.max(0, bodyGap) - width));
  if (!Number.isFinite(outlineLeft) || !Number.isFinite(outlineRight) || outlineRight! <= outlineLeft!) {
    return { left, overlap: "none" as MarkdownFloatingNavigationOverlap };
  }
  const intersection = Math.max(0, Math.min(left + width, outlineRight!) - Math.max(left, outlineLeft!));
  const overlap: MarkdownFloatingNavigationOverlap = intersection <= 0
    ? "none"
    : intersection >= width - .5
      ? "full"
      : "partial";
  return { left, overlap };
}

export function markdownOutlineAnimationPosition(
  start: number,
  target: number,
  elapsedMs: number,
  durationMs: number,
) {
  if (![start, target, elapsedMs, durationMs].every(Number.isFinite)) return Number.isFinite(target) ? target : 0;
  if (durationMs <= 0 || elapsedMs >= durationMs) return target;
  if (elapsedMs <= 0) return start;
  const progress = elapsedMs / durationMs;
  const eased = 1 - Math.pow(1 - progress, 3);
  return start + (target - start) * eased;
}
