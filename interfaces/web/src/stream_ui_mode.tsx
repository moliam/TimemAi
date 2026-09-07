import { setBetaPreference, useBetaPreference } from "./beta_preferences";
export function setStreamUiMode(value: boolean) { setBetaPreference("stream", value); }
export function useStreamUiMode() { return useBetaPreference("stream"); }
export function useToolResultStatus() { return useBetaPreference("toolResults"); }
export function ToolResultStatusSetting() {
  const active = useToolResultStatus();
  return <section className="settings-group toolgen-beta-card"><div className="settings-group-heading"><div>
    <strong>Tool Result Status</strong><p>Show success or failure based on tool return values. When off, finished calls show Done. This describes execution, not task correctness. Defaults on with --debug; your choice is stored in this browser.</p>
    </div><button type="button" role="switch" className="settings-feature-switch" aria-label="Tool Result Status" aria-checked={active} onClick={() => setBetaPreference("toolResults", !active)}><span className="settings-feature-switch-thumb" /></button></div></section>;
}
export function StreamUiModeSetting() {
  const active = useStreamUiMode();
  return <section className="settings-group toolgen-beta-card"><div className="settings-group-heading"><div>
    <strong>Stream UI Mode</strong><p>Show response and Chat previews while the model is streaming. Invalid replies are retracted. Defaults on with --debug; your choice is stored only in this browser.</p>
    </div><button type="button" role="switch" className="settings-feature-switch" aria-label="Stream UI Mode" aria-checked={active} onClick={() => setStreamUiMode(!active)}><span className="settings-feature-switch-thumb" /></button></div></section>;
}
