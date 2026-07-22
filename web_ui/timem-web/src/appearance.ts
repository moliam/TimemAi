export type Theme = "dark" | "light";
export type UiFont = "system" | "serif" | "mono";
export type TextSize = "small" | "medium" | "large";

export type Appearance = {
  theme: Theme;
  font: UiFont;
  textSize: TextSize;
};

export const APPEARANCE_STORAGE_KEY = "timem-web-appearance-v1";

export function defaultAppearance(prefersLight: boolean): Appearance {
  return {
    theme: prefersLight ? "light" : "dark",
    font: "system",
    textSize: "medium",
  };
}

export function parseAppearance(raw: string | null, prefersLight: boolean): Appearance {
  const fallback = defaultAppearance(prefersLight);
  if (!raw) return fallback;
  try {
    const value = JSON.parse(raw) as Partial<Appearance>;
    return {
      theme: value.theme === "dark" || value.theme === "light" ? value.theme : fallback.theme,
      font: value.font === "system" || value.font === "serif" || value.font === "mono" ? value.font : fallback.font,
      textSize: value.textSize === "small" || value.textSize === "medium" || value.textSize === "large" ? value.textSize : fallback.textSize,
    };
  } catch {
    return fallback;
  }
}

export function loadAppearance(): Appearance {
  const prefersLight = window.matchMedia("(prefers-color-scheme: light)").matches;
  try {
    return parseAppearance(window.localStorage.getItem(APPEARANCE_STORAGE_KEY), prefersLight);
  } catch {
    return defaultAppearance(prefersLight);
  }
}

export function applyAppearance(appearance: Appearance) {
  const root = document.documentElement;
  root.dataset.theme = appearance.theme;
  root.dataset.font = appearance.font;
  root.dataset.textSize = appearance.textSize;
  try {
    window.localStorage.setItem(APPEARANCE_STORAGE_KEY, JSON.stringify(appearance));
  } catch {
    // Hardened browser profiles may disable storage; the current page still updates.
  }
}
