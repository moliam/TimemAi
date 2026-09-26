/**
 * Restart working-directory decision presentation.
 *
 * Scope:
 * - render the authoritative Host decision projection;
 * - expose only the two typed decisions already defined by the wire protocol;
 * - hide the impossible "keep old directory" action when that directory no
 *   longer exists.
 *
 * Constraints:
 * - this component owns no Session state, command retry, or optimistic update;
 * - it must not infer lifecycle from paths or visible text;
 * - the parent remains responsible for command delivery and interaction locks.
 */
import { FolderOpen } from "lucide-react";
import { t, useT } from "./i18n";
import type { Session } from "./protocol";

type RestartCwdDecision = NonNullable<Session["restart_cwd_decision"]>;

export type RestartCwdGateProps = {
  decision: RestartCwdDecision;
  enabled: boolean;
  onResolve: (decision: "use_runtime" | "keep_session") => void;
};

export function RestartCwdGate({
  decision,
  enabled,
  onResolve,
}: RestartCwdGateProps) {
  useT();
  const canKeepSessionDirectory = decision.session_cwd_available;
  return (
    <section
      className="restart-cwd-gate"
      role="alertdialog"
      aria-live="assertive"
      aria-labelledby="restart-cwd-title"
    >
      <div className="restart-cwd-gate-copy">
        <span className="restart-cwd-gate-icon" aria-hidden="true">
          <FolderOpen size={17} />
        </span>
        <p id="restart-cwd-title">
          {canKeepSessionDirectory
            ? t("restartGate.mismatchPrompt")
            : t("restartGate.missingPrompt")}
        </p>
      </div>
      <div className="restart-cwd-options">
        <div className="restart-cwd-option">
          <button
            type="button"
            disabled={!enabled}
            onClick={() => onResolve("use_runtime")}
          >
            {canKeepSessionDirectory ? t("restartGate.switchToRuntime") : t("restartGate.useRuntime")}
          </button>
          {canKeepSessionDirectory && (
            <>
              <span>{t("restartGate.toNewRuntime")}</span>
              <code title={decision.runtime_cwd}>{decision.runtime_cwd}</code>
            </>
          )}
        </div>
        {canKeepSessionDirectory && (
          <div className="restart-cwd-option">
            <button
              type="button"
              className="secondary"
              disabled={!enabled}
              onClick={() => onResolve("keep_session")}
            >
              {t("restartGate.keepInSession")}
            </button>
            <span>{t("restartGate.inOldSession")}</span>
            <code title={decision.session_cwd}>{decision.session_cwd}</code>
          </div>
        )}
      </div>
    </section>
  );
}
