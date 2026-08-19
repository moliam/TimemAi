import { describe, expect, it } from "vitest";
import {
  commandSessionId,
  UNCONFIGURED_MODEL_LABEL,
  isModelSubmissionCommand,
  modelDisplayName,
  modelServiceIssue,
  sessionModelConfigurationIssue,
} from "../src/model_service_ui";

describe("model display name", () => {
  it("shows the actual model only when the Session model service is configured", () => {
    expect(modelDisplayName(
      { runtime_profile: { model: "session-model", api_key_configured: true } as never },
    )).toBe("session-model");
  });

  it("shows unconfigured instead of a host or protocol default", () => {
    expect(modelDisplayName(undefined)).toBe(UNCONFIGURED_MODEL_LABEL);

    expect(modelDisplayName(
      { runtime_profile: { model: "qwen-plus", api_key_configured: false } as never },
    )).toBe("未配置");

    expect(modelDisplayName(
      { runtime_profile: { model: "  ", api_key_configured: true } as never },
    )).toBe("未配置");
  });
});

describe("Session model configuration issue", () => {
  it("explains missing model configuration and missing API keys", () => {
    expect(sessionModelConfigurationIssue(undefined)).toEqual({
      title: "Model not configured",
      detail: "Open Runtime settings, configure a model and Base URL, then save a Session API key before sending a message.",
    });
    expect(sessionModelConfigurationIssue({
      runtime_profile: { model: "qwen-plus", api_key_configured: false } as never,
    })).toEqual({
      title: "API key required",
      detail: "Open Runtime settings, enter the Session API key, and save it before sending a message.",
    });
  });

  it("does not show a configuration warning when model and key are configured", () => {
    expect(sessionModelConfigurationIssue({
      runtime_profile: { model: "configured-model", api_key_configured: true } as never,
    })).toBeNull();
  });
});

describe("model service issue presentation", () => {
  it("gives actionable guidance for a missing Session API key", () => {
    expect(modelServiceIssue(
      "session_model_service_config_incomplete:missing_api_key",
    )).toEqual({
      title: "API key required",
      detail: "Open Runtime settings, enter the Session API key, and save it before sending another message.",
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
