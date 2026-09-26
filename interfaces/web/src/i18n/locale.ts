import { useSyncExternalStore } from "react";

// UI 语言是浏览器本地的呈现偏好，与 appearance/beta_preferences 同构：
// 不进入 Host 权威状态，不影响任何协议或会话语义。
// 约束（详见 docs/web-i18n-architecture.md）：
// 1. 用户可见文案必须经 strings 目录与 useT/t 获取；JSX/CSS 内联可见文本是缺陷。
// 2. 目录只承载 Interface 呈现文本；模型输出、Session/Role 等用户数据、Host
//    权威状态文本永不翻译。
// 3. strings.zh 是源目录；strings.en 必须覆盖全部键（类型层面强制齐平）。
export type Locale = "zh" | "en";

export const LOCALE_STORAGE_KEY = "timem-web-locale-v1";

const listeners = new Set<() => void>();
let current: Locale = resolveInitial();

function parseLocale(raw: string | null): Locale | undefined {
  return raw === "zh" || raw === "en" ? raw : undefined;
}

function browserLocale(): Locale {
  try {
    for (const tag of window.navigator.languages ?? []) {
      if (tag.toLowerCase().startsWith("zh")) return "zh";
      if (tag.toLowerCase().startsWith("en")) return "en";
    }
  } catch { /* Hardened profiles may block navigator; fall through. */ }
  return "zh";
}

function resolveInitial(): Locale {
  try {
    return parseLocale(window.localStorage.getItem(LOCALE_STORAGE_KEY)) ?? browserLocale();
  } catch {
    return browserLocale();
  }
}

function notify() { listeners.forEach(listener => listener()); }

/** Applies the accessibility language and persists; storage denial keeps a tab-local choice. */
export function setLocale(locale: Locale) {
  current = locale;
  applyDocumentLanguage(locale);
  try {
    // Store the bare tag, not JSON: parseLocale expects the exact literal.
    window.localStorage.setItem(LOCALE_STORAGE_KEY, locale);
  } catch { /* Storage unavailable: retain this tab's choice. */ }
  notify();
}

export function getLocale(): Locale { return current; }

export function applyDocumentLanguage(locale: Locale) {
  try { document.documentElement.lang = locale === "zh" ? "zh-CN" : "en"; } catch { /* Non-DOM test env. */ }
}

if (typeof window !== "undefined") {
  applyDocumentLanguage(current);
  window.addEventListener("storage", event => {
    if (event.key !== LOCALE_STORAGE_KEY && event.key !== null) return;
    try {
      const next = parseLocale(window.localStorage.getItem(LOCALE_STORAGE_KEY));
      if (next && next !== current) { current = next; applyDocumentLanguage(next); notify(); }
    } catch { /* Keep the current locale when storage is unreadable. */ }
  });
}

export function useLocale() {
  return useSyncExternalStore(listener => {
    listeners.add(listener);
    return () => { listeners.delete(listener); };
  }, getLocale, () => "zh" as Locale);
}
