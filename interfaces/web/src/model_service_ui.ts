import { t } from "./i18n";
import { ModelErrorClass, modelErrorReason, parseModelError } from "./model_error";
import { Session } from "./protocol";

// 展示标签在渲染期取词，跟随语言切换。
export const unconfiguredModelLabel = () => t("modelService.unconfigured");

export type ModelServiceIssue = {
  title: string;
  detail: string;
  diagnostic?: string;
};

export const noModelEndpointsIssue = (): ModelServiceIssue => ({
  title: t("modelService.noEndpointsTitle"),
  detail: t("modelService.noEndpointsDetail"),
});

export function modelDisplayName(
  session: Pick<Session, "runtime_profile"> | undefined,
): string {
  const profile = session?.runtime_profile;
  return profile?.model.trim() || unconfiguredModelLabel();
}


export function sessionModelConfigurationIssue(
  session: Pick<Session, "runtime_profile"> | undefined,
): ModelServiceIssue | null {
  const profile = session?.runtime_profile;
  if (!profile || !profile.model.trim()) {
    return {
      title: t("modelService.sessionNotConfiguredTitle"),
      detail: t("modelService.sessionNotConfiguredDetail"),
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

function providerReason(safeError: string): string {
  return modelErrorReason(safeError)
    .replace(/^http\s+\d{3}\s*:?\s*/i, "")
    .trim();
}

/** Classified guidance. Returns undefined for classes the UI has no copy for. */
function classIssue(
  class_: ModelErrorClass,
  reason: string,
  safe: string,
  code?: string,
): ModelServiceIssue | undefined {
  const diagnostic = code ? { diagnostic: safe } : {};
  const detail = (guidance: string) => serviceDetail(reason, guidance);
  switch (class_) {
    case "authentication":
      return {
        title: t("service.authFailedTitle"),
        detail: detail(t("service.authFailedDetail")),
        ...diagnostic,
      };
    case "permission":
      return {
        title: t("service.permissionTitle"),
        detail: detail(t("service.permissionDetail")),
        ...diagnostic,
      };
    case "billing":
      return {
        title: t("service.billingTitle"),
        detail: detail(t("service.billingDetail")),
        ...diagnostic,
      };
    case "subscription":
      return {
        title: t("service.subscriptionTitle"),
        detail: detail(t("service.subscriptionDetail")),
        ...diagnostic,
      };
    case "quota":
      return {
        title: t("service.quotaTitle"),
        detail: detail(t("service.quotaDetail")),
        ...diagnostic,
      };
    case "rate_limited":
      return {
        title: t("service.rateLimitedTitle"),
        detail: detail(t("service.rateLimitedDetail")),
        ...diagnostic,
      };
    case "overloaded":
      return {
        title: t("service.overloadedTitle"),
        detail: reason || t("service.overloadedFallback"),
        ...diagnostic,
      };
    case "timeout":
      return {
        title: t("service.timeoutTitle"),
        detail: detail(t("service.timeoutDetail")),
        ...diagnostic,
      };
    case "server_error":
      return {
        title: t("service.serverErrorTitle"),
        detail: detail(t("service.serverErrorDetail")),
        ...diagnostic,
      };
    case "transport":
      return {
        title: t("service.unreachableTitle"),
        detail: detail(t("service.unreachableDetail")),
        ...diagnostic,
      };
    case "invalid_request":
      return {
        title: t("service.invalidRequestTitle"),
        detail: detail(t("service.invalidRequestDetail")),
        ...diagnostic,
      };
    case "model_not_found":
      return {
        title: t("service.unavailableTitle"),
        detail: detail(t("service.unavailableDetail")),
        ...diagnostic,
      };
    case "input_too_large":
      return {
        title: t("service.inputTooLargeTitle"),
        detail: detail(t("service.inputTooLargeDetail")),
        ...diagnostic,
      };
    case "content_policy":
      return {
        title: t("service.contentPolicyTitle"),
        detail: detail(t("service.contentPolicyDetail")),
        ...diagnostic,
      };
    default:
      return undefined;
  }
}

/** Classes that stay actionable even when they arrive as a stream failure. */
const STREAM_ACTIONABLE: readonly ModelErrorClass[] = [
  "authentication",
  "permission",
  "billing",
  "subscription",
  "quota",
  "invalid_request",
  "model_not_found",
  "content_policy",
];

function serviceDetail(reason: string, guidance: string): string {
  return reason ? t("service.responseDetail", { reason, guidance }) : guidance;
}

function streamFailureMessage(safeError: string): string {
  const match = safeError.match(/(?:^|\s)message=(.+)$/i);
  return match?.[1]?.trim() ?? "";
}

export function modelServiceIssue(rawError: unknown): ModelServiceIssue {
  const raw = typeof rawError === "string" ? rawError : "";
  const safe = sanitizeModelServiceError(raw);
  const lower = safe.toLowerCase();
  const reason = providerReason(safe);

  const facts = parseModelError(safe);
  const streamFailure =
    lower.includes("model_responses_stream_failed")
    || lower.includes("model responses stream error");
  if (streamFailure && STREAM_ACTIONABLE.includes(facts.class)) {
    // A stream failure that the provider classified as credentials, billing,
    // plan, quota, bad request or content policy keeps that actionable copy
    // instead of the generic "could not complete" message.
    const issue = classIssue(facts.class, streamFailureMessage(safe) || reason, safe, facts.code);
    if (issue) return issue;
  }

  if (
    lower.includes("model_responses_stream_failed")
    && (
      lower.includes("server_is_overloaded")
      || lower.includes("currently overloaded")
      || lower.includes("service_unavailable")
      || lower.includes("temporarily_unavailable")
    )
  ) {
    return {
      title: t("service.overloadedTitle"),
      detail: streamFailureMessage(safe) || t("service.overloadedFallback"),
      ...(safe ? { diagnostic: safe } : {}),
    };
  }

  if (
    lower.includes("model_responses_stream_failed")
    || lower.includes("model responses stream error")
  ) {
    return {
      title: t("service.streamFailedTitle"),
      detail: streamFailureMessage(safe) || t("service.streamFailedFallback"),
      ...(safe ? { diagnostic: safe } : {}),
    };
  }

  if (
    lower.includes("session_model_service_config_incomplete")
    || lower.includes("missing_api_key")
    || lower.includes("api key required")
  ) {
    return {
      title: t("service.authTitle"),
      detail: t("service.authDetail"),
    };
  }

  if (
    lower.includes("cache_control")
    && (lower.includes("maximum of") || lower.includes("too many") || lower.includes("found "))
  ) {
    return {
      title: t("service.cacheTitle"),
      detail: serviceDetail(
        reason,
        t("service.cacheDetail"),
      ),
    };
  }

  if (!streamFailure && facts.class !== "unknown") {
    const issue = classIssue(facts.class, reason, safe, facts.code);
    if (issue) return issue;
  }

  if (
    /(?:\b|model_http_)(?:401|403)\b/.test(lower)
    || lower.includes("unauthorized")
    || lower.includes("forbidden")
    || lower.includes("authentication failed")
    || lower.includes("invalid api key")
    || lower.includes("invalid_api_key")
  ) {
    return {
      title: t("service.authFailedTitle"),
      detail: serviceDetail(
        reason,
        t("service.authFailedDetail"),
      ),
    };
  }

  if (
    /(?:\b|model_http_)404\b/.test(lower)
    || lower.includes("model not found")
    || lower.includes("unknown model")
    || lower.includes("model_not_found")
  ) {
    return {
      title: t("service.unavailableTitle"),
      detail: serviceDetail(
        reason,
        t("service.unavailableDetail"),
      ),
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
      title: t("service.unreachableTitle"),
      detail: serviceDetail(
        reason,
        t("service.unreachableDetail"),
      ),
    };
  }

  return {
    title: t("service.failedTitle"),
    detail: safe || t("service.failedDetailFallback"),
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
