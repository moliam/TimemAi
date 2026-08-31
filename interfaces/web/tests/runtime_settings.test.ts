import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import {
  reconcileRuntimeDrafts,
  runtimeOptionLabel,
  sessionRuntimeOptions,
  shouldAutoRevealSessionApiKey,
  updateRevealedSessionApiKeys,
} from "../src/runtime_settings";

const mainSource = readFileSync(new URL("../src/main.tsx", import.meta.url), "utf8");

describe("runtime setting labels", () => {
  it("hides the Timem namespace prefix without changing other keys", () => {
    expect(runtimeOptionLabel("TIMEM_MODEL")).toBe("MODEL");
    expect(runtimeOptionLabel("TIMEM_BASE_URL")).toBe("BASE_URL");
    expect(runtimeOptionLabel("TIMEM_MAX_ROUNDS")).toBe("MAX STEPS");
    expect(runtimeOptionLabel("CUSTOM_OPTION")).toBe("CUSTOM_OPTION");
  });
});

describe("runtime setting drafts", () => {
  it("removes only the field acknowledged by the runtime", () => {
    const drafts = { TIMEM_MODEL: "model-b", TIMEM_BASE_URL: "https://new.example/v1" };
    expect(reconcileRuntimeDrafts(drafts, [
      { key: "TIMEM_MODEL", value: "model-b" },
      { key: "TIMEM_BASE_URL", value: "https://old.example/v1" },
    ])).toEqual({ TIMEM_BASE_URL: "https://new.example/v1" });
  });

  it("preserves every unsaved draft after an unrelated snapshot update", () => {
    const drafts = { TIMEM_MODEL: "model-b", TIMEM_BASE_URL: "https://new.example/v1" };
    expect(reconcileRuntimeDrafts(drafts, [
      { key: "TIMEM_MODEL", value: "model-a" },
      { key: "TIMEM_BASE_URL", value: "https://old.example/v1" },
    ])).toBe(drafts);
  });
});

describe("session runtime settings", () => {
  it("renders values from the selected Session instead of shared host defaults", () => {
    const options = sessionRuntimeOptions({
      model: "session-model",
      api_protocol: "anthropic",
      base_url: "https://session.example/v1",
      max_llm_input_tokens: 64000,
      max_llm_output_tokens: 8000,
      max_rounds: "unlimited",
      bash_approval: "ask",
      work_instructions: "silent",
    }, [
      { key: "TIMEM_MODEL", value: "host-model" },
      { key: "TIMEM_BASE_URL", value: "https://host.example/v1" },
      { key: "TIMEM_MAX_ROUNDS", value: "50" },
    ]);
    expect(options).toEqual([
      { key: "TIMEM_MODEL", value: "session-model" },
      { key: "TIMEM_BASE_URL", value: "https://session.example/v1" },
      { key: "TIMEM_MAX_ROUNDS", value: "unlimited" },
    ]);
  });
});

describe("session API key presentation", () => {
  it("loads an existing key once so the password input can render its masked value", () => {
    const base = {
      sessionId: "session-1",
      configured: true,
      revealedApiKey: undefined,
      pending: false,
      requestedSessionId: "",
    };
    expect(shouldAutoRevealSessionApiKey(base)).toBe(true);
    expect(shouldAutoRevealSessionApiKey({ ...base, requestedSessionId: "session-1" })).toBe(false);
    expect(shouldAutoRevealSessionApiKey({ ...base, pending: true })).toBe(false);
    expect(shouldAutoRevealSessionApiKey({ ...base, revealedApiKey: "secret" })).toBe(false);
    expect(shouldAutoRevealSessionApiKey({ ...base, configured: false })).toBe(false);
  });

  it("uses the acknowledged saved value as the new masked baseline", () => {
    const current = { "session-1": "old-secret", "session-2": "other-secret" };
    expect(updateRevealedSessionApiKeys(current, "session-1", "new-secret")).toEqual({
      "session-1": "new-secret",
      "session-2": "other-secret",
    });
    expect(updateRevealedSessionApiKeys(current, "session-1")).toEqual({ "session-2": "other-secret" });
  });
});


describe("active session runtime controls", () => {
  it("keeps working API keys revealable and copyable but prevents credential changes", () => {
    expect(mainSource).toMatch(/disabled=\{!session \|\| credentialPending\}\s+readOnly=\{sessionWorking\}/);
    expect(mainSource).toMatch(/disabled=\{!session \|\| credentialPending\}\s+onClick=\{toggleApiKey\}/);
    expect(mainSource).toMatch(/const canSaveApiKey =\s*!!session && apiKeyDirty && !credentialPending && !sessionWorking;/);
    expect(mainSource).not.toContain('disabled={!session || credentialPending || sessionWorking}');
    expect(mainSource).toContain('disabled={pending}');
    expect(mainSource).toContain('disabled={pending || !dirty}');
    expect(mainSource).not.toContain('disabled={pending || sessionWorking}');
    expect(mainSource).not.toContain('disabled={pending || !dirty || sessionWorking}');
    expect(mainSource).not.toContain('dirty && !pending && !sessionWorking');
  });
});
