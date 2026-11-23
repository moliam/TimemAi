import { useSyncExternalStore } from "react";

// UI-only preferences. Host debug mode supplies defaults, never overrides a choice.
const keys = {
  stream: "timem-web-stream-ui-mode-v1",
  streamArchive: "timem-web-stream-archive-v1",
  toolResults: "timem-web-tool-result-status-v1",
  autoCollapse: "timem-web-turn-auto-collapse-v1",
  fileEditsWrap: "timem-web-file-edits-wrap-v1",
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
  stream: read("stream"), streamArchive: read("streamArchive"), toolResults: read("toolResults"),
  autoCollapse: read("autoCollapse"), fileEditsWrap: read("fileEditsWrap"),
};
// Per-preference defaults. Every current switch is user-facing, so each
// keeps its fixed default in every mode: streaming display, tool status,
// turn auto-collapse and diff wrapping default on; the stream archive
// sub-setting stays off.
const defaults: Record<Preference, boolean> = {
  stream: true, streamArchive: false, toolResults: true,
  autoCollapse: true, fileEditsWrap: true,
};
function notify() { listeners.forEach(listener => listener()); }
// Debug-exempt switches ignore the Host debug default entirely: their
// user-facing contract is fixed (archive and diff wrap default off, turn
// auto-collapse defaults on) in every mode.
const debugExempt: ReadonlySet<Preference> = new Set([
  "stream", "streamArchive", "toolResults", "autoCollapse", "fileEditsWrap",
]);
export function applyBetaDebugDefault(debug: boolean) {
  if (debugDefault === debug) return;
  debugDefault = debug;
  notify();
}
export function getBetaPreference(name: Preference) {
  return choices[name] ?? (debugExempt.has(name) ? defaults[name] : debugDefault || defaults[name]);
}
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
