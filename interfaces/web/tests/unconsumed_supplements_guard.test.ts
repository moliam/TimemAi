import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

// Guard for the unconsumed_supplements display suppression in
// activityFromTurnEvent (main.tsx): the worker re-submits pending supplements
// as a new turn right after TurnFinished, so broadcasting the raw payload
// would only be transient technical noise. The branch must keep suppressing
// the event ahead of the generic payload rendering path.
const mainSource = readFileSync(new URL("../src/main.tsx", import.meta.url), "utf8");

describe("unconsumed supplements suppression guard", () => {
  it("drops unconsumed_supplements events with an early return before generic rendering", () => {
    const marker = 'if (kind === "unconsumed_supplements") {';
    const occurrences = mainSource.split(marker).length - 1;
    expect(occurrences).toBe(1);
    const branchStart = mainSource.indexOf(marker);
    const window = mainSource.slice(branchStart, branchStart + 400);
    expect(window).toContain("return null;");
    const genericStart = mainSource.indexOf("const detail = Object.entries(event.payload)");
    expect(genericStart).toBeGreaterThan(branchStart);
  });

  it("anchors the suppression rationale to the worker resubmission behavior", () => {
    expect(mainSource).toContain(
      "The worker re-submits pending supplements as a new turn right after",
    );
    expect(mainSource).toContain(
      "TurnFinished; broadcasting the raw payload is transient technical noise.",
    );
  });
});
