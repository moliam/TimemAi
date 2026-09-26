import { beforeAll, describe, expect, it } from "vitest";
import { setLocale } from "../src/i18n";
import { zh } from "../src/i18n/strings.zh";
import {
  commandSessionId,
  isModelSubmissionCommand,
  modelDisplayName,
  modelServiceIssue,
  noModelEndpointsIssue,
  sessionModelConfigurationIssue,
  unconfiguredModelLabel,
} from "../src/model_service_ui";

// Catalog-copy assertions run against the zh source catalog for determinism;
// key parity with en is enforced by i18n.test.ts and the Strings type.
beforeAll(() => {
  setLocale("zh");
});

describe("model display name", () => {
  it("shows the configured model whether or not the endpoint uses an API key", () => {
    expect(modelDisplayName(
      { runtime_profile: { model: "session-model", api_key_configured: true } as never },
    )).toBe("session-model");
    expect(modelDisplayName(
      { runtime_profile: { model: "local-model", api_key_configured: false } as never },
    )).toBe("local-model");
  });

  it("shows unconfigured instead of a host or protocol default", () => {
    expect(modelDisplayName(undefined)).toBe(unconfiguredModelLabel());
    expect(unconfiguredModelLabel()).toBe("未配置");

    expect(modelDisplayName(
      { runtime_profile: { model: "  ", api_key_configured: true } as never },
    )).toBe("未配置");
  });
});

describe("shared endpoint availability", () => {
  it("uses a dedicated issue instead of claiming an API key is required", () => {
    expect(noModelEndpointsIssue()).toEqual({
      title: zh.modelService.noEndpointsTitle,
      detail: zh.modelService.noEndpointsDetail,
    });
  });
});

describe("Session model configuration issue", () => {
  it("explains missing model configuration without requiring an API key", () => {
    expect(sessionModelConfigurationIssue(undefined)).toEqual({
      title: zh.modelService.sessionNotConfiguredTitle,
      detail: zh.modelService.sessionNotConfiguredDetail,
    });
    expect(sessionModelConfigurationIssue({
      runtime_profile: { model: "qwen-plus", api_key_configured: false } as never,
    })).toBeNull();
  });

  it("does not show a configuration warning when a keyed model is configured", () => {
    expect(sessionModelConfigurationIssue({
      runtime_profile: { model: "configured-model", api_key_configured: true } as never,
    })).toBeNull();
  });
});

describe("model service issue presentation", () => {
  it("treats a missing API key as endpoint-specific rather than globally required", () => {
    expect(modelServiceIssue(
      "session_model_service_config_incomplete:missing_api_key",
    )).toEqual({
      title: zh.service.authTitle,
      detail: zh.service.authDetail,
    });
  });

  it.each([
    ["HTTP 401 unauthorized", zh.service.authFailedTitle],
    ["HTTP 403 forbidden", zh.service.authFailedTitle],
    ["HTTP 404 model not found", zh.service.unavailableTitle],
    ["connection refused", zh.service.unreachableTitle],
    ["request timed out", zh.service.unreachableTitle],
  ])("maps %s to %s", (error, title) => {
    expect(modelServiceIssue(error).title).toBe(title);
  });

  it("shows the provider cache_control rejection instead of hiding it behind an HTTP status", () => {
    const cacheIssue = modelServiceIssue(
      "model_http_404: A maximum of 4 blocks with cache_control may be provided. Found 5.",
    );
    expect(cacheIssue.title).toBe(zh.service.cacheTitle);
    expect(cacheIssue.detail).toContain("A maximum of 4 blocks with cache_control may be provided. Found 5.");
    expect(cacheIssue.detail).toContain(zh.service.cacheDetail);
  });

  it("keeps a useful provider reason when applying HTTP status guidance", () => {
    const issue = modelServiceIssue("model_http_404: deployment blue is temporarily unavailable");
    expect(issue.title).toBe(zh.service.unavailableTitle);
    expect(issue.detail).toContain("deployment blue is temporarily unavailable");
    expect(issue.detail).toContain(zh.service.unavailableDetail);
  });

  it("preserves an unknown useful reason while redacting credentials", () => {
    const issue = modelServiceIssue(
      "provider rejected request; Authorization: Bearer secret-token; api_key=sk-supersecret123",
    );
    expect(issue.title).toBe(zh.service.failedTitle);
    expect(issue.detail).toContain("provider rejected request");
    expect(issue.detail).not.toContain("secret-token");
    expect(issue.detail).not.toContain("sk-supersecret123");
    expect(issue.detail).toContain("[redacted]");
  });

  it("provides a fallback when no usable service reason exists", () => {
    expect(modelServiceIssue(undefined)).toEqual({
      title: zh.service.failedTitle,
      detail: zh.service.failedDetailFallback,
    });
  });
});

describe("command Session attribution", () => {
  it("uses the rejected command's Session instead of the currently selected Session", () => {
    expect(commandSessionId({ session_id: "session-original" })).toBe("session-original");
    expect(commandSessionId({ session_id: "" })).toBeUndefined();
    expect(commandSessionId(undefined)).toBeUndefined();
    expect(commandSessionId("not-a-command")).toBeUndefined();
  });

  it("classifies only turn submissions and supplements as model-bound commands", () => {
    expect(isModelSubmissionCommand({ type: "turn_submit" })).toBe(true);
    expect(isModelSubmissionCommand({ type: "turn_supplement" })).toBe(true);
    expect(isModelSubmissionCommand({ type: "session_rename" })).toBe(false);
    expect(isModelSubmissionCommand(undefined)).toBe(false);
  });
});
