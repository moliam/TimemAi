// Canonical model-error decoding for the Web Interface.
//
// Core classifies a provider failure once (HTTP status + documented provider
// business code) and writes the class into the error string:
//
//   model_http_429[billing,zhipu/1113]: 您的账户已欠费，请充值后重试
//   model_http_503[overloaded]: upstream overloaded
//   model_responses_stream_failed: class=overloaded event_type=error code=...
//
// This module reads that header. It must never classify by matching a
// provider's natural-language message: the same class must render the same
// guidance for every provider, and adding a provider is a Core table change.

export type ModelErrorClass =
  | "authentication"
  | "permission"
  | "billing"
  | "subscription"
  | "quota"
  | "rate_limited"
  | "overloaded"
  | "timeout"
  | "server_error"
  | "transport"
  | "invalid_request"
  | "model_not_found"
  | "input_too_large"
  | "content_policy"
  | "unknown";

export type ModelErrorFacts = {
  status?: number;
  class: ModelErrorClass;
  provider?: string;
  code?: string;
};

const KNOWN_CLASSES: readonly ModelErrorClass[] = [
  "authentication",
  "permission",
  "billing",
  "subscription",
  "quota",
  "rate_limited",
  "overloaded",
  "timeout",
  "server_error",
  "transport",
  "invalid_request",
  "model_not_found",
  "input_too_large",
  "content_policy",
  "unknown",
];

// Symbolic codes used by the OpenAI family (HTTP bodies and Responses stream
// failure events). Mirror of Core's `OPENAI_CODES`; status-only providers
// (DeepSeek, Kimi) need no table.
const SYMBOLIC_CODES: ReadonlyArray<[string, ModelErrorClass]> = [
  ["invalid_request_error", "invalid_request"],
  ["invalid_api_key", "authentication"],
  ["authentication_error", "authentication"],
  ["permission_error", "permission"],
  ["model_not_found", "model_not_found"],
  ["not_found_error", "model_not_found"],
  ["insufficient_quota", "quota"],
  ["rate_limit_exceeded", "rate_limited"],
  ["rate_limited", "rate_limited"],
  ["context_length_exceeded", "input_too_large"],
  ["content_policy_violation", "content_policy"],
  ["server_error", "server_error"],
  ["internal_error", "server_error"],
  ["api_error", "server_error"],
  ["service_unavailable", "overloaded"],
  ["temporarily_unavailable", "overloaded"],
  ["server_is_overloaded", "overloaded"],
  ["overloaded_error", "overloaded"],
  ["engine_overloaded", "overloaded"],
  ["timeout", "timeout"],
];

function classFromToken(token: string | undefined): ModelErrorClass | undefined {
  const value = token?.trim().toLowerCase();
  return KNOWN_CLASSES.find((candidate) => candidate === value);
}

function classFromSymbolicCode(code: string | undefined): ModelErrorClass | undefined {
  const value = code?.trim().toLowerCase();
  if (!value) return undefined;
  return SYMBOLIC_CODES.find(([id]) => id === value)?.[1];
}

/** Provider-neutral class for an HTTP status (mirror of Core's table). */
export function classFromStatus(status: number): ModelErrorClass {
  switch (status) {
    case 400:
    case 422:
      return "invalid_request";
    case 401:
      return "authentication";
    case 402:
      return "billing";
    case 403:
      return "permission";
    case 404:
      return "model_not_found";
    case 408:
      return "timeout";
    case 409:
      return "server_error";
    case 413:
      return "input_too_large";
    case 425:
      return "timeout";
    case 429:
      return "rate_limited";
    case 500:
    case 502:
      return "server_error";
    case 503:
      return "overloaded";
    case 504:
      return "timeout";
    default:
      if (status >= 400 && status < 500) return "invalid_request";
      if (status >= 500 && status < 600) return "server_error";
      return "unknown";
  }
}

function keyValue(input: string, key: string): string | undefined {
  for (const part of input.split(/\s+/)) {
    if (part.startsWith(key)) return part.slice(key.length).replace(/,+$/, "").toLowerCase();
  }
  return undefined;
}

/** Decode facts from a canonical Core error string, or the status-only class. */
export function parseModelError(raw: unknown): ModelErrorFacts {
  const input = typeof raw === "string" ? raw.trim() : "";
  const lower = input.toLowerCase();
  if (!input) return { class: "unknown" };
  if (lower.startsWith("model_timeout")) return { class: "timeout" };
  for (const prefix of [
    "model_network_error",
    "model_dns_error",
    "model_connect_error",
    "model_proxy_error",
    "model_body_error",
  ]) {
    if (lower.startsWith(prefix)) return { class: "transport" };
  }
  if (lower.startsWith("model_responses_stream_failed")) {
    const code = keyValue(input, "code=");
    return {
      class:
        classFromToken(keyValue(input, "class="))
        ?? classFromSymbolicCode(code)
        ?? classFromSymbolicCode(keyValue(input, "type="))
        ?? "unknown",
      ...(code ? { code } : {}),
    };
  }
  if (lower === "model_responses_stream_missing_terminal") return { class: "unknown" };
  const match = /^model_http_(\d{1,4})(?:\[([^\]]*)\])?/i.exec(input);
  if (!match) return { class: "unknown" };
  const status = Number(match[1]);
  const header = match[2];
  if (!header) return { status, class: classFromStatus(status) };
  const [classToken, source] = header.split(",", 2);
  const facts: ModelErrorFacts = {
    status,
    class: classFromToken(classToken) ?? classFromStatus(status),
  };
  if (source) {
    const slash = source.indexOf("/");
    if (slash >= 0) {
      facts.provider = source.slice(0, slash);
      facts.code = source.slice(slash + 1);
    } else {
      facts.code = source;
    }
  }
  return facts;
}

/** Provider reason without the canonical header (for display). */
export function modelErrorReason(raw: unknown): string {
  const input = typeof raw === "string" ? raw : "";
  return input.replace(/^model_http_\d{1,4}(?:\[[^\]]*\])?\s*:\s*/i, "").trim();
}
