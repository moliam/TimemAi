// Translation entry point. `t` is a plain function reading module state, so it
// stays callable in non-hook contexts (view-model helpers, event callbacks).
// Reactivity contract: a component that renders t() output must subscribe via
// useT() (or render inside a subscribed ancestor); memoized subtrees that show
// UI chrome must call useT() themselves. Never cache t() output at module
// scope — a locale switch would not refresh it.
import { getLocale, useLocale, type Locale } from "./locale";
import { zh, type StringKey, type Strings } from "./strings.zh";
import { en } from "./strings.en";

const catalogs: Record<Locale, Strings> = { zh, en };

export type { Locale, StringKey, Strings };
export { getLocale, setLocale, useLocale, LOCALE_STORAGE_KEY } from "./locale";

export type TParams = Record<string, string | number>;

function interpolate(template: string, params: TParams | undefined): string {
  if (!params) return template;
  return template.replace(/\{(\w+)\}/g, (match, name: string) =>
    Object.prototype.hasOwnProperty.call(params, name) ? String(params[name]) : match,
  );
}

export function t(key: StringKey, params?: TParams): string {
  const [domain, name] = key.split(".") as [keyof Strings, string];
  const entry = (catalogs[getLocale()] as Record<string, unknown>)[domain] as
    | Record<string, string>
    | undefined;
  const template = entry?.[name];
  if (typeof template !== "string") return key;
  return interpolate(template, params);
}

export function useT() {
  useLocale();
  return t;
}
