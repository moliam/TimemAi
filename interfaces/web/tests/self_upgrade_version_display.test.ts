import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { zh } from "../src/i18n/strings.zh";
import { en } from "../src/i18n/strings.en";

const source = readFileSync(new URL("../src/self_upgrade.tsx", import.meta.url), "utf8");

// Remote release tags arrive with a leading "v" while Host-side current
// versions arrive bare; the UI normalizes at the display boundary so
// templates that render "v{...}" never produce a doubled prefix ("vv3.0.1").
describe("self upgrade version display", () => {
  it("normalizes remote and pending versions to bare form before interpolation", () => {
    expect(source).toContain("function bareVersion(");
    expect(source).toContain("next: bareVersion(phase.remote_version)");
    expect(source).toContain(
      't("system.upgradeDownloading", { version: bareVersion(phase.remote_version) })',
    );
    expect(source).toContain(
      't("system.upgradePendingRestart", { version: bareVersion(phase.pending_version) })',
    );
    expect(source).not.toContain("next: phase.remote_version");
  });

  it("keeps exactly one template-owned v prefix for every version slot", () => {
    expect(zh.system.upgradeAvailable).toContain("v{next}");
    expect(zh.system.upgradeAvailable).toContain("v{current}");
    expect(en.system.upgradeAvailable).toContain("v{next}");
    expect(zh.system.upgradeDownloading).toContain("v{version}");
    expect(en.system.upgradeDownloading).toContain("v{version}");
    expect(zh.system.upgradePendingRestart).toContain("v{version}");
    expect(en.system.upgradePendingRestart).toContain("v{version}");
  });
});
