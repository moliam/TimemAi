import { memo } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import remarkMath from "remark-math";
import { CodeBlock, textFromNode } from "./markdown_render";
import { heavyRehypePlugins } from "./markdown_plugins";
import { normalizeMarkdownMath } from "./markdown_math";
import { extractMarkdownOutline, markdownHeadingId } from "./markdown_outline";
import { safeMarkdownImageUrl, safeMarkdownLinkUrl } from "./markdown_security";

// Full synchronous markdown pipeline including heavy rehype plugins. Used by
// tests (renderToStaticMarkup cannot await lazy chunks); production renders
// through MarkdownContent which loads the heavy plugins progressively.
export const MarkdownContentFull = memo(function MarkdownContentFull({ text, headingIdPrefix }: { text: string; headingIdPrefix?: string }) {
  const headingOccurrences = new Map<string, number>();
  const outlineIds = headingIdPrefix ? extractMarkdownOutline(text).map((item) => item.id) : [];
  let outlineIndex = 0;
  const sourceLines = headingIdPrefix ? text.split(/\r?\n/) : [];
  const heading = (level: 1 | 2 | 3) => ({ node, children, ...props }: React.HTMLAttributes<HTMLHeadingElement> & { node?: { position?: { start?: { line?: number } } } }) => {
    const sourceLine = sourceLines[(node?.position?.start?.line ?? 0) - 1] ?? "";
    const atxLevel = sourceLine.match(/^ {0,3}(#{1,6})(?:[ \t]|$)/)?.[1].length;
    const outlineId = atxLevel === level ? outlineIds[outlineIndex++] : undefined;
    const title = textFromNode(children).trim();
    const fallbackId = markdownHeadingId(title, headingOccurrences);
    const id = headingIdPrefix ? `${headingIdPrefix}-${outlineId ?? fallbackId}` : undefined;
    const Tag = `h${level}` as const;
    return <Tag {...props} id={id}>{children}</Tag>;
  };
  return <div className="markdown-body"><ReactMarkdown
    remarkPlugins={[remarkGfm, remarkMath]}
    rehypePlugins={[...heavyRehypePlugins]}
    components={{
      h1: heading(1),
      h2: heading(2),
      h3: heading(3),
      a: ({ node: _node, href, ...props }) => {
        const safeHref = safeMarkdownLinkUrl(href);
        return safeHref ? <a {...props} href={safeHref} target="_blank" rel="noopener noreferrer"/> : <span {...props}/>;
      },
      img: ({ node: _node, src, alt, ...props }) => {
        const safeSrc = safeMarkdownImageUrl(src);
        return safeSrc ? <img {...props} src={safeSrc} alt={alt ?? ""}/> : null;
      },
      pre: CodeBlock,
      table: ({ node: _node, ...props }) => <div className="table-scroll" role="region" tabIndex={0} aria-label="Scrollable table. Use horizontal scroll to inspect all columns."><table {...props}/></div>,
    }}
  >{normalizeMarkdownMath(text)}</ReactMarkdown></div>;
});
