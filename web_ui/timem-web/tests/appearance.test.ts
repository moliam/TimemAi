import { describe, expect, it } from "vitest";
import { defaultAppearance, parseAppearance } from "../src/appearance";

describe("web appearance preferences", () => {
  it("uses the operating-system theme for a new browser profile", () => {
    expect(defaultAppearance(true)).toEqual({ theme: "light", font: "system", textSize: "medium" });
    expect(defaultAppearance(false).theme).toBe("dark");
  });

  it("restores valid persisted choices", () => {
    expect(parseAppearance('{"theme":"light","font":"serif","textSize":"large"}', false)).toEqual({
      theme: "light",
      font: "serif",
      textSize: "large",
    });
  });

  it("bounds malformed and unknown persisted values", () => {
    expect(parseAppearance("not-json", false)).toEqual({ theme: "dark", font: "system", textSize: "medium" });
    expect(parseAppearance('{"theme":"neon","font":"comic","textSize":"huge"}', true)).toEqual({ theme: "light", font: "system", textSize: "medium" });
  });
});
