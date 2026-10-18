import { useCallback, useEffect, useState } from "react";
import { t, useT } from "./i18n";

type UpgradePhase =
  | { phase: "idle" }
  | { phase: "checking" }
  | { phase: "no_update"; current_version: string }
  | { phase: "update_available"; current_version: string; remote_version: string }
  | { phase: "downloading"; remote_version: string; percent: number }
  | { phase: "verifying"; remote_version: string }
  | { phase: "staging"; remote_version: string }
  | { phase: "applying"; remote_version: string }
  | { phase: "pending_restart"; current_version: string; pending_version: string }
  | { phase: "failed"; error: string };

type UpgradeStatus = { phase: UpgradePhase; current_version: string };

function queryToken() {
  try {
    return window.sessionStorage.getItem("timem-web-access-token") ?? "";
  } catch {
    return "";
  }
}

function upgradeUrl(path: string): string {
  const token = queryToken();
  const query = token ? `?token=${encodeURIComponent(token)}` : "";
  return `/api/upgrade/${path}${query}`;
}

async function postUpgrade(path: string): Promise<boolean> {
  try {
    const response = await fetch(upgradeUrl(path), { method: "POST" });
    return response.ok;
  } catch {
    return false;
  }
}

export function SelfUpgradeSetting() {
  useT();
  const [status, setStatus] = useState<UpgradeStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [restarting, setRestarting] = useState(false);
  const refresh = useCallback(async (): Promise<UpgradePhase | null> => {
    try {
      const response = await fetch(upgradeUrl("status"));
      if (!response.ok) return null;
      const data = (await response.json()) as UpgradeStatus;
      setStatus(data);
      return data.phase;
    } catch {
      return null;
    }
  }, []);

  useEffect(() => {
    let cancelled = false;
    let timer: number | undefined;
    const poll = async () => {
      const phase = await refresh();
      if (cancelled) return;
      const active =
        phase?.phase === "checking" ||
        phase?.phase === "downloading" ||
        phase?.phase === "verifying" ||
        phase?.phase === "staging" ||
        phase?.phase === "applying";
      if (active) timer = window.setTimeout(poll, 800);
      else timer = window.setTimeout(poll, 5000);
    };
    poll();
    return () => {
      cancelled = true;
      if (timer) window.clearTimeout(timer);
    };
  }, [refresh]);

  const onCheck = async () => {
    setBusy(true);
    await postUpgrade("check");
    window.setTimeout(refresh, 400);
    window.setTimeout(refresh, 1500);
    setBusy(false);
  };

  const onApply = async () => {
    setBusy(true);
    await postUpgrade("apply");
    window.setTimeout(refresh, 400);
    setBusy(false);
  };

  const onRestart = async () => {
    setRestarting(true);
    await postUpgrade("restart");
    // The runtime exits gracefully; the connection drops and the
    // reconnecting transport picks the replacement process back up.
    window.setTimeout(() => window.location.reload(), 8000);
  };

  const phase = status?.phase ?? { phase: "idle" as const };
  const phaseName = phase.phase;
  const currentVersion = status?.current_version ?? "";
  const busyPhase =
    phaseName === "checking" ||
    phaseName === "downloading" ||
    phaseName === "verifying" ||
    phaseName === "staging" ||
    phaseName === "applying";

  return (
    <section className="settings-group system-upgrade-card">
      <div className="settings-group-heading">
        <div>
          <strong>{t("system.upgradeTitle")}</strong>
          <p>{t("system.upgradeDesc")}</p>
        </div>
        <span className="system-upgrade-version">v{currentVersion}</span>
      </div>
      <div className="system-upgrade-body">
        {phaseName === "idle" && (
          <button
            type="button"
            className="system-upgrade-button"
            onClick={onCheck}
            disabled={busy || restarting}
          >
            {t("system.upgradeCheck")}
          </button>
        )}
        {phaseName === "checking" && (
          <span className="system-upgrade-status" role="status" aria-live="polite">
            {t("system.upgradeChecking")}
          </span>
        )}
        {phaseName === "no_update" && (
          <div className="system-upgrade-status ok" role="status" aria-live="polite">
            <span>{t("system.upgradeUpToDate", { version: currentVersion })}</span>
            <button
              type="button"
              className="system-upgrade-button"
              onClick={onCheck}
              disabled={busy || restarting}
            >
              {t("system.upgradeRecheck")}
            </button>
          </div>
        )}
        {phaseName === "update_available" && (
          <div className="system-upgrade-status" role="status" aria-live="polite">
            <span>
              {t("system.upgradeAvailable", {
                current: currentVersion,
                next: phase.remote_version,
              })}
            </span>
            <button
              type="button"
              className="system-upgrade-button primary"
              onClick={onApply}
              disabled={busy || restarting}
            >
              {t("system.upgradeApply")}
            </button>
          </div>
        )}
        {phaseName === "downloading" && (
          <div className="system-upgrade-status" role="status" aria-live="polite">
            <span>
              {t("system.upgradeDownloading", { version: phase.remote_version })}
            </span>
            <span className="system-upgrade-progress">
              <span style={{ width: `${Math.min(100, Math.max(0, phase.percent))}%` }} />
            </span>
            <span className="system-upgrade-percent">{phase.percent}%</span>
          </div>
        )}
        {(phaseName === "verifying" || phaseName === "staging" || phaseName === "applying") && (
          <span className="system-upgrade-status" role="status" aria-live="polite">
            {phaseName === "verifying"
              ? t("system.upgradeVerifying")
              : phaseName === "staging"
                ? t("system.upgradeStaging")
                : t("system.upgradeApplying")}
          </span>
        )}
        {phaseName === "pending_restart" && (
          <div className="system-upgrade-status ok" role="status" aria-live="polite">
            <span>
              {t("system.upgradePendingRestart", { version: phase.pending_version })}
            </span>
            <button
              type="button"
              className="system-upgrade-button primary"
              onClick={onRestart}
              disabled={busy || restarting}
            >
              {restarting ? t("system.upgradeRestarting") : t("system.upgradeRestartNow")}
            </button>
          </div>
        )}
        {phaseName === "failed" && (
          <div className="system-upgrade-status error" role="alert">
            <span>{t("system.upgradeFailed")}</span>
            <code className="system-upgrade-error">{phase.error}</code>
            <button
              type="button"
              className="system-upgrade-button"
              onClick={onCheck}
              disabled={busy || restarting}
            >
              {t("system.upgradeRecheck")}
            </button>
          </div>
        )}
        {busyPhase && <span className="system-setting-status">{t("system.saving")}</span>}
      </div>
    </section>
  );
}
