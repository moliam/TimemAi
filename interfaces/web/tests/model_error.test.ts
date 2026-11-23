import { describe, expect, it } from "vitest";
import {
  classFromStatus,
  modelErrorReason,
  parseModelError,
} from "../src/model_error";

describe("canonical model error parsing", () => {
  it("reads the class, provider and business code from the header", () => {
    expect(
      parseModelError("model_http_429[billing,zhipu/1113]: 您的账户已欠费，请充值后重试"),
    ).toEqual({
      status: 429,
      class: "billing",
      provider: "zhipu",
      code: "1113",
    });
  });

  it("keeps a class-only header readable", () => {
    expect(parseModelError("model_http_503[overloaded]: upstream overloaded")).toEqual({
      status: 503,
      class: "overloaded",
    });
  });

  it("records a business code even when the provider is unknown", () => {
    expect(parseModelError("model_http_429[rate_limited,1113]: slow down")).toMatchObject({
      class: "rate_limited",
      code: "1113",
    });
  });

  it("classifies legacy strings by status only, never by message text", () => {
    expect(parseModelError("model_http_429: upstream overloaded")).toMatchObject({
      status: 429,
      class: "rate_limited",
    });
    expect(parseModelError("model_http_402: Insufficient Balance").class).toBe("billing");
    expect(parseModelError("model_http_404: model not found").class).toBe("model_not_found");
    expect(parseModelError("model_http_413: payload too large").class).toBe("input_too_large");
    expect(parseModelError("HTTP 401 unauthorized").class).toBe("unknown");
  });

  it("maps transport prefixes and unknown payloads", () => {
    expect(parseModelError("model_timeout: stage=response_headers").class).toBe("timeout");
    expect(parseModelError("model_network_error: connection reset").class).toBe("transport");
    expect(parseModelError("unexpected_retryable_failure").class).toBe("unknown");
    expect(parseModelError(undefined).class).toBe("unknown");
  });

  it("uses the class token of stream failures and falls back to the symbolic code", () => {
    expect(
      parseModelError(
        "model_responses_stream_failed: class=overloaded event_type=error code=server_is_overloaded message=overloaded",
      ).class,
    ).toBe("overloaded");
    expect(
      parseModelError(
        "model_responses_stream_failed: event_type=response.failed code=insufficient_quota message=quota",
      ).class,
    ).toBe("quota");
    expect(
      parseModelError(
        "model_responses_stream_failed: event_type=response.failed code=invalid_request_error message=bad tools",
      ).class,
    ).toBe("invalid_request");
    expect(parseModelError("model_responses_stream_failed").class).toBe("unknown");
  });

  it("mirrors the provider-neutral status table", () => {
    expect(classFromStatus(400)).toBe("invalid_request");
    expect(classFromStatus(401)).toBe("authentication");
    expect(classFromStatus(402)).toBe("billing");
    expect(classFromStatus(403)).toBe("permission");
    expect(classFromStatus(404)).toBe("model_not_found");
    expect(classFromStatus(408)).toBe("timeout");
    expect(classFromStatus(409)).toBe("server_error");
    expect(classFromStatus(413)).toBe("input_too_large");
    expect(classFromStatus(422)).toBe("invalid_request");
    expect(classFromStatus(429)).toBe("rate_limited");
    expect(classFromStatus(500)).toBe("server_error");
    expect(classFromStatus(503)).toBe("overloaded");
    expect(classFromStatus(504)).toBe("timeout");
    expect(classFromStatus(4290)).toBe("unknown");
  });

  it("strips the canonical header from the display reason only", () => {
    expect(modelErrorReason("model_http_429[billing,zhipu/1113]: 您的账户已欠费")).toBe(
      "您的账户已欠费",
    );
    expect(modelErrorReason("model_http_503: overloaded")).toBe("overloaded");
    expect(modelErrorReason("plain reason")).toBe("plain reason");
    expect(modelErrorReason(undefined)).toBe("");
  });
});
