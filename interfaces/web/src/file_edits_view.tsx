import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
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
import { useSelectionGestureGuard } from "./selection_gesture";

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

// A per-path entry in the turn-level change summary.
export type TurnEditEntry = FileEditPresentation;

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
  // (128 paths) rather than the per-tool-call card budget.
  const edits = fileEditsPresentation(summary.edits, { maxEntries: 128 });
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

/// Auto-expanding a very large diff while a turn is still streaming inserts
/// thousands of rows into the live DOM. Keep those folded until the reader
/// explicitly opens them; the header still carries the complete +/- stats.
export const MAX_AUTO_EXPAND_DIFF_CHARS = 6_000;

export function shouldAutoExpandDiff(
  diff: string | null | undefined,
  options: { autoCollapse: boolean; expandDuringWork: boolean },
): boolean {
  if (!diff) return false;
  if (diff.length > MAX_AUTO_EXPAND_DIFF_CHARS) return false;
  return options.expandDuringWork || !options.autoCollapse;
}

/// A diff the runtime had to truncate is a static summary row: expanding a
/// partial diff behind a "truncated" notice only showed an incomplete body.
export function fileEditSectionExpandable(edit: FileEditPresentation): boolean {
  return !!edit.diff && !edit.truncated;
}

/// A diff body only stays mounted while it is on screen. Collapsing an
/// off-screen section keeps historical diffs from accumulating DOM; focus and
/// text selection win over the automatic cleanup.
export function shouldCollapseOffscreenDiff(input: {
  expanded: boolean;
  intersecting: boolean;
  focusInside: boolean;
  selectionInside: boolean;
}): boolean {
  if (!input.expanded || input.intersecting) return false;
  return !input.focusInside && !input.selectionInside;
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
  const autoExpand = shouldAutoExpandDiff(edit.diff, { autoCollapse, expandDuringWork });
  const [expanded, setExpanded] = useState(autoExpand);
  const manualToggle = useRef(false);
  useEffect(() => {
    if (manualToggle.current) return;
    setExpanded(autoExpand);
  }, [autoExpand]);
  const sectionRef = useRef<HTMLDivElement | null>(null);
  const pendingCollapseCompensation = useRef<{
    viewport: HTMLElement;
    beforeHeight: number;
  } | null>(null);
  const { copyState, copy, copyLabel, copyClass } = useTimedClipboardCopy(edit.diff ?? "", {
    idle: t("tools.copyDiff"),
    copied: t("tools.diffCopied"),
    failed: t("tools.copyFailed"),
  });
  // Parse only while the body is mounted: a folded section must not pay the
  // diff parser cost for thousands of rows that are not in the DOM.
  const rows = useMemo(
    () => (expanded && edit.diff ? parseDiffRows(edit.diff) : []),
    [edit.diff, expanded],
  );
  const { directory, name } = splitEditPath(edit.path);
  const expandable = fileEditSectionExpandable(edit);
  // Non-text edits carry no diff; the capture size rides in the tag tooltip.
  const kindLabel = !expandable && edit.skip_reason
    ? t(KIND_LABEL_KEYS[edit.skip_reason] ?? ("tools.fileKindUnreadable" as StringKey))
    : undefined;
  const sizeLabel = expandable ? undefined : formatFileSize(edit.size_bytes);
  const kindTitle = kindLabel
    ? sizeLabel ? `${kindLabel} · ${sizeLabel}` : kindLabel
    : undefined;
  // User feedback: selecting text inside a diff must not collapse/expand it (that
  // made the page jump). A drag, a gesture starting in selectable text, or any
  // live selection belongs to the browser: it only selects. ASCII only here:
  // this view file is checked to stay free of literal CJK strings.
  const { onPointerDown: trackSelectionGesture, ignoreToggle } = useSelectionGestureGuard();
  const toggle = (event?: { clientX: number; clientY: number }) => {
    if (!expandable) return;
    if (event && ignoreToggle(event)) return;
    manualToggle.current = true;
    setExpanded(current => !current);
  };
  // Screen-bounded retention: an expanded diff body lives only while its
  // section is near the active chat viewport. Historical turns are already
  // collapsed at the turn level, so this bounds the one turn the reader is in.
  useEffect(() => {
    if (!expanded || !edit.diff) return;
    const node = sectionRef.current;
    if (!node || typeof IntersectionObserver === "undefined") return;
    const viewport = node.closest<HTMLElement>(".chat-scroll");
    if (!viewport) return;
    let timer = 0;
    const observer = new IntersectionObserver(
      entries => {
        const entry = entries[entries.length - 1];
        if (!entry) return;
        if (entry.isIntersecting) {
          if (timer) {
            window.clearTimeout(timer);
            timer = 0;
          }
          return;
        }
        if (timer) return;
        timer = window.setTimeout(() => {
          timer = 0;
          if (!node.isConnected) return;
          const viewportRect = viewport.getBoundingClientRect();
          const rect = node.getBoundingClientRect();
          const withinMargin =
            rect.bottom > viewportRect.top - 120 &&
            rect.top < viewportRect.bottom + 120;
          const selection = window.getSelection();
          const selectionInside =
            !!selection &&
            !selection.isCollapsed &&
            !!selection.anchorNode &&
            node.contains(selection.anchorNode);
          const focusInside = node.contains(document.activeElement);
          if (
            withinMargin ||
            !shouldCollapseOffscreenDiff({
              expanded: true,
              intersecting: false,
              focusInside,
              selectionInside,
            })
          )
            return;
          if (rect.bottom <= viewportRect.top)
            pendingCollapseCompensation.current = {
              viewport,
              beforeHeight: rect.height,
            };
          setExpanded(false);
        }, 200);
      },
      { root: viewport, rootMargin: "120px 0px", threshold: 0 },
    );
    observer.observe(node);
    return () => {
      observer.disconnect();
      if (timer) window.clearTimeout(timer);
    };
  }, [edit.diff, expanded]);

  // Collapsing a section above the viewport removes height above the reader;
  // restore scrollTop in the same layout pass so the visible content does not
  // jump upward when the body is unmounted.
  useLayoutEffect(() => {
    const pending = pendingCollapseCompensation.current;
    if (!pending) return;
    pendingCollapseCompensation.current = null;
    const node = sectionRef.current;
    if (!node || !pending.viewport.isConnected) return;
    const afterHeight = node.getBoundingClientRect().height;
    const delta = pending.beforeHeight - afterHeight;
    if (delta > 0) pending.viewport.scrollTop -= delta;
  }, [expanded]);

  return (
    <div className="file-edit-section" ref={sectionRef}>
      <div
        className={`file-edit-head${expandable ? " file-edit-toggle" : ""}`}
        role={expandable ? "button" : undefined}
        tabIndex={expandable ? 0 : undefined}
        aria-expanded={expandable ? expanded : undefined}
        aria-label={expandable ? (expanded ? t("tools.collapseOutput") : t("tools.expandOutput")) : undefined}
        onPointerDown={trackSelectionGesture}
        onClick={event => toggle(event)}
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
        {kindLabel && (
          <span className="file-edit-kind" title={kindTitle}>{kindLabel}</span>
        )}
        {(edit.added_lines > 0 || edit.removed_lines > 0) && (
          <span className="file-edit-stats">
            {edit.added_lines > 0 && <span className="file-edit-additions">+{edit.added_lines}</span>}
            {edit.removed_lines > 0 && <span className="file-edit-deletions">&minus;{edit.removed_lines}</span>}
          </span>
        )}
        {edit.truncated && !edit.skip_reason && (
          <span className="file-edit-kind">{t("tools.fileDiffTooLarge")}</span>
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
        </div>
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
