/**
 * Confirmed MEM-switch boundary presentation.
 *
 * The parent owns Session inspection, switch state, command delivery, and
 * authoritative outcomes. This component only presents the destructive
 * execution boundary and emits close/confirm intent.
 */
import { CircleStop, LoaderCircle, X } from "lucide-react";

export type MemSwitchCandidate = {
  path: string;
  runningSessionCount: number;
};

export type MemSwitchConfirmDialogProps = {
  candidate: MemSwitchCandidate;
  pending: boolean;
  onClose: () => void;
  onConfirm: () => void;
};

export function shellQuoteCommandArgument(value: string) {
  return `'${value.replaceAll("'", `'"'"'`)}'`;
}

export function MemSwitchConfirmDialog({
  candidate,
  pending,
  onClose,
  onConfirm,
}: MemSwitchConfirmDialogProps) {
  const closeIfIdle = () => {
    if (!pending) onClose();
  };
  const descriptionId = "mem-switch-confirm-description";
  const statusId = "mem-switch-confirm-status";
  return (
    <div
      className="modal-backdrop"
      role="presentation"
      aria-label="Dismiss MEM switch confirmation"
      onClick={closeIfIdle}
    >
      <section
        className="decision-modal mem-switch-confirm-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="mem-switch-confirm-title"
        aria-describedby={
          pending ? `${descriptionId} ${statusId}` : descriptionId
        }
        onClick={(event) => event.stopPropagation()}
        onKeyDown={(event) => {
          if (event.key === "Escape") {
            event.preventDefault();
            event.stopPropagation();
            closeIfIdle();
          }
        }}
      >
        <div className="modal-titlebar">
          <div>
            <span className="eyebrow">STOP AND SWITCH</span>
            <h2 id="mem-switch-confirm-title">Stop current MEM work?</h2>
          </div>
          <button
            type="button"
            className="icon-button"
            title="Close MEM switch confirmation"
            aria-label="Close MEM switch confirmation"
            disabled={pending}
            onClick={closeIfIdle}
          >
            <X size={16} />
          </button>
        </div>
        <p id={descriptionId}>
          Switching MEM will stop all running and queued work in the current
          MEM. {candidate.runningSessionCount} affected Session
          {candidate.runningSessionCount === 1 ? "" : "s"} will be marked
          interrupted. Nothing from the current MEM will continue or restart
          automatically.
        </p>
        <p className="mem-switch-alternative">
          To keep the current work running, start a separate instance for the
          destination MEM instead:{" "}
          <code>timem --space {shellQuoteCommandArgument(candidate.path)}</code>
        </p>
        <code className="mem-switch-confirm-path" title={candidate.path}>
          {candidate.path}
        </code>
        {pending && (
          <p
            id={statusId}
            className="session-delete-status"
            role="status"
            aria-live="polite"
          >
            Stopping current MEM workers and switching…
          </p>
        )}
        <div className="decision-actions">
          <button
            type="button"
            className="secondary"
            disabled={pending}
            onClick={closeIfIdle}
          >
            Keep current MEM
          </button>
          <button
            type="button"
            className={`danger ${pending ? "sending" : ""}`}
            disabled={pending}
            onClick={onConfirm}
          >
            {pending ? <LoaderCircle size={15} /> : <CircleStop size={15} />} {" "}
            {pending ? "Stopping and switching…" : "Stop work and switch"}
          </button>
        </div>
      </section>
    </div>
  );
}
