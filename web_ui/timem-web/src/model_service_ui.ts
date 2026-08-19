import { Session } from "./protocol";

export const UNCONFIGURED_MODEL_LABEL = "未配置";


export type ModelServiceIssue = {
  title: string;
  detail: string;
};

export function modelDisplayName(
  session: Pick<Session, "runtime_profile"> | undefined,
): string {
  const profile = session?.runtime_profile;
  if (!profile?.api_key_configured) return UNCONFIGURED_MODEL_LABEL;
  return profile.model.trim() || UNCONFIGURED_MODEL_LABEL;
}


export function sessionModelConfigurationIssue(
  session: Pick<Session, "runtime_profile"> | undefined,
): ModelServiceIssue | null {
  const profile = session?.runtime_profile;
  if (!profile || !profile.model.trim()) {
    return {
      title: "Model not configured",
      detail: "Open Runtime settings, configure a model and Base URL, then save a Session API key before sending a message.",
    };
  }
  if (!profile.api_key_configured) {
    return {
      title: "API key required",
      detail: "Open Runtime settings, enter the Session API key, and save it before sending a message.",
    };
  }
  return null;
}

function sanitizeModelServiceError(rawError: string): string {
  return rawError
    .replace(/(authorization\s*[:=]\s*bearer\s+)[^\s,;]+/gi, "$1[redacted]")
    .replace(/(bearer\s+)[A-Za-z0-9._~+/=-]+/gi, "$1[redacted]")
    .replace(/((?:api[_ -]?key|x-api-key)\s*[:=]\s*)[^\s,;]+/gi, "$1[redacted]")
    .replace(/\b(sk-[A-Za-z0-9_-]{8,})\b/g, "[redacted]")
    .trim();
}

export function modelServiceIssue(rawError: unknown): ModelServiceIssue {
  const raw = typeof rawError === "string" ? rawError : "";
  const safe = sanitizeModelServiceError(raw);
  const lower = safe.toLowerCase();

  if (
    lower.includes("session_model_service_config_incomplete")
    || lower.includes("missing_api_key")
    || lower.includes("api key required")
  ) {
    return {
      title: "API key required",
      detail: "Open Runtime settings, enter the Session API key, and save it before sending another message.",
    };
  }

  if (
    /\b(?:401|403)\b/.test(lower)
    || lower.includes("unauthorized")
    || lower.includes("forbidden")
    || lower.includes("authentication failed")
    || lower.includes("invalid api key")
    || lower.includes("invalid_api_key")
  ) {
    return {
      title: "Model authentication failed",
      detail: "Open Runtime settings and verify the Session API key. If the key is correct, check that it has access to the configured model and Base URL.",
    };
  }

  if (
    /\b404\b/.test(lower)
    || lower.includes("model not found")
    || lower.includes("unknown model")
    || lower.includes("model_not_found")
  ) {
    return {
      title: "Model unavailable",
      detail: "Open Runtime settings and verify the model name and Base URL. The configured model may not exist or may not be available to this account.",
    };
  }

  if (
    lower.includes("connection refused")
    || lower.includes("failed to connect")
    || lower.includes("network")
    || lower.includes("timed out")
    || lower.includes("timeout")
    || lower.includes("dns")
  ) {
    return {
      title: "Model service unavailable",
      detail: "Check the Base URL, network connection, and model service status, then retry.",
    };
  }

  return {
    title: "Model request failed",
    detail: safe || "The model service did not provide a usable reason. Check Runtime settings and retry.",
  };
}

export function commandSessionId(command: unknown): string | undefined {
  if (!command || typeof command !== "object") return undefined;
  const sessionId = (command as Record<string, unknown>).session_id;
  return typeof sessionId === "string" && sessionId ? sessionId : undefined;
}

export function isModelSubmissionCommand(command: unknown): boolean {
  if (!command || typeof command !== "object") return false;
  const type = (command as Record<string, unknown>).type;
  return type === "turn_submit" || type === "turn_supplement";
}
