const SAFE_LINK_PROTOCOLS = new Set(["http:", "https:", "mailto:"]);
const SAFE_IMAGE_PROTOCOLS = new Set(["http:", "https:"]);

function safeMarkdownUrlForPolicy(
  value: unknown,
  protocols: ReadonlySet<string>,
  allowHash: boolean,
): string | undefined {
  if (typeof value !== "string") return undefined;
  const trimmed = value.trim();
  if (!trimmed) return undefined;
  if (trimmed.startsWith("#")) return allowHash ? trimmed : undefined;
  if (trimmed.startsWith("/")) return trimmed;
  // Do not let URL(base) turn a plain relative destination into an apparently
  // safe HTTP URL. Markdown destinations must be explicit or root-relative.
  if (!/^[A-Za-z][A-Za-z0-9+.-]*:/.test(trimmed)) return undefined;
  try {
    const parsed = new URL(trimmed);
    return protocols.has(parsed.protocol) ? trimmed : undefined;
  } catch {
    return undefined;
  }
}

export function safeMarkdownLinkUrl(value: unknown): string | undefined {
  return safeMarkdownUrlForPolicy(value, SAFE_LINK_PROTOCOLS, true);
}

export function safeMarkdownImageUrl(value: unknown): string | undefined {
  return safeMarkdownUrlForPolicy(value, SAFE_IMAGE_PROTOCOLS, false);
}

/** @deprecated Use the link- or image-specific policy. */
export const safeMarkdownUrl = safeMarkdownLinkUrl;
