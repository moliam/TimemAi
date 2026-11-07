import rehypeHighlight from "rehype-highlight";
import rehypeKatex from "rehype-katex";
// Vendor styles ship with the enhancement chunk: the highlight theme and
// KaTeX layout only style nodes these plugins produce, so sessions without
// code fences or math never pay for the CSS either.
import "highlight.js/styles/github-dark.css";
import "katex/dist/katex.min.css";

// Heavy rehype plugins (highlight.js + KaTeX, several hundred KB). Kept in a
// separate module so the browser loads them off the first-paint critical
// path; markdown text still renders synchronously before they arrive, and
// MarkdownContent only requests it when the content actually needs it.
export const heavyRehypePlugins = [rehypeHighlight, rehypeKatex] as const;
