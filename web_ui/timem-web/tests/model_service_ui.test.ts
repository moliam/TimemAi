import { describe, expect, it } from "vitest";
import {
  commandSessionId,
  NO_MODEL_ENDPOINTS_ISSUE,
  UNCONFIGURED_MODEL_LABEL,
  isModelSubmissionCommand,
  modelDisplayName,
  modelServiceIssue,
  sessionModelConfigurationIssue,
} from "../src/model_service_ui";

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
    expect(modelDisplayName(undefined)).toBe(UNCONFIGURED_MODEL_LABEL);

    expect(modelDisplayName(
      { runtime_profile: { model: "  ", api_key_configured: true } as never },
    )).toBe("未配置");
  });
});

describe("shared endpoint availability", () => {
  it("uses a dedicated issue instead of claiming an API key is required", () => {
    expect(NO_MODEL_ENDPOINTS_ISSUE).toEqual({
      title: "没有接入点可用",
      detail: "请先新增并配置一个模型接入点，再发送消息。",
    });
  });
});

describe("Session model configuration issue", () => {
  it("explains missing model configuration without requiring an API key", () => {
    expect(sessionModelConfigurationIssue(undefined)).toEqual({
      title: "Model not configured",
      detail: "Open Runtime settings and configure a model and Base URL before sending a message.",
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
      title: "Endpoint authentication not configured",
      detail: "This endpoint has no API key. If the target service requires authentication, edit the endpoint and add one; otherwise verify the service response.",
    });
  });

  it.each([
    ["HTTP 401 unauthorized", "Model authentication failed"],
    ["HTTP 403 forbidden", "Model authentication failed"],
    ["HTTP 404 model not found", "Model unavailable"],
    ["connection refused", "Model service unavailable"],
    ["request timed out", "Model service unavailable"],
  ])("maps %s to %s", (error, title) => {
    expect(modelServiceIssue(error).title).toBe(title);
  });

  it("preserves an unknown useful reason while redacting credentials", () => {
    const issue = modelServiceIssue(
      "provider rejected request; Authorization: Bearer secret-token; api_key=sk-supersecret123",
    );
    expect(issue.title).toBe("Model request failed");
    expect(issue.detail).toContain("provider rejected request");
    expect(issue.detail).not.toContain("secret-token");
    expect(issue.detail).not.toContain("sk-supersecret123");
    expect(issue.detail).toContain("[redacted]");
  });

  it("provides a fallback when no usable service reason exists", () => {
    expect(modelServiceIssue(undefined)).toEqual({
      title: "Model request failed",
      detail: "The model service did not provide a usable reason. Check Runtime settings and retry.",
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
