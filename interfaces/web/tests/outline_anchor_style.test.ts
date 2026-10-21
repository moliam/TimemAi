import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

// Source contract: Start and Summary are delivery anchors, not text chapters.
// Both rows must carry a persistent tinted fill in every theme so the outline
// reads anchor rows vs plain chapter rows at a glance.
const styles = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8");

describe("outline anchor row styling", () => {
  it("tints the Start and Summary anchor rows in the dark theme", () => {
    expect(styles).toMatch(
      /\.final-answer-outline-card nav > button\.final-answer-outline-start, \.final-answer-outline-card nav > button\.outline-summary \{ background: linear-gradient\(90deg, #75b8a318, #75b8a30d\); box-shadow: inset 0 0 0 1px #75b8a312; \}/,
    );
  });

  it("tints the same anchor rows in the light theme", () => {
    expect(styles).toMatch(
      /:root\[data-theme="light"\] \.final-answer-outline-card nav > button\.final-answer-outline-start, :root\[data-theme="light"\] \.final-answer-outline-card nav > button\.outline-summary \{ background: linear-gradient\(90deg, #287e6816, #287e6809\); box-shadow: inset 0 0 0 1px #287e6812; \}/,
    );
  });

  it("keeps plain chapter rows transparent so anchors stay distinct", () => {
    expect(styles).toMatch(
      /\.final-answer-outline-card nav > button \{[^}]*background: transparent;/,
    );
  });
});
