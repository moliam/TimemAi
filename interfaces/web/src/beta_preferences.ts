import { useSyncExternalStore } from "react";

// UI-only preferences. Host debug mode supplies defaults, never overrides a choice.
const keys = {
  stream: "timem-web-stream-ui-mode-v1",
  toolResults: "timem-web-tool-result-status-v1",
} as const;
type Preference = keyof typeof keys;
const listeners = new Set<() => void>();
let debugDefault = false;
function read(name: Preference): boolean | undefined {
  try {
    const raw = window.localStorage.getItem(keys[name]);
    return raw === "true" ? true : raw === "false" ? false : undefined;
  } catch { return undefined; }
}
const choices: Record<Preference, boolean | undefined> = {
  stream: read("stream"), toolResults: read("toolResults"),
};
function notify() { listeners.forEach(listener => listener()); }
export function applyBetaDebugDefault(debug: boolean) {
  if (debugDefault === debug) return;
  debugDefault = debug;
  notify();
}
export function getBetaPreference(name: Preference) { return choices[name] ?? debugDefault; }
export function setBetaPreference(name: Preference, value: boolean) {
  choices[name] = value;
  try { window.localStorage.setItem(keys[name], JSON.stringify(value)); }
  catch { /* Storage unavailable: retain this tab's explicit choice. */ }
  notify();
}
if (typeof window !== "undefined") window.addEventListener("storage", event => {
  for (const name of Object.keys(keys) as Preference[]) {
    if (event.key === keys[name] || event.key === null) choices[name] = read(name);
  }
  notify();
});
export function useBetaPreference(name: Preference) {
  return useSyncExternalStore(listener => {
    listeners.add(listener);
    return () => { listeners.delete(listener); };
  }, () => getBetaPreference(name), () => false);
}
