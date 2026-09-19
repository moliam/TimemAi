import { describe, expect, it } from "vitest";
import { endpointLabelForProfile, endpointDraftValid, endpointMatchesProfile, endpointNameForProfile, formatContextWindowTokens, isValidMaxLlmInputTokens, MODEL_CONTEXT_WINDOW_OPTIONS } from "../src/model_endpoints";

const endpoint = { id: "one", name: "Production", model: "gpt-4.1", api_protocol: "openai-compatible", response_protocol: "xml", base_url: "https://api.example/v1", max_llm_input_tokens: 100_000, max_llm_output_tokens: 10_000, stream: false, api_key_configured: true };

describe("shared model endpoints", () => {
  it("matches an endpoint to the complete active Session route", () => {
    expect(endpointMatchesProfile(endpoint, { ...endpoint })).toBe(true);
    expect(endpointMatchesProfile(endpoint, { ...endpoint, base_url: "https://other" })).toBe(false);
    expect(endpointMatchesProfile(endpoint, { ...endpoint, api_key_configured: false })).toBe(false);
    expect(endpointMatchesProfile(endpoint, { ...endpoint, max_llm_input_tokens: 200_000 })).toBe(false);
    expect(endpointMatchesProfile(endpoint, { ...endpoint, max_llm_output_tokens: 20_000 })).toBe(false);
    expect(endpointMatchesProfile(endpoint, { ...endpoint, stream: true })).toBe(false);
  });

  it("resolves only the shared endpoint name for a Session profile", () => {
    expect(endpointNameForProfile([endpoint], { ...endpoint })).toBe("Production");
    expect(endpointNameForProfile([endpoint], { ...endpoint, model: "other-model" })).toBeUndefined();
    expect(endpointNameForProfile([], { ...endpoint })).toBeUndefined();
  });

  it("requires every route field while allowing an empty key", () => {
    expect(endpointDraftValid({ name: "Local", model: "qwen", api_protocol: "openai-compatible", response_protocol: "xml", base_url: "http://localhost:8000/v1", max_llm_input_tokens: 1_000_000, max_llm_output_tokens: 50_000, stream: true, api_key: "" })).toBe(true);
    expect(endpointDraftValid({ name: "", model: "qwen", api_protocol: "openai-compatible", response_protocol: "xml", base_url: "http://localhost", max_llm_input_tokens: 100_000, max_llm_output_tokens: 10_000, stream: false })).toBe(false);
    expect(endpointDraftValid({ name: "Custom input window", model: "qwen", api_protocol: "openai-compatible", response_protocol: "xml", base_url: "http://localhost", max_llm_input_tokens: 128_000, max_llm_output_tokens: 10_000, stream: false })).toBe(true);
    expect(endpointDraftValid({ name: "Invalid", model: "qwen", api_protocol: "openai-compatible", response_protocol: "xml", base_url: "http://localhost", max_llm_input_tokens: 0, max_llm_output_tokens: 8_000, stream: false })).toBe(false);
    expect(endpointDraftValid({ name: "Invalid", model: "qwen", api_protocol: "openai-compatible", response_protocol: "xml", base_url: "http://localhost", max_llm_input_tokens: 1.5, max_llm_output_tokens: 8_000, stream: false })).toBe(false);
  });
});

describe("context window options", () => {
  it("keeps preset options and adds 300K", () => {
    expect([...MODEL_CONTEXT_WINDOW_OPTIONS]).toEqual([100_000, 200_000, 300_000, 1_000_000]);
  });

  it("accepts any positive integer token count up to the u32 ceiling", () => {
    expect(isValidMaxLlmInputTokens(1)).toBe(true);
    expect(isValidMaxLlmInputTokens(128_000)).toBe(true);
    expect(isValidMaxLlmInputTokens(4_294_967_295)).toBe(true);
    expect(isValidMaxLlmInputTokens(0)).toBe(false);
    expect(isValidMaxLlmInputTokens(-100)).toBe(false);
    expect(isValidMaxLlmInputTokens(4_294_967_296)).toBe(false);
    expect(isValidMaxLlmInputTokens(Number.NaN)).toBe(false);
  });

  it("formats presets and custom values compactly", () => {
    expect(formatContextWindowTokens(100_000)).toBe("100K");
    expect(formatContextWindowTokens(300_000)).toBe("300K");
    expect(formatContextWindowTokens(1_000_000)).toBe("1M");
    expect(formatContextWindowTokens(2_000_000)).toBe("2M");
    expect(formatContextWindowTokens(131_072)).toBe("131.072K");
    expect(formatContextWindowTokens(500)).toBe("500");
  });
});

describe("Session endpoint labels", () => {
  it("keeps the saved name for a matching profile", () => {
    expect(endpointLabelForProfile([endpoint], endpoint)).toBe("Production");
  });

  it("shows a custom route when the Session retains a different URL", () => {
    const profile = { ...endpoint, base_url: "https://retained.example/v1" };
    expect(endpointLabelForProfile([endpoint], profile)).toBe("自定义配置 · gpt-4.1");
    expect(endpointMatchesProfile(endpoint, profile)).toBe(false);
  });

  it("does not lose the Session configuration when a preset changes or disappears", () => {
    expect(endpointLabelForProfile([], endpoint)).toBe("自定义配置 · gpt-4.1");
    expect(endpointLabelForProfile([endpoint], { ...endpoint, stream: true }))
      .toBe("自定义配置 · gpt-4.1");
  });

  it("reserves unconfigured for missing or empty model profiles", () => {
    expect(endpointLabelForProfile([endpoint], undefined)).toBe("未配置");
    expect(endpointLabelForProfile([], { ...endpoint, model: "  " })).toBe("未配置");
  });
});

describe("stable endpoint binding", () => {
  it("uses the bound ID despite route edits and renames", () => {
    const profile = { ...endpoint, model_endpoint_id: endpoint.id, base_url: "https://old.example/v1" };
    expect(endpointMatchesProfile(endpoint, profile)).toBe(true);
    expect(endpointLabelForProfile([{ ...endpoint, name: "Renamed" }], profile)).toBe("Renamed");
    expect(endpointMatchesProfile({ ...endpoint, id: "other" }, profile)).toBe(false);
  });
});

it("does not silently fall back when the bound endpoint is deleted", () => {
  const profile = { ...endpoint, model_endpoint_id: "deleted" };
  expect(endpointLabelForProfile([endpoint], profile)).toBe("接入点已删除");
  expect(endpointMatchesProfile(endpoint, profile)).toBe(false);
});
