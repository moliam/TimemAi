const SAFE_URL_PROTOCOLS = new Set(["http:", "https:", "mailto:"]);

export function safeMarkdownUrl(value: unknown): string | undefined {
  if (typeof value !== "string") return undefined;
  const trimmed = value.trim();
  if (!trimmed) return undefined;
  if (trimmed.startsWith("#") || trimmed.startsWith("/")) return trimmed;
  try {
    const origin = typeof window === "undefined" ? "http://localhost" : window.location.origin;
    const parsed = new URL(trimmed, origin);
    return SAFE_URL_PROTOCOLS.has(parsed.protocol) ? trimmed : undefined;
  } catch {
    return undefined;
  }
}
