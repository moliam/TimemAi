import { setBetaPreference, useBetaPreference } from "./beta_preferences";
import { t, useT } from "./i18n";
export function setStreamUiMode(value: boolean) { setBetaPreference("stream", value); }
export function useStreamUiMode() { return useBetaPreference("stream"); }
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
  useT();
  return <section className="settings-group toolgen-beta-card"><div className="settings-group-heading"><div>
    <strong>{t("beta.streamUiTitle")}</strong><p>{t("beta.streamUiDesc")}</p>
    </div><button type="button" role="switch" className="settings-feature-switch" aria-label={t("beta.streamUiAria")} aria-checked={active} onClick={() => setStreamUiMode(!active)}><span className="settings-feature-switch-thumb" /></button></div></section>;
}
