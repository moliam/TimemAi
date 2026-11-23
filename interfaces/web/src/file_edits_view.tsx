import { useEffect, useMemo, useRef, useState } from "react";
import { Check, ChevronDown, ChevronRight, Copy, PenLine, X } from "lucide-react";
import { useTimedClipboardCopy } from "./clipboard_copy";
import { useBetaPreference } from "./beta_preferences";
import { useStreamWorkInProgress } from "./stream_work_context";
import { useFileEditsDuringWork } from "./stream_ui_mode";
import { t } from "./i18n";
import type { StringKey } from "./i18n";
import { fileEditsPresentation, parseDiffRows } from "./tool_presentation";
import type { DiffRow, FileEditOp, FileEditPresentation, FileEditsPresentation } from "./tool_presentation";
import { redactSensitiveDisplayText } from "./view_model";

const OP_LABEL_KEYS: Record<FileEditOp, StringKey> = {
  created: "tools.fileOpCreated",
  modified: "tools.fileOpModified",
  deleted: "tools.fileOpDeleted",
  unchanged: "tools.fileOpUnchanged",
  skipped: "tools.fileOpSkipped",
};

// Compact kind tag for captures that carry no diff. The op text itself is
// always created/modified/deleted; this tag only says what the path is or why
// the diff is missing (Binary, folder, too large ...).
const KIND_LABEL_KEYS: Record<string, StringKey> = {
  binary: "tools.fileKindBinary",
  directory: "tools.fileKindDirectory",
  too_large: "tools.fileKindTooLarge",
  unreadable: "tools.fileKindUnreadable",
  not_captured: "tools.fileKindNoBaseline",
  budget_exhausted: "tools.fileKindDiffOmitted",
};


// Compact always-visible summary for the tool card header line.
export function FileEditsChip({
  edits,
  onActivate,
}: {
  edits: FileEditsPresentation;
  // Clicking the diff summary must reveal the diff itself; the surrounding
  // tool row header would otherwise swallow the click and expand the command
  // view. The chip therefore owns activation and stops event bubbling.
  onActivate?: () => void;
}) {
  const added = edits.reduce((sum, edit) => sum + edit.added_lines, 0);
  const removed = edits.reduce((sum, edit) => sum + edit.removed_lines, 0);
  const content = (
    <>
      <PenLine size={12} aria-hidden="true" />
      <span aria-label={t("tools.fileEditsAria")}>
        {t("tools.fileEditsChip", { count: edits.length, added, removed })}
      </span>
    </>
  );
  if (!onActivate)
    return <span className="stream-tool-edits-chip" title={t("tools.fileEditsAria")}>{content}</span>;
  return (
    <span
      className="stream-tool-edits-chip stream-tool-edits-chip-action"
      role="button"
      tabIndex={0}
      title={t("tools.fileEditsAria")}
      onClick={event => {
        event.stopPropagation();
        onActivate();
      }}
      onKeyDown={event => {
        if (event.key !== "Enter" && event.key !== " ") return;
        event.preventDefault();
        event.stopPropagation();
        onActivate();
      }}
    >
      {content}
    </span>
  );
}

// Per-file collapsible diff sections, rendered inside the expanded tool card.
// The panel itself is the single framed box; file rows stay plain text lines
// and only an expanded diff renders as an inner box. The card header chip owns
// the summed file count and +/- totals, so the panel renders no second head —
// repeating the same numbers there only tripled the same reading three times.
export function FileEditsPanel({ edits }: { edits: ReadonlyArray<TurnEditEntry> }) {
  const wrap = useBetaPreference("fileEditsWrap");
  // A single file needs no ordinal: "1." only repeats what the card already
  // says. The index appears only when the panel lists several files.
  const showIndex = edits.length > 1;
  return (
    <div className={`file-edits-panel${wrap ? " edits-wrap" : ""}`} aria-label={t("tools.fileEditsAria")}>
      {edits.map((edit, index) => (
        <FileEditSection key={`${edit.path}:${index}`} edit={edit} index={index} showIndex={showIndex} />
      ))}
    </div>
  );
}

// A per-path entry in the turn-level change summary. `capture_count` records
// how many tool calls captured that path during the turn; the displayed
// op/diff is the final non-skipped capture so the row reflects the end state.
export type TurnEditEntry = FileEditPresentation & { capture_count?: number };

export type TurnEditsSummary = {
  edits: TurnEditEntry[];
  overflowCount: number;
};

// The host folds the turn's net begin-to-end file changes from the complete
// history (server::TurnFileChangeAccumulator) and ships them on the turn, so the
// client only validates and renders. Deriving the same fact here from the
// transport event window is what made a long turn report a single file after a
// restart: that window is trimmed for size, and the fact was derived from it.
export const EMPTY_TURN_EDITS_SUMMARY: TurnEditsSummary = { edits: [], overflowCount: 0 };

export function turnFileChangesSummary(value: unknown): TurnEditsSummary | undefined {
  if (!value || typeof value !== "object") return undefined;
  const summary = value as { edits?: unknown; overflow_count?: unknown };
  // The host already folded the turn, so the entry budget is the host's own
  // (64 paths) rather than the per-tool-call card budget.
  const edits = fileEditsPresentation(summary.edits, { maxEntries: 64 });
  if (!edits) return undefined;
  const overflow = summary.overflow_count;
  return {
    edits,
    overflowCount:
      typeof overflow === "number" && Number.isSafeInteger(overflow) && overflow > 0 ? overflow : 0,
  };
}

// Folded-turn change summary: rendered beside the archived Thought/Action
// frame so the whole turn's file diff stays visible without expanding it.
export function TurnEditsSummaryBox({ summary }: { summary: TurnEditsSummary }) {
  return (
    <section className="turn-edits-summary" aria-label={t("tools.turnEditsTitle")}>
      <div className="turn-edits-summary-head">
        <b>{t("tools.turnEditsTitle")}</b>
        <FileEditsChip edits={summary.edits} />
      </div>
      <FileEditsPanel edits={summary.edits} />
      {summary.overflowCount > 0 && (
        <div className="file-edit-notice">
          {t("tools.turnEditsOverflow", { count: summary.overflowCount })}
        </div>
      )}
    </section>
  );
}

// Byte counts only: a non-text edit has no diff, so its size is the evidence
// the row can show. ASCII units keep this view file free of literal strings.
function formatFileSize(bytes: number | null | undefined): string | undefined {
  if (bytes === null || bytes === undefined) return undefined;
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB"];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value >= 10 ? Math.round(value) : value.toFixed(1)} ${units[unit]}`;
}

function splitEditPath(path: string): { directory: string; name: string } {
  const slash = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
  return {
    directory: slash >= 0 ? path.slice(0, slash + 1) : "",
    name: slash >= 0 ? path.slice(slash + 1) : path,
  };
}

function FileEditSection({ edit, index, showIndex }: { edit: TurnEditEntry; index: number; showIndex: boolean }) {
  // With turn auto-collapse on (the default), sections stay folded so the
  // reader expands only the files they care about; when auto-collapse is
  // off, sections carrying a diff stay expanded and notice-only sections
  // (skipped/unchanged) remain header-only.
  const autoCollapse = useBetaPreference("autoCollapse");
  // User request: keep file edit panels expanded while work is in progress, so
  // the diff stays open even after its tool row folds; the finished turn falls
  // back to the auto-collapse rule, and a section the user toggled keeps that
  // explicit choice.
  const expandDuringWork = useFileEditsDuringWork() && useStreamWorkInProgress();
  const [expanded, setExpanded] = useState(() => !!edit.diff && (!autoCollapse || expandDuringWork));
  const manualToggle = useRef(false);
  useEffect(() => {
    if (manualToggle.current) return;
    setExpanded(!!edit.diff && (expandDuringWork || !autoCollapse));
  }, [expandDuringWork, autoCollapse, edit.diff]);
  const sectionRef = useRef<HTMLDivElement | null>(null);
  const { copyState, copy, copyLabel, copyClass } = useTimedClipboardCopy(edit.diff ?? "", {
    idle: t("tools.copyDiff"),
    copied: t("tools.diffCopied"),
    failed: t("tools.copyFailed"),
  });
  const rows = useMemo(() => (edit.diff ? parseDiffRows(edit.diff) : []), [edit.diff]);
  const { directory, name } = splitEditPath(edit.path);
  const expandable = !!edit.diff;
  // Non-text edits carry no diff; the capture size rides in the tag tooltip.
  const kindLabel = !expandable && edit.skip_reason
    ? t(KIND_LABEL_KEYS[edit.skip_reason] ?? ("tools.fileKindUnreadable" as StringKey))
    : undefined;
  const sizeLabel = expandable ? undefined : formatFileSize(edit.size_bytes);
  const kindTitle = kindLabel
    ? sizeLabel ? `${kindLabel} · ${sizeLabel}` : kindLabel
    : undefined;
  const toggle = () => {
    if (!expandable) return;
    manualToggle.current = true;
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
        {showIndex && <span className="file-edit-index">{index + 1}.</span>}
        <span className={`file-edit-op file-edit-op-${edit.op}`}>{t(OP_LABEL_KEYS[edit.op])}</span>
        <span className="file-edit-path" title={edit.path}>
          {directory && <span className="file-edit-directory">{directory}</span>}
          <span className="file-edit-name">{name}</span>
        </span>
        {(edit.capture_count ?? 0) > 1 && (
          <span
            className="file-edit-captures"
            title={t("tools.captureCount", { count: edit.capture_count ?? 0 })}
          >
            ×{edit.capture_count}
          </span>
        )}
        {kindLabel && (
          <span className="file-edit-kind" title={kindTitle}>{kindLabel}</span>
        )}
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
            {/* Icon-only control: the label stays in aria-label/title so the
                diff header keeps its one-line row height. */}
            {copyState === "copied"
              ? <Check size={12} aria-hidden="true" />
              : copyState === "failed"
                ? <X size={12} aria-hidden="true" />
                : <Copy size={12} aria-hidden="true" />}
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
      {/* Budget trimming already shows the "diff omitted" kind tag; repeating a
          truncation notice there read as a contradiction (1 line yet "over the
          limit"), because that limit is the turn payload, not this file. */}
      {!expandable && edit.truncated && !edit.skip_reason && (
        <div className="file-edit-notice">{t("tools.fileDiffOmitted")}</div>
      )}
    </div>
  );
}

function DiffRowView({ row }: { row: DiffRow }) {
  const sign = row.kind === "add" ? "+" : row.kind === "del" ? "-" : "";
  const content = row.kind === "hunk" || row.kind === "meta"
    ? row.text
    : redactSensitiveDisplayText(row.text) || "\u00A0";
  // Unchanged rows share the same number on both sides; render it once in
  // the old-side gutter instead of once per side.
  const newLabel = row.kind === "ctx" ? "" : row.newNo !== undefined ? row.newNo : "";
  return (
    <div className={`file-edit-row file-edit-row-${row.kind}`}>
      <span className="file-edit-gutter">{row.oldNo !== undefined ? row.oldNo : ""}</span>
      <span className="file-edit-gutter">{newLabel}</span>
      <span className="file-edit-sign" aria-hidden="true">{sign}</span>
      <span className="file-edit-code">{content}</span>
    </div>
  );
}
