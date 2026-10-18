import { describe, expect, it } from "vitest";
import { formatUserEntryTime } from "../src/user_entry_time";

describe("formatUserEntryTime", () => {
  it("renders a local wall-clock hour:minute string", () => {
    const value = new Date(2026, 9, 18, 9, 5).getTime();
    const rendered = formatUserEntryTime(value);
    expect(rendered.length).toBeGreaterThan(0);
    expect(rendered).toContain(":");
  });

  it("returns an empty string for invalid timestamps", () => {
    expect(formatUserEntryTime(Number.NaN)).toBe("");
  });
});
