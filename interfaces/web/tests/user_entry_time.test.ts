import { describe, expect, it } from "vitest";
import { formatUserEntryTime } from "../src/user_entry_time";

describe("formatUserEntryTime", () => {
  const now = new Date(2026, 9, 18, 12, 0).getTime();

  it("renders only hour:minute for entries from today", () => {
    const value = new Date(2026, 9, 18, 9, 5).getTime();
    const rendered = formatUserEntryTime(value, now);
    expect(rendered.length).toBeGreaterThan(0);
    expect(rendered).toContain(":");
    expect(rendered).not.toContain("10");
  });

  it("adds month/day for same-year entries outside today", () => {
    const value = new Date(2026, 2, 3, 9, 5).getTime();
    const rendered = formatUserEntryTime(value, now);
    expect(rendered).toContain(":");
    expect(rendered).toContain("3");
    expect(rendered).not.toContain("2026");
  });

  it("adds the year for entries from a different year", () => {
    const value = new Date(2025, 2, 3, 9, 5).getTime();
    const rendered = formatUserEntryTime(value, now);
    expect(rendered).toContain(":");
    expect(rendered).toContain("2025");
  });

  it("returns an empty string for invalid timestamps", () => {
    expect(formatUserEntryTime(Number.NaN, now)).toBe("");
  });
});
