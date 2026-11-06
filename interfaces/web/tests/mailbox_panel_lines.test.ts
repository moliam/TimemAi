import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { en } from "../src/i18n/strings.en";
import { zh } from "../src/i18n/strings.zh";

const source = readFileSync(new URL("../src/main.tsx", import.meta.url), "utf8");
const styles = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8");

describe("mailbox panel one-line lists", () => {
  it("previews the newest eight mails per section and folds the rest", () => {
    expect(source).toContain("const MAILBOX_PREVIEW_COUNT = 8;");
    expect(source).toContain("const recentMails = inboxMails.slice(0, MAILBOX_PREVIEW_COUNT);");
    expect(source).toContain("const olderMails = inboxMails.slice(MAILBOX_PREVIEW_COUNT);");
    expect(source).toContain("const recentSent = sentMails.slice(0, MAILBOX_PREVIEW_COUNT);");
    expect(source).toContain("const olderSent = sentMails.slice(MAILBOX_PREVIEW_COUNT);");
    expect(source).toContain('{t("mailbox.more")}');
    expect(zh.mailbox.more).toBe("更多…");
    expect(en.mailbox.more).toBe("More…");
  });

  it("renders inbox and sent as expandable one-line rows routing through MailboxBody", () => {
    expect(source).toContain('className={`mailbox-item-line${mail.read ? " read" : ""}${mail.kind === "receipt" ? " receipt" : ""}`}');
    expect(source).toContain('className="mailbox-item-preview"');
    expect(source).toContain("<MailboxBody text={mail.msg} />");
    expect(source).toContain("<MailboxBody text={entry.msg} />");
  });

  it("retires the read-only inbox and the archive fold", () => {
    expect(source).not.toContain("unreadMails");
    expect(source).not.toContain("archivedMails");
    expect(source).not.toContain('className="mailbox-archive"');
    expect(source).not.toContain('t("mailbox.archiveTitle")');
    expect(zh.mailbox.archiveTitle).toBeUndefined();
    expect(en.mailbox.archiveTitle).toBeUndefined();
  });

  it("styles the line rows and the more fold for both themes", () => {
    expect(styles).toMatch(/\.mailbox-item-line \{ border: 1px solid #243040/);
    expect(styles).toMatch(/\.mailbox-item-preview \{ flex: 1 1 auto; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;/);
    expect(styles).toMatch(/\.mailbox-more > summary \{[^}]*font-size: 11px;/);
    expect(styles).toMatch(/:root\[data-theme="light"\] \.mailbox-item-line \{ border-color: #dde5ec; background: #fff; \}/);
    expect(styles).toMatch(/:root\[data-theme="light"\] \.mailbox-more > summary \{ border-color: #c6d2db;/);
  });
});
