export type Theme = "dark" | "light";
export type UiFont = "system" | "serif" | "mono";
export type ChineseFont = "system" | "heiti" | "kaiti" | "songti";
export type TextSize = "small" | "medium" | "large";

export type Appearance = {
  theme: Theme;
  userFont: UiFont;
  userChineseFont: ChineseFont;
  userBold: boolean;
  agentFont: UiFont;
  agentChineseFont: ChineseFont;
  agentBold: boolean;
  textSize: TextSize;
};

export const APPEARANCE_STORAGE_KEY = "timem-web-appearance-v1";

export function defaultAppearance(prefersLight: boolean): Appearance {
  return {
    theme: prefersLight ? "light" : "dark",
    userFont: "system",
    userChineseFont: "system",
    userBold: true,
    agentFont: "system",
    agentChineseFont: "system",
    agentBold: false,
    textSize: "medium",
  };
}

function parseUiFont(value: unknown, fallback: UiFont): UiFont {
  return value === "system" || value === "serif" || value === "mono" ? value : fallback;
}

function parseChineseFont(value: unknown, fallback: ChineseFont): ChineseFont {
  return value === "sans" ? "heiti"
    : value === "serif" ? "songti"
      : value === "system" || value === "heiti" || value === "kaiti" || value === "songti" ? value : fallback;
}

export function parseAppearance(raw: string | null, prefersLight: boolean): Appearance {
  const fallback = defaultAppearance(prefersLight);
  if (!raw) return fallback;
  try {
    const value = JSON.parse(raw) as Partial<Appearance> & { font?: unknown; chineseFont?: unknown };
    const legacyFont = parseUiFont(value.font, fallback.userFont);
    const legacyChineseFont = parseChineseFont(value.chineseFont, fallback.userChineseFont);
    return {
      theme: value.theme === "dark" || value.theme === "light" ? value.theme : fallback.theme,
      userFont: parseUiFont(value.userFont, legacyFont),
      userChineseFont: parseChineseFont(value.userChineseFont, legacyChineseFont),
      userBold: typeof value.userBold === "boolean" ? value.userBold : fallback.userBold,
      agentFont: parseUiFont(value.agentFont, legacyFont),
      agentChineseFont: parseChineseFont(value.agentChineseFont, legacyChineseFont),
      agentBold: typeof value.agentBold === "boolean" ? value.agentBold : fallback.agentBold,
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
  root.dataset.userFont = appearance.userFont;
  root.dataset.userChineseFont = appearance.userChineseFont;
  root.dataset.userBold = String(appearance.userBold);
  root.dataset.agentFont = appearance.agentFont;
  root.dataset.agentChineseFont = appearance.agentChineseFont;
  root.dataset.agentBold = String(appearance.agentBold);
  root.dataset.textSize = appearance.textSize;
  try {
    window.localStorage.setItem(APPEARANCE_STORAGE_KEY, JSON.stringify(appearance));
  } catch {
    // Hardened browser profiles may disable storage; the current page still updates.
  }
}
