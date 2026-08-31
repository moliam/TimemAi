import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const mainSource = readFileSync(
  new URL("../src/main.tsx", import.meta.url),
  "utf8",
);

describe("MEM switch execution boundary", () => {
  it("warns that confirmed switching stops running and queued work without restart", () => {
    expect(mainSource).toContain(
      "Switching MEM will stop all running and queued work in the current",
    );
    expect(mainSource).toContain(
      "Nothing from the current MEM will continue or restart",
    );
    expect(mainSource).toMatch(
      /affected Session[\s\S]*will be marked[\s\S]*interrupted/,
    );
  });

  it("offers a separate Runtime instance as the non-destructive alternative", () => {
    expect(mainSource).toContain(
      "To keep the current work running, start a separate instance for the",
    );
    expect(mainSource).toContain("timem --space");
  });
});
