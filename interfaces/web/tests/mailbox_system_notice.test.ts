import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const main = readFileSync(new URL("../src/main.tsx", import.meta.url), "utf8");
const styles = readFileSync(
  new URL("../src/styles.css", import.meta.url),
  "utf8",
);

describe("mailbox system notices", () => {
  it("treats mailbox status entries as machine-injected system entries", () => {
    expect(main).toContain('entry.kind === "mailbox_status"');
    expect(main).toContain("const isSystemTriggeredEntry");
    expect(main).toContain("system-triggered");
    expect(main).toContain("systemOnlyFrame");
  });

  it("keeps the small mail icon on the system card", () => {
    expect(main).toContain('className="mailbox-entry-badge"');
    expect(main).toContain("<Mail size={11} />");
  });

  it("uses the neutral system background and smaller type", () => {
    expect(styles).toMatch(
      /\.turn-user-content\.system-only \{[^}]*background: #cdd4da/,
    );
    expect(styles).toMatch(
      /:root\[data-theme="light"\] \.turn-user-content\.system-only \{[^}]*background: #e6eaee/,
    );
    expect(styles).toMatch(
      /\.turn-user-entry\.system-triggered \{[^}]*font-size: 12px/,
    );
  });

  it("renders a system-only frame without the user bubble background", () => {
    expect(main).toMatch(
      /turn-user-content\$\{systemOnlyFrame \? " system-only" : ""\}/,
    );
  });
});
