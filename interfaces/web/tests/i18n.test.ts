import { describe, expect, it } from "vitest";
import { getLocale, setLocale, t } from "../src/i18n";
import { zh, type Strings } from "../src/i18n/strings.zh";
import { en } from "../src/i18n/strings.en";

function keysOf(catalog: Strings): string[] {
  return Object.entries(catalog).flatMap(([domain, entries]) =>
    Object.keys(entries).map((name) => `${domain}.${name}`),
  );
}

describe("i18n catalogs", () => {
  it("keep zh and en key sets identical", () => {
    expect(keysOf(en).sort()).toEqual(keysOf(zh).sort());
  });

  it("hold non-empty string values in both languages", () => {
    for (const catalog of [zh, en]) {
      for (const entries of Object.values(catalog)) {
        for (const value of Object.values(entries)) {
          expect(typeof value).toBe("string");
          expect(value.length).toBeGreaterThan(0);
        }
      }
    }
  });

  it("interpolate named params without touching unknown ones", () => {
    setLocale("zh");
    expect(t("sessions.selectForDelete", { name: "A/B" })).toBe("选择删除 A/B");
    expect(t("composer.charCount", { count: 12 })).toBe("12 字符");
    expect(t("composer.charCount")).toContain("{count}");
  });

  it("switch languages at runtime and fall back to the key for unknown entries", () => {
    setLocale("zh");
    expect(t("common.cancel")).toBe("取消");
    setLocale("en");
    expect(getLocale()).toBe("en");
    expect(t("common.cancel")).toBe("Cancel");
    expect(t("nope.missing" as never)).toBe("nope.missing");
    setLocale("zh");
    expect(t("common.cancel")).toBe("取消");
  });

  it("use a deterministic default locale in non-browser tests", () => {
    expect(["zh", "en"]).toContain(getLocale());
  });
});
