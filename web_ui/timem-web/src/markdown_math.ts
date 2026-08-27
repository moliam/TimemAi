const LATEX_INLINE_DELIMITER = /\\{1,2}\(([^\n]+?)\\{1,2}\)/g;
const LATEX_DISPLAY_DELIMITER = /\\{1,2}\[([\s\S]+?)\\{1,2}\]/g;
const CUSTOM_DISPLAY_DELIMITER = /\[\/math\]([\s\S]*?)\[\/math\]/g;
const CUSTOM_INLINE_DELIMITER = /\[\/inline\]([\s\S]*?)\[\/inline\]/g;
// Escape amounts such as "$5 " and "$19.99," without touching valid math
// that begins with a number, for example "$2P$" or "$100\sim500$".
const CURRENCY_DOLLAR = /(^|[^\\$])((?:\\\\)*)\$(?=\d(?:[\d,]*(?:\.\d+)?)?(?=\s|[.,;:!?)}\]]|$))/g;

function displayMathBlock(body: string) {
  return `\n\n$$\n${body.trim()}\n$$\n\n`;
}

function normalizeMathText(text: string) {
  return text
    .replace(CURRENCY_DOLLAR, "$1$2\\$")
    .replace(CUSTOM_DISPLAY_DELIMITER, (_, body: string) => displayMathBlock(body))
    .replace(CUSTOM_INLINE_DELIMITER, (_, body: string) => `$${body.trim()}$`)
    .replace(LATEX_INLINE_DELIMITER, (_, body: string) => `$${body.trim()}$`)
    .replace(LATEX_DISPLAY_DELIMITER, (_, body: string) => displayMathBlock(body));
}

function backtickRunLength(text: string, start: number) {
  let end = start;
  while (text[end] === "`") end += 1;
  return end - start;
}

/** Normalize common model-emitted math delimiters without altering inline code. */
function normalizeInlineMath(text: string) {
  let output = "";
  let plainStart = 0;
  let offset = 0;
  while (offset < text.length) {
    if (text[offset] !== "`") {
      offset += 1;
      continue;
    }
    const runLength = backtickRunLength(text, offset);
    const delimiter = "`".repeat(runLength);
    const closing = text.indexOf(delimiter, offset + runLength);
    if (closing < 0) {
      offset += runLength;
      continue;
    }
    output += normalizeMathText(text.slice(plainStart, offset));
    output += text.slice(offset, closing + runLength);
    offset = closing + runLength;
    plainStart = offset;
  }
  return output + normalizeMathText(text.slice(plainStart));
}

/**
 * Prepares assistant Markdown for remark-math. Supports `$...$`, `$$...$$`,
 * `\\(...\\)`, `\\[...\\]`, and model-specific math tags while preserving
 * fenced/inline code and ordinary currency amounts.
 */
export function normalizeMarkdownMath(markdown: string) {
  const lines = markdown.split(/(?<=\n)/);
  let output = "";
  let prose = "";
  let fenceMarker = "";

  const flushProse = () => {
    output += normalizeInlineMath(prose);
    prose = "";
  };

  for (const line of lines) {
    const fence = line.match(/^ {0,3}(`{3,}|~{3,})/);
    if (!fenceMarker) {
      if (fence) {
        flushProse();
        fenceMarker = fence[1][0].repeat(fence[1].length);
        output += line;
      } else {
        prose += line;
      }
      continue;
    }

    output += line;
    const closing = line.match(/^ {0,3}(`{3,}|~{3,})\s*$/);
    if (closing && closing[1][0] === fenceMarker[0] && closing[1].length >= fenceMarker.length) {
      fenceMarker = "";
    }
  }
  flushProse();
  return output;
}
