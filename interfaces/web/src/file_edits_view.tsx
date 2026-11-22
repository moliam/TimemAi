import { useMemo, useRef, useState } from "react";
import { ChevronDown, ChevronRight, PenLine } from "lucide-react";
import { useTimedClipboardCopy } from "./clipboard_copy";
import { t } from "./i18n";
import type { StringKey } from "./i18n";
import { parseDiffRows } from "./tool_presentation";
import type { DiffRow, FileEditOp, FileEditPresentation, FileEditsPresentation } from "./tool_presentation";
import { redactSensitiveDisplayText } from "./view_model";

const OP_LABEL_KEYS: Record<FileEditOp, StringKey> = {
  created: "tools.fileOpCreated",
  modified: "tools.fileOpModified",
  deleted: "tools.fileOpDeleted",
  unchanged: "tools.fileOpUnchanged",
  skipped: "tools.fileOpSkipped",
};

const SKIP_LABEL_KEYS: Record<string, StringKey> = {
  binary: "tools.fileSkipBinary",
  too_large: "tools.fileSkipTooLarge",
  unreadable: "tools.fileSkipUnreadable",
  not_captured: "tools.fileSkipNotCaptured",
  budget_exhausted: "tools.fileSkipBudget",
};


// Compact always-visible summary for the tool card header line.
export function FileEditsChip({ edits }: { edits: FileEditsPresentation }) {
  const added = edits.reduce((sum, edit) => sum + edit.added_lines, 0);
  const removed = edits.reduce((sum, edit) => sum + edit.removed_lines, 0);
  return (
    <span className="stream-tool-edits-chip" title={t("tools.fileEditsAria")}>
      <PenLine size={12} aria-hidden="true" />
      <span aria-label={t("tools.fileEditsAria")}>
        {t("tools.fileEditsChip", { count: edits.length, added, removed })}
      </span>
    </span>
  );
}

// Per-file collapsible diff sections, rendered inside the expanded tool card.
export function FileEditsPanel({ edits }: { edits: FileEditsPresentation }) {
  return (
    <div className="file-edits-panel" aria-label={t("tools.fileEditsAria")}>
      {edits.map((edit, index) => (
        <FileEditSection key={`${edit.path}:${index}`} edit={edit} />
      ))}
    </div>
  );
}

function splitEditPath(path: string): { directory: string; name: string } {
  const slash = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
  return {
    directory: slash >= 0 ? path.slice(0, slash + 1) : "",
    name: slash >= 0 ? path.slice(slash + 1) : path,
  };
}

function FileEditSection({ edit }: { edit: FileEditPresentation }) {
  // User feedback: sections carrying a diff stay expanded by default;
  // sections without a diff (skipped/unchanged) remain header-only.
  const [expanded, setExpanded] = useState(() => !!edit.diff);
  const sectionRef = useRef<HTMLDivElement | null>(null);
  const { copy, copyLabel, copyClass } = useTimedClipboardCopy(edit.diff ?? "", {
    idle: t("tools.copyDiff"),
    copied: t("tools.diffCopied"),
    failed: t("tools.copyFailed"),
  });
  const rows = useMemo(() => (edit.diff ? parseDiffRows(edit.diff) : []), [edit.diff]);
  const { directory, name } = splitEditPath(edit.path);
  const expandable = !!edit.diff;
  const skipLabel = edit.skip_reason
    ? t(SKIP_LABEL_KEYS[edit.skip_reason] ?? ("tools.fileSkipUnreadable" as StringKey))
    : undefined;
  const toggle = () => {
    if (!expandable) return;
    const selection = window.getSelection();
    if (selection && !selection.isCollapsed && sectionRef.current?.contains(selection.anchorNode)) return;
    setExpanded(current => !current);
  };
  return (
    <div className="file-edit-section" ref={sectionRef}>
      <div
        className={`file-edit-head${expandable ? " file-edit-toggle" : ""}`}
        role={expandable ? "button" : undefined}
        tabIndex={expandable ? 0 : undefined}
        aria-expanded={expandable ? expanded : undefined}
        aria-label={expandable ? (expanded ? t("tools.collapseOutput") : t("tools.expandOutput")) : undefined}
        onClick={toggle}
        onKeyDown={event => {
          if (!expandable || (event.key !== "Enter" && event.key !== " ")) return;
          event.preventDefault();
          toggle();
        }}
      >
        <span className={`file-edit-op file-edit-op-${edit.op}`}>{t(OP_LABEL_KEYS[edit.op])}</span>
        <span className="file-edit-path" title={edit.path}>
          {directory && <span className="file-edit-directory">{directory}</span>}
          <span className="file-edit-name">{name}</span>
        </span>
        {(edit.added_lines > 0 || edit.removed_lines > 0) && (
          <span className="file-edit-stats">
            {edit.added_lines > 0 && <span className="file-edit-additions">+{edit.added_lines}</span>}
            {edit.removed_lines > 0 && <span className="file-edit-deletions">&minus;{edit.removed_lines}</span>}
          </span>
        )}
        {expandable && (
          <span
            className={`file-edit-copy ${copyClass}`}
            role="button"
            tabIndex={0}
            aria-label={copyLabel}
            title={copyLabel}
            onClick={event => {
              event.stopPropagation();
              void copy();
            }}
            onKeyDown={event => {
              if (event.key !== "Enter" && event.key !== " ") return;
              event.preventDefault();
              event.stopPropagation();
              void copy();
            }}
          >
            {copyLabel}
          </span>
        )}
        {expandable && (
          <span className="file-edit-chevron" aria-hidden="true">
            {expanded ? <ChevronDown size={13} /> : <ChevronRight size={13} />}
          </span>
        )}
      </div>
      {expanded && expandable && (
        <div className="file-edit-diff">
          <div className="file-edit-diff-table">
            {rows.map((row, index) => (
              <DiffRowView key={index} row={row} />
            ))}
          </div>
          {edit.truncated && <div className="file-edit-notice">{t("tools.fileDiffOmitted")}</div>}
        </div>
      )}
      {!expandable && edit.truncated && (
        <div className="file-edit-notice">{t("tools.fileDiffOmitted")}</div>
      )}
      {!expandable && skipLabel && edit.op === "skipped" && (
        <div className="file-edit-notice">{skipLabel}</div>
      )}
    </div>
  );
}

function DiffRowView({ row }: { row: DiffRow }) {
  const sign = row.kind === "add" ? "+" : row.kind === "del" ? "-" : "";
  const content = row.kind === "hunk" || row.kind === "meta"
    ? row.text
    : redactSensitiveDisplayText(row.text) || "\u00A0";
  return (
    <div className={`file-edit-row file-edit-row-${row.kind}`}>
      <span className="file-edit-gutter">{row.oldNo !== undefined ? row.oldNo : ""}</span>
      <span className="file-edit-gutter">{row.newNo !== undefined ? row.newNo : ""}</span>
      <span className="file-edit-sign" aria-hidden="true">{sign}</span>
      <span className="file-edit-code">{content}</span>
    </div>
  );
}
