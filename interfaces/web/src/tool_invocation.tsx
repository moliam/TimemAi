import { DatabaseSearch } from "./database_search_icon";
import { Database, FilePen, FileText, Info } from "lucide-react";
import { t } from "./i18n";
import type { FileSelector, MemorySearchPresentation, ReadFilePresentation, RunBashEditPresentation, SelfToolPresentation } from "./tool_presentation";

export function ReadFileIcon() {
  return <span className="file-tool-icon" title="readfile"><FileText size={14} aria-hidden="true" /><span className="sr-only">readfile</span></span>;
}

function selectorLabel(value: FileSelector, start: boolean): string {
  if ("line_nr" in value) return t(start ? "tools.readFromLine" : "tools.readThroughLine", { value: value.line_nr });
  if ("byte_nr" in value) return t(start ? "tools.readFromByte" : "tools.readThroughByte", { value: value.byte_nr });
  return t(start ? "tools.readFromMatch" : "tools.readThroughMatch", { value: value.match });
}

export function readFileRangeLabel(file: ReadFilePresentation): string {
  const { starter: start, ender: end } = file;
  if (start && end && "line_nr" in start && "line_nr" in end)
    return t("tools.readLines", { start: start.line_nr, end: end.line_nr });
  if (start && end && "byte_nr" in start && "byte_nr" in end)
    return t("tools.readBytes", { start: start.byte_nr, end: end.byte_nr });
  return [start && selectorLabel(start, true), end && selectorLabel(end, false)].filter(Boolean).join(" · ");
}

export function ReadFileInvocation({ file }: { file: ReadFilePresentation }) {
  const range = readFileRangeLabel(file);
  const parts = [range, file.tail && t("tools.readTail"), file.encoding].filter(Boolean) as string[];
  const slash = Math.max(file.path.lastIndexOf("/"), file.path.lastIndexOf("\\"));
  const directory = file.path.slice(0, slash + 1);
  const name = file.path.slice(slash + 1) || file.path;
  return <span className="file-tool-preview tool-invocation-preview" title={[file.path, ...parts].join(" · ")}>
    <span className="file-tool-path" aria-label={file.path}>
      {directory && <span className="file-tool-directory" aria-hidden="true">{directory}</span>}
      <span className="file-tool-name" aria-hidden="true">{name}</span>
    </span>
    {parts.length > 0 && <span className="file-tool-range">{parts.join(" · ")}</span>}
  </span>;
}


export function RunBashEditIcon() {
  return <span className="bash-edit-icon" title="run_bash edit"><FilePen size={14} aria-hidden="true" /><span className="sr-only">run_bash edit</span></span>;
}

export function RunBashEditInvocation({ edit }: { edit: RunBashEditPresentation }) {
  const paths = edit.paths.join(", ");
  return <span className="bash-edit-preview tool-invocation-preview" title={paths}>{paths}</span>;
}


export function MemorySearchIcon() {
  return <span className="memory-search-icon" title={t("tools.memorySearch")}><DatabaseSearch size={14} aria-hidden="true" /><span className="sr-only">{t("tools.memorySearch")}</span></span>;
}


export function MemoryIcon() {
  return <span className="memory-tool-icon" title="memmgr"><Database size={14} aria-hidden="true" /><span className="sr-only">memmgr</span></span>;
}


export function MemorySearchInvocation({ search }: { search: MemorySearchPresentation }) {
  if (search.kind === "sql")
    return <span className="memory-search-preview tool-invocation-preview">{t("tools.queryMemoryIn")} <code>{search.source}</code></span>;
  return <span className="memory-search-preview tool-invocation-preview">
    {search.query ? <>{t("tools.searchMemory")} &quot;{search.query}&quot;</> : t("tools.searchAllMemory")} {t("tools.inMemory")} <code>{search.source}</code>
  </span>;
}

export function SelfToolIcon() {
  return <span className="self-tool-icon" title="self_tool"><Info size={14} aria-hidden="true" /><span className="sr-only">self_tool</span></span>;
}

export function SelfToolInvocation({ operation }: { operation: SelfToolPresentation }) {
  if (operation.kind === "change_cwd")
    return <span className="self-tool-preview tool-invocation-preview">{t("tools.changeDirectoryTo")} <code>{operation.path}</code></span>;
  const key = operation.kind === "path"
    ? "tools.inspectRuntimePaths"
    : operation.kind === "params"
      ? "tools.inspectRuntimeSettings"
      : "tools.inspectCurrentDirectory";
  return <span className="self-tool-preview tool-invocation-preview">{t(key)}</span>;
}
