import { describe, expect, it } from "vitest";
import { applyAppearance, defaultAppearance, parseAppearance } from "../src/appearance";

describe("web appearance preferences", () => {
  it("uses the operating-system theme for a new browser profile", () => {
    expect(defaultAppearance(true)).toEqual({
      theme: "light",
      userFont: "system",
      userChineseFont: "system",
      userBold: true,
      agentFont: "system",
      agentChineseFont: "system",
      agentBold: false,
      textSize: "medium",
    });
    expect(defaultAppearance(false).theme).toBe("dark");
  });

  it("restores valid persisted choices", () => {
    expect(parseAppearance('{"theme":"light","userFont":"serif","userChineseFont":"kaiti","userBold":false,"agentFont":"mono","agentChineseFont":"songti","agentBold":true,"textSize":"large"}', false)).toEqual({
      theme: "light",
      userFont: "serif",
      userChineseFont: "kaiti",
      userBold: false,
      agentFont: "mono",
      agentChineseFont: "songti",
      agentBold: true,
      textSize: "large",
    });
  });

  it("migrates old generic Chinese font choices to named typefaces", () => {
    expect(parseAppearance('{"font":"mono","chineseFont":"sans"}', false)).toMatchObject({ userFont: "mono", agentFont: "mono", userChineseFont: "heiti", agentChineseFont: "heiti" });
    expect(parseAppearance('{"chineseFont":"serif"}', false)).toMatchObject({ userChineseFont: "songti", agentChineseFont: "songti" });
  });

  it("keeps old saved appearance preferences and defaults the new Chinese font", () => {
    expect(parseAppearance('{"theme":"light","font":"mono","textSize":"small"}', false)).toEqual({
      theme: "light",
      userFont: "mono",
      userChineseFont: "system",
      userBold: true,
      agentFont: "mono",
      agentChineseFont: "system",
      agentBold: false,
      textSize: "small",
    });
  });

  it("defaults a missing user bold preference to selected while preserving an explicit opt-out", () => {
    expect(parseAppearance('{"theme":"dark","userFont":"system"}', false).userBold).toBe(true);
    expect(parseAppearance('{"theme":"dark","userBold":false}', false).userBold).toBe(false);
  });

  it("bounds malformed and unknown persisted values", () => {
    expect(parseAppearance("not-json", false)).toEqual(defaultAppearance(false));
    expect(parseAppearance('{"theme":"neon","font":"comic","chineseFont":"handwriting","textSize":"huge"}', true)).toEqual(defaultAppearance(true));
  });

  it("keeps native controls aligned with the selected theme", () => {
    expect(applyAppearance.toString()).toContain('root.dataset.theme = appearance.theme;');
  });
});
