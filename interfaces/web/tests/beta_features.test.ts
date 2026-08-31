import { describe, expect, it } from "vitest";
import { parseToolGenEnabled, saveToolGenEnabled, TOOLGEN_ENABLED_STORAGE_KEY } from "../src/beta_features";

describe("web beta feature preferences", () => {
  it("keeps ToolGen disabled by default", () => {
    expect(parseToolGenEnabled(null)).toBe(false);
    expect(parseToolGenEnabled("")).toBe(false);
    expect(parseToolGenEnabled("false")).toBe(false);
  });

  it("restores only an explicit true value", () => {
    expect(parseToolGenEnabled("true")).toBe(true);
    expect(parseToolGenEnabled('"true"')).toBe(false);
    expect(parseToolGenEnabled("1")).toBe(false);
    expect(parseToolGenEnabled("not-json")).toBe(false);
  });

  it("persists the browser-local switch under a dedicated key", () => {
    expect(TOOLGEN_ENABLED_STORAGE_KEY).toBe("timem-web-toolgen-enabled-v1");
    expect(saveToolGenEnabled.toString()).toContain("window.localStorage.setItem");
  });
});
