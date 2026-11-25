import {
  createContext,
  useContext,
  useMemo,
  type AnchorHTMLAttributes,
  type MouseEvent,
  type ReactNode,
} from "react";
import { t, useT } from "./i18n";
import { safeMarkdownLinkUrl } from "./markdown_security";

type FileLinkContextValue = {
  sessionId: string;
  cwd?: string;
  token?: string;
};

const FileLinkContext = createContext<FileLinkContextValue | null>(null);

export function FileLinkProvider({
  sessionId,
  cwd,
  token,
  children,
}: FileLinkContextValue & { children: ReactNode }) {
  const value = useMemo(
    () => ({ sessionId, cwd, token }),
    [sessionId, cwd, token],
  );
  return (
    <FileLinkContext.Provider value={value}>
      {children}
    </FileLinkContext.Provider>
  );
}

function normalizedCwd(cwd?: string) {
  return (cwd ?? "").replace(/\\/g, "/").replace(/\/+$/, "");
}

function filePathExtension(path: string) {
  const clean = path.split(/[?#]/, 1)[0] ?? "";
  const name = clean.slice(clean.lastIndexOf("/") + 1);
  const dot = name.lastIndexOf(".");
  if (dot < 0) return "";
  if (dot === 0) return name.length > 1 ? name.slice(1) : "";
  return name.slice(dot + 1);
}

function decodedPath(path: string) {
  try {
    return decodeURIComponent(path);
  } catch {
    return path;
  }
}

function pathWithoutSuffix(path: string) {
  return path.split(/[?#]/, 1)[0] ?? path;
}

function looksLikeAbsoluteFile(path: string) {
  if (!path.startsWith("/") || path.startsWith("//")) return false;
  if (/^\/(api|assets|ws)(\/|$)/.test(path)) return false;
  return filePathExtension(path) !== "";
}

/**
 * Returns the host-side path for a Markdown link that points at a local file.
 * `file://` links are explicit; plain absolute paths are accepted when they
 * are under the session CWD or clearly look like a file. App routes such as
 * `/docs` remain normal browser links.
 */
export function localFileTarget(href: string | undefined, cwd?: string) {
  const value = href?.trim();
  if (!value) return undefined;

  if (/^file:/i.test(value)) {
    try {
      const url = new URL(value);
      if (url.protocol.toLowerCase() !== "file:") return undefined;
      if (url.host && url.host !== "localhost" && url.host !== "127.0.0.1") {
        return undefined;
      }
      let path: string;
      try {
        path = decodeURIComponent(url.pathname);
      } catch {
        path = url.pathname;
      }
      if (/^\/[A-Za-z]:\//.test(path)) path = path.slice(1);
      return path || undefined;
    } catch {
      return undefined;
    }
  }

  if (/^[A-Za-z]:[\\/]/.test(value)) {
    return decodedPath(value.replace(/\\/g, "/"));
  }

  const cleanValue = decodedPath(pathWithoutSuffix(value));
  const base = normalizedCwd(cwd);
  if (cleanValue.startsWith("/")) {
    if (base && (cleanValue === base || cleanValue.startsWith(`${base}/`))) {
      return cleanValue;
    }
    return looksLikeAbsoluteFile(cleanValue) ? cleanValue : undefined;
  }

  if (value.startsWith("#") || value.startsWith("//")) return undefined;
  if (/^[A-Za-z][A-Za-z0-9+.-]*:/.test(value)) return undefined;
  if (!cleanValue) return undefined;
  if (value.startsWith("./") || value.startsWith("../")) return cleanValue;
  if (cleanValue.includes("/") || filePathExtension(cleanValue) !== "") {
    return cleanValue;
  }
  return undefined;
}

function writePopupMessage(
  popup: Window,
  title: string,
  message: string,
) {
  try {
    popup.document.title = title;
    const body = popup.document.body;
    body.replaceChildren();
    body.style.cssText =
      "margin:0;padding:32px;background:#111;color:#d8e0e5;font:14px/1.6 ui-sans-serif,system-ui,sans-serif";
    const heading = popup.document.createElement("h1");
    heading.textContent = title;
    heading.style.cssText = "font-size:18px;margin:0 0 8px";
    const detail = popup.document.createElement("p");
    detail.textContent = message;
    detail.style.cssText = "margin:0;color:#9aabb3";
    body.append(heading, detail);
  } catch {
    popup.close();
  }
}

type MarkdownLinkProps = Omit<AnchorHTMLAttributes<HTMLAnchorElement>, "href"> & {
  href?: string;
};

// GFM autolinks and hand-written bare URLs use the address as the label. Those
// already read as destinations, so the outbound marker would only add noise.
const ADDRESS_LABEL = /^https?:\/\/\S+$/i;

function labelIsAddress(children: ReactNode): boolean {
  if (typeof children === "string") return ADDRESS_LABEL.test(children.trim());
  return (
    Array.isArray(children) &&
    children.length === 1 &&
    typeof children[0] === "string" &&
    ADDRESS_LABEL.test(children[0].trim())
  );
}

export function MarkdownLink({ href, children, ...props }: MarkdownLinkProps) {
  useT();
  const context = useContext(FileLinkContext);
  const localTarget = localFileTarget(href, context?.cwd);
  const sessionId = context?.sessionId?.trim();
  if (localTarget && sessionId) {
    return <LocalFileLink {...props} href={href} targetPath={localTarget} sessionId={sessionId} token={context?.token} />;
  }
  const safeHref = safeMarkdownLinkUrl(href);
  const className = [
    "markdown-external-link",
    labelIsAddress(children) ? "markdown-address-link" : null,
    props.className,
  ]
    .filter(Boolean)
    .join(" ");
  return safeHref ? (
    <a
      {...props}
      className={className}
      href={safeHref}
      target="_blank"
      rel="noopener noreferrer"
    >
      {children}
    </a>
  ) : (
    <span {...props}>{children}</span>
  );
}

function LocalFileLink({
  targetPath,
  sessionId,
  token,
  children,
  ...props
}: MarkdownLinkProps & {
  targetPath: string;
  sessionId: string;
  token?: string;
}) {
  const openPreview = async (event: MouseEvent<HTMLAnchorElement>) => {
    event.preventDefault();
    const popup = window.open("", "_blank");
    if (popup) {
      popup.opener = null;
      writePopupMessage(
        popup,
        t("tools.localFilePreviewTitle"),
        t("tools.localFilePreviewCreating"),
      );
    }
    try {
      const params = new URLSearchParams({ session_id: sessionId });
      if (token) params.set("token", token);
      const response = await fetch(`/api/file-preview-links?${params.toString()}`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ session_id: sessionId, path: targetPath }),
      });
      const payload = (await response.json().catch(() => null)) as
        | { url?: string; error?: string }
        | null;
      if (!response.ok || !payload?.url) {
        throw new Error(payload?.error ?? "file_preview_link_failed");
      }
      const previewUrl = new URL(payload.url, window.location.origin).toString();
      if (popup) popup.location.replace(previewUrl);
      else window.location.assign(previewUrl);
    } catch (error) {
      const message =
        error instanceof Error
          ? error.message
          : t("tools.localFilePreviewFailed");
      if (popup) {
        writePopupMessage(
          popup,
          t("tools.localFilePreviewFailedTitle"),
          `${t("tools.localFilePreviewFailed")} (${message})`,
        );
      } else {
        window.alert(`${t("tools.localFilePreviewFailed")} (${message})`);
      }
    }
  };

  return (
    <a
      {...props}
      href="#"
      className={["markdown-local-file-link", props.className]
        .filter(Boolean)
        .join(" ")}
      data-local-file={targetPath}
      title={t("tools.localFilePreviewTitle")}
      onClick={openPreview}
      rel="noopener noreferrer"
    >
      {children}
    </a>
  );
}
