import { Children, isValidElement, memo, ReactNode } from "react";
import ReactMarkdown from "react-markdown";
import rehypeHighlight from "rehype-highlight";
import rehypeKatex from "rehype-katex";
import remarkGfm from "remark-gfm";
import remarkMath from "remark-math";
import { CheckCheck, Copy } from "lucide-react";
import { normalizeMarkdownMath } from "./markdown_math";
import { extractMarkdownOutline, markdownHeadingId } from "./markdown_outline";
import { safeMarkdownImageUrl, safeMarkdownLinkUrl } from "./markdown_security";
import { useTimedClipboardCopy } from "./clipboard_copy";

export const MarkdownContent = memo(function MarkdownContent({ text, headingIdPrefix }: { text: string; headingIdPrefix?: string }) {
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
    rehypePlugins={[rehypeHighlight, rehypeKatex]}
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

export function CodeBlock({ children }: React.ComponentPropsWithoutRef<"pre">) {
  const child = Children.count(children) === 1 ? Children.only(children) : null;
  const className = isValidElement<{ className?: string }>(child) ? child.props.className ?? "" : "";
  const language = className.match(/(?:^|\s)language-([^\s]+)/)?.[1] ?? "text";
  const code = textFromNode(children).replace(/\n$/, "");
  const codeCopySubject = `${language} code`;
  const { copyState, copy, copyLabel, copyClass } = useTimedClipboardCopy(code, {
    idle: `Copy ${codeCopySubject}`,
    copied: `${codeCopySubject} copied`,
    failed: `Copy ${codeCopySubject} failed`,
  });
  return <figure className="code-block">
    <figcaption><span title={language}>{language}</span><button type="button" className={copyClass} onClick={() => void copy()} title={copyLabel} aria-label={copyLabel}>{copyState === "copied" ? <CheckCheck size={14}/> : <Copy size={14}/>}</button></figcaption>
    <pre>{children}</pre>
  </figure>;
}

export function textFromNode(node: ReactNode): string {
  if (typeof node === "string" || typeof node === "number") return String(node);
  if (Array.isArray(node)) return node.map(textFromNode).join("");
  if (isValidElement<{ children?: ReactNode; alt?: string }>(node)) {
    if (typeof node.props.alt === "string") return node.props.alt;
    return textFromNode(node.props.children);
  }
  return "";
}
