export const TOOLGEN_ENABLED_STORAGE_KEY = "timem-web-toolgen-enabled-v1";

export function parseToolGenEnabled(raw: string | null): boolean {
  if (!raw) return false;
  try {
    return JSON.parse(raw) === true;
  } catch {
    return false;
  }
}

export function loadToolGenEnabled(): boolean {
  try {
    return parseToolGenEnabled(window.localStorage.getItem(TOOLGEN_ENABLED_STORAGE_KEY));
  } catch {
    return false;
  }
}

export function saveToolGenEnabled(enabled: boolean) {
  try {
    window.localStorage.setItem(TOOLGEN_ENABLED_STORAGE_KEY, JSON.stringify(enabled));
  } catch {
    // Restricted browser profiles may disable storage; the current page still updates.
  }
}
