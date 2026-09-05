import { useSyncExternalStore } from "react";
const key = "timem-web-stream-ui-mode-v1";
const listeners = new Set<() => void>();
function read(): boolean {
  try { return window.localStorage.getItem(key) === "true"; } catch { return false; }
}
let enabled = read();
export function setStreamUiMode(value: boolean) {
  enabled = value;
  try { window.localStorage.setItem(key, JSON.stringify(value)); } catch { /* Page-local mode still works. */ }
  listeners.forEach((listener) => listener());
}
if (typeof window !== "undefined") window.addEventListener("storage", (event) => {
  if (event.key === key || event.key === null) { enabled = read(); listeners.forEach((listener) => listener()); }
});
export function useStreamUiMode() {
  return useSyncExternalStore((listener) => { listeners.add(listener); return () => { listeners.delete(listener); }; }, () => enabled, () => false);
}
export function StreamUiModeSetting() {
  const active = useStreamUiMode();
  return <section className="settings-group toolgen-beta-card"><div className="settings-group-heading"><div>
    <strong>Stream UI Mode</strong><p>Show response and Chat previews while the model is streaming. Invalid replies are retracted. Stored only in this browser.</p>
    </div><button type="button" role="switch" className="settings-feature-switch" aria-label="Stream UI Mode" aria-checked={active} onClick={() => setStreamUiMode(!active)}><span className="settings-feature-switch-thumb" /></button></div></section>;
}
