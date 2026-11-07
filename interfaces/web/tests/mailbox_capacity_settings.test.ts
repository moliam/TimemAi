import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const protocol = readFileSync(
  new URL("../src/protocol.ts", import.meta.url),
  "utf8",
);
const source = readFileSync(new URL("../src/main.tsx", import.meta.url), "utf8");

describe("mailbox capacity settings contract", () => {
  it("exposes mailbox capacity and actual usage on the wire", () => {
    expect(protocol).toMatch(
      /export type MemStorageUsage = \{[\s\S]*temporary_bytes: number;[\s\S]*conversation_bytes: number;[\s\S]*mailbox_bytes: number;[\s\S]*mailbox_files: number;[\s\S]*favorite_bytes: number;/,
    );
    expect(protocol).toContain('{ type: "mem_storage_usage" }');
    expect(protocol).toContain(
      '{ type: "mem_mailbox_capacity_update"; max_bytes: number | null }',
    );
    expect(protocol).toContain('{ type: "mem_storage_usage_query" }');
    expect(protocol).toMatch(/mailbox_capacity_bytes: number \| null;/);
  });

  it("saves the independent mailbox limit and adopts the confirmed event", () => {
    expect(source).toContain('type: "mem_mailbox_capacity_update"');
    expect(source).toContain("setPendingMemMailboxCapacity");
    expect(source).toContain(
      "mailbox_capacity_bytes: event.mailbox_capacity_bytes",
    );
    expect(source).toMatch(/if \(event\.type === "mem_storage_usage"\)/);
    expect(source).toContain("mailbox_files: event.mailbox_files");
  });

  it("shows actual usage for temporary data, conversations and mailbox", () => {
    expect(source).toContain('data-testid="temporary-storage-usage"');
    expect(source).toContain('data-testid="conversation-storage-usage"');
    expect(source).toContain('data-testid="mailbox-storage-usage"');
    expect(source).toContain("Conversations currently use");
    expect(source).toContain("conversations and Mailbox together use");
    expect(source).toContain(
      "across ${memStorageUsage.mailbox_files} body files",
    );
    expect(source).toContain("<strong>Mailbox capacity</strong>");
  });

  it("refreshes usage on page open and mailbox updates, with the backend tiers", () => {
    expect(source).toMatch(
      /setMemStorageUsage\(null\);\s*sendCommand\(\{ type: "mem_storage_usage_query" \}\);/,
    );
    expect(source).toMatch(
      /if \(memorySettingsOpenRef\.current\)\s*sendCommandRef\.current\(\{ type: "mem_storage_usage_query" \}\);/,
    );
    for (const tier of [32, 64, 128, 256, 512]) {
      expect(source).toContain(`<option value={${tier} * 1024 * 1024}>`);
    }
    expect(source).toContain("<option value={1024 * 1024 * 1024}>1 GB</option>");
    expect(source).toContain('<option value="unlimited">Unlimited</option>');
  });
});
