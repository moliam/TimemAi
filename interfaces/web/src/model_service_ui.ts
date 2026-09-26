import { t } from "./i18n";
import { Session } from "./protocol";

// 展示标签在渲染期取词，跟随语言切换。
export const unconfiguredModelLabel = () => t("modelService.unconfigured");

export type ModelServiceIssue = {
  title: string;
  detail: string;
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
  return safeError
    .replace(/^model_http_\d{3}\s*:\s*/i, "")
    .replace(/^http\s+\d{3}\s*:?\s*/i, "")
    .trim();
}

function serviceDetail(reason: string, guidance: string): string {
  return reason ? t("service.responseDetail", { reason, guidance }) : guidance;
}

export function modelServiceIssue(rawError: unknown): ModelServiceIssue {
  const raw = typeof rawError === "string" ? rawError : "";
  const safe = sanitizeModelServiceError(raw);
  const lower = safe.toLowerCase();
  const reason = providerReason(safe);

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
