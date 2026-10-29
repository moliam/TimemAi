import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { en } from "../src/i18n/strings.en";
import { zh } from "../src/i18n/strings.zh";

const component = readFileSync(
  new URL("../src/mailbox_body.tsx", import.meta.url),
  "utf8",
);
const source = readFileSync(new URL("../src/main.tsx", import.meta.url), "utf8");
const styles = readFileSync(
  new URL("../src/styles.css", import.meta.url),
  "utf8",
);

describe("mailbox body collapse", () => {
  it("clamps a mail body to five rendered lines", () => {
    expect(styles).toMatch(
      /\.mailbox-item-body\.clamped \{[^}]*-webkit-line-clamp: 5;/,
    );
    expect(styles).toMatch(
      /\.mailbox-item-body\.clamped \{[^}]*max-height: calc\(5 \* 1\.5em\);/,
    );
  });

  it("shows the toggle only when the rendered body overflows the clamp", () => {
    expect(component).toContain("element.scrollHeight > element.clientHeight + 1");
    expect(component).toContain('typeof ResizeObserver === "undefined"');
    expect(component).toContain("const showToggle = overflowing || expanded;");
    expect(component).toContain(
      'className={`mailbox-item-body${expanded ? "" : " clamped"}`}',
    );
    expect(component).toMatch(
      /showToggle && \([\s\S]*className="mailbox-item-body-toggle"/,
    );
  });

  it("expands and collapses accessibly with localized labels", () => {
    expect(component).toContain("aria-expanded={expanded}");
    expect(component).toContain("aria-controls={bodyId}");
    expect(component).toContain('t("mailbox.expandBody")');
    expect(component).toContain('t("mailbox.collapseBody")');
    expect(zh.mailbox.expandBody.length).toBeGreaterThan(0);
    expect(en.mailbox.expandBody.length).toBeGreaterThan(0);
    expect(zh.mailbox.collapseBody.length).toBeGreaterThan(0);
    expect(en.mailbox.collapseBody.length).toBeGreaterThan(0);
  });

  it("routes inbox and sent bodies through the clamped component", () => {
    expect(source).toContain("<MailboxBody text={mail.msg} />");
    expect(source).toContain("<MailboxBody text={entry.msg} />");
    expect(source).not.toContain('className="mailbox-item-body">{mail.msg}');
    expect(source).not.toContain('className="mailbox-item-body">{entry.msg}');
  });
});
