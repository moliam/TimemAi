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
export function FileEditsPanel({ edits }: { edits: ReadonlyArray<TurnEditEntry> }) {
  return (
    <div className="file-edits-panel" aria-label={t("tools.fileEditsAria")}>
      {edits.map((edit, index) => (
        <FileEditSection key={`${edit.path}:${index}`} edit={edit} />
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

// Fail-closed budget: the folded summary stays bounded even when a turn
// touched a huge number of files; the overflow note reports the remainder.
const TURN_EDITS_SUMMARY_LIMIT = 64;

// Per-path capture state across the whole turn. The first non-skipped op
// defines whether the path existed at the turn start ("created" means it did
// not); the last non-skipped capture defines the end state.
type TurnEditPathState = {
  captures: number;
  startOp: FileEditOp | null;
  sawCaptured: boolean;
  display: FileEditPresentation | null;
};

function trackTurnEditCapture(state: TurnEditPathState, next: FileEditPresentation): void {
  state.captures += 1;
  if (next.op === "skipped") {
    // A skipped capture carries no fresh evidence about existence; while no
    // real capture has landed yet, the latest skip reason is the only
    // reportable fact for this path.
    if (!state.sawCaptured) state.display = next;
    return;
  }
  if (state.startOp === null) state.startOp = next.op;
  state.sawCaptured = true;
  state.display = next;
}

// Net change of one path across the whole turn (start state vs end state).
// A file that was created mid-turn and deleted before the turn ended never
// existed at either boundary, so it is dropped entirely instead of leaving a
// misleading "deleted" row behind.
function netTurnEditEntry(state: TurnEditPathState): TurnEditEntry | null {
  const display = state.display;
  if (!display) return null;
  if (!state.sawCaptured) {
    // Nothing was ever captured: report the latest skip verbatim.
    return { ...display, capture_count: state.captures };
  }
  const startExisted = state.startOp !== "created";
  const endExisted = display.op !== "deleted";
  if (!startExisted && !endExisted) return null;
  let op: FileEditOp = display.op;
  if (!startExisted) op = "created";
  else if (display.op === "created") op = "modified";
  return { ...display, op, capture_count: state.captures };
}

// Aggregate every tool call's file edits of one turn into a per-path summary
// of the net begin-to-end change: first-touch order, bounded size. The full
// per-call evidence stays on the individual tool cards; only files that
// actually differ between the turn's start and end appear here.
export function summarizeTurnEdits(
  groups: ReadonlyArray<FileEditsPresentation | undefined | null> | undefined | null,
): TurnEditsSummary {
  const states = new Map<string, TurnEditPathState>();
  const list = Array.isArray(groups) ? groups : [];
  for (const group of list) {
    if (!Array.isArray(group)) continue;
    for (const edit of group) {
      if (!edit || typeof edit.path !== "string" || !edit.path) continue;
      let state = states.get(edit.path);
      if (!state) {
        state = { captures: 0, startOp: null, sawCaptured: false, display: null };
        states.set(edit.path, state);
      }
      trackTurnEditCapture(state, edit);
    }
  }
  const edits: TurnEditEntry[] = [];
  let overflowCount = 0;
  for (const state of states.values()) {
    const entry = netTurnEditEntry(state);
    if (!entry) continue;
    if (edits.length >= TURN_EDITS_SUMMARY_LIMIT) {
      overflowCount += 1;
      continue;
    }
    edits.push(entry);
  }
  return { edits, overflowCount };
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

function splitEditPath(path: string): { directory: string; name: string } {
  const slash = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
  return {
    directory: slash >= 0 ? path.slice(0, slash + 1) : "",
    name: slash >= 0 ? path.slice(slash + 1) : path,
  };
}

function FileEditSection({ edit }: { edit: TurnEditEntry }) {
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
        {(edit.capture_count ?? 0) > 1 && (
          <span
            className="file-edit-captures"
            title={t("tools.captureCount", { count: edit.capture_count ?? 0 })}
          >
            ×{edit.capture_count}
          </span>
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
