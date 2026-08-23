import { describe, expect, it } from "vitest";
import { endpointDraftValid, endpointMatchesProfile, endpointNameForProfile } from "../src/model_endpoints";

const endpoint = { id: "one", name: "Production", model: "gpt-4.1", api_protocol: "openai-compatible", response_protocol: "xml", base_url: "https://api.example/v1", api_key_configured: true };

describe("shared model endpoints", () => {
  it("matches an endpoint to the complete active Session route", () => {
    expect(endpointMatchesProfile(endpoint, { ...endpoint })).toBe(true);
    expect(endpointMatchesProfile(endpoint, { ...endpoint, base_url: "https://other" })).toBe(false);
    expect(endpointMatchesProfile(endpoint, { ...endpoint, api_key_configured: false })).toBe(false);
  });

  it("resolves only the shared endpoint name for a Session profile", () => {
    expect(endpointNameForProfile([endpoint], { ...endpoint })).toBe("Production");
    expect(endpointNameForProfile([endpoint], { ...endpoint, model: "other-model" })).toBeUndefined();
    expect(endpointNameForProfile([], { ...endpoint })).toBeUndefined();
  });

  it("requires every route field while allowing an empty key", () => {
    expect(endpointDraftValid({ name: "Local", model: "qwen", api_protocol: "openai-compatible", response_protocol: "xml", base_url: "http://localhost:8000/v1", api_key: "" })).toBe(true);
    expect(endpointDraftValid({ name: "", model: "qwen", api_protocol: "openai-compatible", response_protocol: "xml", base_url: "http://localhost" })).toBe(false);
  });
});
