import { describe, expect, it } from "vitest";
import { endpointLabelForProfile, endpointDraftValid, endpointMatchesProfile, endpointNameForProfile } from "../src/model_endpoints";

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
    expect(endpointDraftValid({ name: "Invalid", model: "qwen", api_protocol: "openai-compatible", response_protocol: "xml", base_url: "http://localhost", max_llm_input_tokens: 128_000, max_llm_output_tokens: 8_000, stream: false })).toBe(false);
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
