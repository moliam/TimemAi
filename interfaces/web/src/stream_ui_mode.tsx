import { setBetaPreference, useBetaPreference } from "./beta_preferences";
import { t, useT } from "./i18n";
export function setStreamUiMode(value: boolean) { setBetaPreference("stream", value); }
export function useStreamUiMode() { return useBetaPreference("stream"); }
export function setStreamArchive(value: boolean) { setBetaPreference("streamArchive", value); }
export function useStreamArchive() { return useBetaPreference("streamArchive"); }
export function useToolResultStatus() { return useBetaPreference("toolResults"); }
export function ToolResultStatusSetting() {
  const active = useToolResultStatus();
  useT();
  return <section className="settings-group toolgen-beta-card"><div className="settings-group-heading"><div>
    <strong>{t("beta.toolResultTitle")}</strong><p>{t("beta.toolResultDesc")}</p>
    </div><button type="button" role="switch" className="settings-feature-switch" aria-label={t("beta.toolResultAria")} aria-checked={active} onClick={() => setBetaPreference("toolResults", !active)}><span className="settings-feature-switch-thumb" /></button></div></section>;
}
export function StreamUiModeSetting() {
  const active = useStreamUiMode();
  const archive = useStreamArchive();
  useT();
  return <section className="settings-group system-feature-card"><div className="settings-group-heading"><div>
    <strong>{t("system.streamUiTitle")}</strong><p>{t("system.streamUiDesc")}</p>
    </div><button type="button" role="switch" className="settings-feature-switch" aria-label={t("system.streamUiAria")} aria-checked={active} onClick={() => setStreamUiMode(!active)}><span className="settings-feature-switch-thumb" /></button></div>
    {active && <div className="stream-archive-subsetting" role="group" aria-label={t("system.streamArchiveTitle")}>
      <div className="stream-archive-subsetting-text">
        <strong>{t("system.streamArchiveTitle")}</strong>
        <p>{t("system.streamArchiveDesc")}</p>
      </div>
      <button type="button" role="switch" className="settings-feature-switch" aria-label={t("system.streamArchiveAria")} aria-checked={archive} onClick={() => setStreamArchive(!archive)}><span className="settings-feature-switch-thumb" /></button>
    </div>}
    </section>;
}
