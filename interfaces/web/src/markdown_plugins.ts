import rehypeHighlight from "rehype-highlight";
import rehypeKatex from "rehype-katex";

// Heavy rehype plugins (highlight.js + KaTeX, several hundred KB). Kept in a
// separate module so the browser loads them off the first-paint critical
// path; markdown text still renders synchronously before they arrive.
export const heavyRehypePlugins = [rehypeHighlight, rehypeKatex] as const;
