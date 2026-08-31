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
            ? "当前 Timem 的启动目录和 Session 上次工作的目录不同，您要将工作目录："
            : "原工作目录已不可用。聊天记录已保留，请切换到当前启动目录后继续："}
        </p>
      </div>
      <div className="restart-cwd-options">
        <div className="restart-cwd-option">
          <button
            type="button"
            disabled={!enabled}
            onClick={() => onResolve("use_runtime")}
          >
            {canKeepSessionDirectory ? "切换" : "使用当前工作目录"}
          </button>
          {canKeepSessionDirectory && (
            <>
              <span>至新启动目录：</span>
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
              保持
            </button>
            <span>在旧工作目录：</span>
            <code title={decision.session_cwd}>{decision.session_cwd}</code>
          </div>
        )}
      </div>
    </section>
  );
}
