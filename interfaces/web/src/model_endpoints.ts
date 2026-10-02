import { t } from "./i18n";
import { unconfiguredModelLabel } from "./model_service_ui";

export type ModelEndpoint = {
  id: string;
  name: string;
  model: string;
  api_protocol: string;
  response_protocol: string;
  base_url: string;
  max_llm_input_tokens: number;
  max_llm_output_tokens: number;
  stream: boolean;
  api_key_configured: boolean;
  http_headers: Record<string, string>;
  request_fields: Record<string, unknown>;
  allow_cross_origin_redirects: boolean;
  private_ca_configured: boolean;
  requirements?: EndpointRequirements;
  catalog_id?: string | null;
  reasoning_effort?: string | null;
};

export type ModelEndpointDraft = {
  id?: string;
  name: string;
  model: string;
  api_protocol: string;
  response_protocol: string;
  base_url: string;
  max_llm_input_tokens: number;
  max_llm_output_tokens: number;
  stream: boolean;
  api_key?: string;
  http_headers: Record<string, string>;
  request_fields: Record<string, unknown>;
  allow_cross_origin_redirects: boolean;
  private_ca_pem?: string;
  requirements?: EndpointRequirements;
  catalog_id?: string | null;
  reasoning_effort?: string | null;
};

type ModelEndpointProfile = {
  model_endpoint_id?: string | null;
  model: string;
  api_protocol: string;
  response_protocol: string;
  base_url: string;
  max_llm_input_tokens: number;
  max_llm_output_tokens: number;
  stream: boolean;
  api_key_configured: boolean;
};

export const MODEL_CONTEXT_WINDOW_OPTIONS = [
  100_000, 200_000, 300_000, 1_000_000,
] as const;
export const MODEL_OUTPUT_TOKEN_OPTIONS = [10_000, 20_000, 50_000] as const;

export const REASONING_EFFORT_OPTIONS = [
  "minimal",
  "low",
  "medium",
  "high",
  "xhigh",
] as const;

/// Sentinel value: turn thinking off. Core translates it per API protocol
/// (`thinking={"type":"disabled"}` for OpenAI-compatible, `reasoning.effort=none`
/// for responses).
export const REASONING_EFFORT_DISABLED = "disabled" as const;

// Mirrors the Core-side u32 token-count bound (config_edit::parse_token_count).
export const MAX_LLM_INPUT_TOKENS_CEILING = 4_294_967_295;

export function isValidMaxLlmInputTokens(value: number): boolean {
  return (
    Number.isInteger(value) && value >= 1 && value <= MAX_LLM_INPUT_TOKENS_CEILING
  );
}

export function formatContextWindowTokens(tokens: number): string {
  if (!Number.isFinite(tokens) || tokens < 1_000) return String(tokens);
  if (tokens % 1_000_000 === 0) return `${tokens / 1_000_000}M`;
  return `${tokens / 1_000}K`;
}

/// Compact protocol label for endpoint buttons: openai-comp / openai-resp /
/// anthropic; unknown values fall back to the raw string.
export function apiProtocolShort(protocol: string): string {
  switch (protocol) {
    case "openai-compatible":
      return "openai-comp";
    case "openai-responses":
      return "openai-resp";
    case "anthropic":
      return "anthropic";
    default:
      return protocol;
  }
}

export function endpointMatchesProfile(
  endpoint: ModelEndpoint,
  profile: ModelEndpointProfile | undefined,
): boolean {
  if (profile?.model_endpoint_id) return endpoint.id === profile.model_endpoint_id;
  return (
    !!profile &&
    endpoint.model === profile.model &&
    endpoint.api_protocol === profile.api_protocol &&
    endpoint.response_protocol === profile.response_protocol &&
    endpoint.base_url === profile.base_url &&
    endpoint.max_llm_input_tokens === profile.max_llm_input_tokens &&
    endpoint.max_llm_output_tokens === profile.max_llm_output_tokens &&
    endpoint.stream === profile.stream &&
    endpoint.api_key_configured === profile.api_key_configured
  );
}

export function endpointNameForProfile(
  endpoints: readonly ModelEndpoint[],
  profile: ModelEndpointProfile | undefined,
): string | undefined {
  return endpoints.find((endpoint) => endpointMatchesProfile(endpoint, profile))
    ?.name;
}

// A saved preset match is a display name, not proof that a Session has a route.
// Render the Host-provided profile without changing selection or admission.
export function endpointLabelForProfile(
  endpoints: readonly ModelEndpoint[],
  profile: ModelEndpointProfile | undefined,
): string {
  if (profile?.model_endpoint_id) {
    return endpoints.find((endpoint) => endpoint.id === profile.model_endpoint_id)?.name
      ?? t("modelService.endpointDeleted");
  }
  if (!profile?.model.trim()) return unconfiguredModelLabel();
  return endpointNameForProfile(endpoints, profile)
    ?? t("modelService.customConfig", { model: profile.model.trim() });
}

export function endpointDraftValid(draft: ModelEndpointDraft): boolean {
  return (
    !!draft.name.trim() &&
    !!draft.model.trim() &&
    !!draft.api_protocol.trim() &&
    !!draft.response_protocol.trim() &&
    !!draft.base_url.trim() &&
    Object.keys(draft.http_headers ?? {}).every((name) => !!name.trim()) &&
    Object.keys(draft.request_fields ?? {}).every((name) => !!name.trim()) &&
    isValidMaxLlmInputTokens(draft.max_llm_input_tokens) &&
    Number.isInteger(draft.max_llm_output_tokens) &&
    draft.max_llm_output_tokens >= 512 && draft.max_llm_output_tokens <= MAX_LLM_INPUT_TOKENS_CEILING
  );
}

function canonicalEndpointValue(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(canonicalEndpointValue);
  if (value && typeof value === "object") {
    return Object.fromEntries(
      Object.entries(value as Record<string, unknown>)
        .filter(([, item]) => item !== undefined)
        .sort(([left], [right]) => left.localeCompare(right))
        .map(([key, item]) => [key, canonicalEndpointValue(item)]),
    );
  }
  return value;
}

function comparableEndpointDraft(draft: ModelEndpointDraft): unknown {
  const requirements = draft.requirements;
  return canonicalEndpointValue({
    id: draft.id ?? null,
    catalog_id: draft.catalog_id ?? null,
    name: draft.name,
    model: draft.model,
    api_protocol: draft.api_protocol,
    response_protocol: draft.response_protocol,
    base_url: draft.base_url,
    max_llm_input_tokens: draft.max_llm_input_tokens,
    max_llm_output_tokens: draft.max_llm_output_tokens,
    stream: draft.stream,
    api_key: draft.api_key ?? null,
    http_headers: draft.http_headers ?? {},
    request_fields: draft.request_fields ?? {},
    allow_cross_origin_redirects: draft.allow_cross_origin_redirects,
    private_ca_pem: draft.private_ca_pem ?? null,
    reasoning_effort: draft.reasoning_effort ?? null,
    requirements: requirements ? {
      version: requirements.version,
      provider: requirements.provider ?? null,
      allowed_reasoning: requirements.allowed_reasoning == null
        ? null
        : [...new Set(requirements.allowed_reasoning)],
      adaptive_reasoning: requirements.adaptive_reasoning ?? null,
      field_sources: requirements.field_sources,
    } : null,
  });
}

export function endpointDraftChanged(
  initial: ModelEndpointDraft,
  current: ModelEndpointDraft,
): boolean {
  return JSON.stringify(comparableEndpointDraft(initial)) !== JSON.stringify(comparableEndpointDraft(current));
}

export type CatalogModel = {
  id: string; revision: number; provider: string; model: string; label: string; base_url: string;
  efforts: string[]; default_effort: string; middle_default: boolean;
  min_input: number; max_input: number; min_output: number; max_output: number;
  context_window: number;
  protocols: { protocol: string; base_url?: string; disabled_reason: string | null;
    fixed_effort: string | null; fixed_reason: string | null }[];
};


export type EndpointRequirements = {
  version: number;
  provider?: string | null;
  allowed_reasoning?: string[] | null;
  adaptive_reasoning?: boolean | null;
  field_sources: Record<string, "template" | "user">;
};

export function effectiveAllowedReasoning(
  allowed: readonly string[] | null | undefined,
  modelEfforts: readonly string[] | undefined,
  daily: string | null | undefined,
): string[] {
  return [...new Set(allowed ?? modelEfforts ?? (daily ? [daily] : []))];
}

const CUSTOM_REASONING_LEVEL_ORDER = ["none", ...REASONING_EFFORT_OPTIONS] as const;

function orderReasoningLevels(
  levels: readonly string[],
  modelEfforts: readonly string[] | undefined,
): string[] {
  const order = modelEfforts ?? CUSTOM_REASONING_LEVEL_ORDER;
  const rank = new Map(order.map((level, index) => [level, index]));
  const unique = [...new Set(levels)];
  // Unknown custom levels carry user-declared ordering semantics. Do not guess
  // their rank or move them relative to recognized levels.
  if (unique.some((level) => !rank.has(level))) return unique;
  return unique
    .map((level, index) => ({ level, index, rank: rank.get(level) }))
    .sort((left, right) =>
      left.rank !== undefined && right.rank !== undefined
        ? left.rank - right.rank
        : left.rank !== undefined
          ? -1
          : right.rank !== undefined
            ? 1
            : left.index - right.index,
    )
    .map(({ level }) => level);
}

export function toggleAllowedReasoning(
  allowed: readonly string[] | null | undefined,
  modelEfforts: readonly string[] | undefined,
  daily: string | null | undefined,
  defaultEffort: string | null | undefined,
  level: string,
  checked: boolean,
): { allowedReasoning: string[]; dailyReasoning: string } {
  const current = effectiveAllowedReasoning(allowed, modelEfforts, daily);
  const changed = checked
    ? [...new Set([...current, level])]
    : current.filter((value) => value !== level);
  const retained = changed.length > 0 ? changed : current.length > 0 ? current : [level];
  const next = orderReasoningLevels(retained, modelEfforts);
  const dailyReasoning = daily && next.includes(daily)
    ? daily
    : defaultEffort && next.includes(defaultEffort)
      ? defaultEffort
      : next[0];
  return { allowedReasoning: next, dailyReasoning };
}

export type EndpointCapabilityIssue =
  | { code: "daily_not_allowed" }
  | { code: "model_capabilities_missing" }
  | { code: "provider_protocol_unsupported" }
  | { code: "protocol_unsupported" }
  | { code: "fixed_reasoning"; level: string }
  | { code: "daily_reasoning_unsupported" }
  | { code: "allowed_reasoning_unsupported" }
  | { code: "input_budget" }
  | { code: "output_budget" }
  | { code: "context_budget" };

export function endpointCapabilityIssue(
  draft: ModelEndpointDraft,
  model: CatalogModel | undefined,
): EndpointCapabilityIssue | null {
  const allowed = draft.requirements?.allowed_reasoning;
  const daily = draft.reasoning_effort === REASONING_EFFORT_DISABLED
    ? "none"
    : draft.reasoning_effort ?? model?.default_effort;
  if (allowed != null && (!allowed.length || (daily != null && !allowed.includes(daily))))
    return { code: "daily_not_allowed" };
  const provider = draft.requirements?.provider;
  if (provider === "zhipu" && !model) return { code: "model_capabilities_missing" };
  if ((provider === "zhipu" && draft.api_protocol !== "openai-compatible")
    || (provider === "openai" && draft.api_protocol === "anthropic"))
    return { code: "provider_protocol_unsupported" };
  if (!model) return null;
  const protocol = model.protocols.find((item) => item.protocol === draft.api_protocol);
  if (!protocol || protocol.disabled_reason) return { code: "protocol_unsupported" };
  if (protocol.fixed_effort != null && daily !== protocol.fixed_effort)
    return { code: "fixed_reasoning", level: protocol.fixed_effort };
  if (daily != null && !model.efforts.includes(daily))
    return { code: "daily_reasoning_unsupported" };
  if (allowed != null && allowed.some((level) => !model.efforts.includes(level)))
    return { code: "allowed_reasoning_unsupported" };
  if (draft.max_llm_input_tokens < model.min_input || draft.max_llm_input_tokens > model.max_input)
    return { code: "input_budget" };
  if (draft.max_llm_output_tokens < model.min_output || draft.max_llm_output_tokens > model.max_output)
    return { code: "output_budget" };
  if (draft.max_llm_input_tokens + draft.max_llm_output_tokens > model.context_window)
    return { code: "context_budget" };
  return null;
}

export function endpointCapabilityIssueMessage(issue: EndpointCapabilityIssue): string {
  switch (issue.code) {
    case "daily_not_allowed": return t("endpoints.validationDailyNotAllowed");
    case "model_capabilities_missing": return t("endpoints.validationCapabilitiesMissing");
    case "provider_protocol_unsupported": return t("endpoints.validationProviderProtocol");
    case "protocol_unsupported": return t("endpoints.validationProtocolUnsupported");
    case "fixed_reasoning": return t("endpoints.validationFixedReasoning", { level: issue.level });
    case "daily_reasoning_unsupported": return t("endpoints.validationDailyUnsupported");
    case "allowed_reasoning_unsupported": return t("endpoints.validationAllowedUnsupported");
    case "input_budget": return t("endpoints.validationInputBudget");
    case "output_budget": return t("endpoints.validationOutputBudget");
    case "context_budget": return t("endpoints.validationContextBudget");
  }
}

export function endpointImportIssueMessage(issue: string): string {
  const code = issue.split(":", 1)[0];
  switch (code) {
    case "claude_effort_level_not_imported": return t("endpoints.importIssueClaudeEffortSkipped");
    case "claude_thinking_tokens_not_imported": return t("endpoints.importIssueClaudeThinkingSkipped");
    case "claude_thinking_tokens_invalid": return t("endpoints.importIssueClaudeThinkingInvalid");
    case "codex_ept_auth_token_missing": return t("endpoints.importIssueApiKeyMissing");
    case "claude_model_missing":
    case "codex_default_model_missing":
    case "codex_profile_model_missing":
    case "codex_profile_overlay_model_missing":
    case "codex_provider_model_missing":
      return t("endpoints.importIssueModelMissing");
    case "claude_settings_missing": return t("endpoints.importIssueClaudeSettingsMissing");
    case "codex_model_providers_missing": return t("endpoints.importIssueCodexProvidersMissing");
    case "codex_provider_base_url_missing": return t("endpoints.importIssueBaseUrlMissing");
    case "codex_provider_wire_api_unsupported": return t("endpoints.importIssueProtocolUnsupported");
    case "model_endpoint_import_candidate_limit":
    case "codex_profile_overlay_limit":
      return t("endpoints.importIssueLimitReached");
    case "claude_settings_read_failed":
    case "claude_settings_invalid":
    case "codex_config_read_failed":
    case "codex_config_invalid":
    case "codex_ept_auth_read_failed":
    case "codex_ept_auth_invalid":
    case "codex_profile_overlay_read_failed":
    case "codex_profile_overlay_invalid":
      return t("endpoints.importIssueConfigUnreadable");
    default: return t("endpoints.importIssueUnknown");
  }
}

export function endpointImportCommandErrorMessage(
  operation: "scan" | "apply",
  error: string | undefined,
): string {
  const code = error?.split(":", 1)[0] ?? "";
  if (operation === "scan") {
    if (code === "model_endpoint_import_directory_required"
      || code === "model_endpoint_import_directory_empty")
      return t("errors.endpointImportDirectoryRequired");
    if (code === "model_endpoint_import_home_user_unsupported")
      return t("errors.endpointImportHomeUserUnsupported");
    if (code === "home_directory_unavailable")
      return t("errors.endpointImportHomeUnavailable");
    if (/disconnect|timeout|socket|connection/i.test(error ?? ""))
      return t("errors.checkConnection");
    return t("errors.endpointImportScanRetry");
  }
  if (code === "model_endpoint_import_selection_empty"
    || code === "model_endpoint_import_candidate_id_empty")
    return t("errors.endpointImportSelectionRequired");
  if (code === "model_endpoint_import_selection_too_large")
    return t("errors.endpointImportSelectionTooLarge");
  if (code === "model_endpoint_import_candidate_not_found")
    return t("errors.endpointImportCandidateExpired");
  if (code.startsWith("model_endpoint_store_"))
    return t("errors.endpointStorageFailed");
  if (/disconnect|timeout|socket|connection/i.test(error ?? ""))
    return t("errors.checkConnection");
  return t("errors.endpointImportApplyRetry");
}

export function endpointSaveErrorMessage(error: string | undefined): string {
  if (!error) return t("errors.endpointSaveRetry");
  if (error === "model_endpoint_name_conflict") return t("errors.endpointNameConflict");
  if (error.startsWith("empty_model endpoint name")) return t("errors.endpointNameRequired");
  if (error.startsWith("empty_model endpoint model")) return t("errors.endpointModelRequired");
  if (error.startsWith("empty_model endpoint base url")) return t("errors.endpointBaseUrlRequired");
  if (error.startsWith("invalid_model_endpoint_api_key")) return t("errors.endpointApiKeyInvalid");
  if (error.startsWith("invalid_model_endpoint_headers")) return t("errors.endpointHeadersInvalid");
  if (error.startsWith("invalid_model_endpoint_request_fields")) return t("errors.endpointRequestFieldsInvalid");
  if (error.startsWith("invalid_model_endpoint_private_ca")) return t("errors.endpointPrivateCaInvalid");
  if (error === "model_endpoint_stream_requires_openai_compatible") return t("errors.endpointStreamProtocol");
  if (error === "model_endpoint_reasoning_effort_requires_openai_protocol") return t("errors.endpointReasoningProtocol");
  if ([
    "daily_reasoning_not_in_allowed_set",
    "invalid_allowed_reasoning",
    "allowed_reasoning_exceeds_model_capability",
    "catalog_reasoning_not_supported",
  ].includes(error)) return t("errors.endpointReasoningInvalid");
  if ([
    "provider_protocol_adapter_not_implemented",
    "catalog_protocol_not_supported",
    "catalog_model_mismatch",
  ].includes(error)) return t("errors.endpointProtocolInvalid");
  if (error === "model_capabilities_not_declared" || error === "catalog_model_not_found")
    return t("errors.endpointCapabilitiesUnavailable");
  if (error === "catalog_token_budget_out_of_range"
    || error === "invalid_model_endpoint_max_input_tokens"
    || error === "invalid_model_endpoint_max_output_tokens")
    return t("errors.endpointTokenBudgetInvalid");
  if (error.startsWith("model_endpoint_store_")) return t("errors.endpointStorageFailed");
  if (/disconnect|timeout|socket|connection/i.test(error)) return t("errors.endpointConnectionInterrupted");
  return t("errors.endpointSaveRetry");
}

const suggestionFields = ["name", "model", "provider", "api_protocol", "base_url", "stream", "reasoning_effort", "allowed_reasoning", "adaptive_reasoning", "max_llm_input_tokens", "max_llm_output_tokens"];
export function initialEndpointRequirements(endpoint?: ModelEndpoint, catalog: readonly CatalogModel[] = []): EndpointRequirements {
  if (endpoint?.requirements?.version === 1) return structuredClone(endpoint.requirements);
  return { version: 1, provider: endpoint?.requirements?.provider ?? catalog.find(m => m.id === endpoint?.catalog_id)?.provider, field_sources: endpoint ? Object.fromEntries(suggestionFields.map(k => [k, "user" as const])) : {} };
}

export function editEndpoint(draft: ModelEndpointDraft, patch: Partial<ModelEndpointDraft>): ModelEndpointDraft {
  const requirements = draft.requirements ?? initialEndpointRequirements();
  return { ...draft, ...patch, requirements: { ...requirements,
    field_sources: { ...requirements.field_sources, ...Object.fromEntries(Object.keys(patch).filter(k => suggestionFields.includes(k)).map(k => [k, "user" as const])) } } };
}

export function editEndpointRequirements(draft: ModelEndpointDraft, patch: Partial<EndpointRequirements>): ModelEndpointDraft {
  const requirements = draft.requirements ?? initialEndpointRequirements();
  return { ...draft, requirements: { ...requirements, ...patch, field_sources: {
    ...requirements.field_sources, ...Object.fromEntries(Object.keys(patch).map(k => [k, "user" as const])),
  } } };
}

// An absent protocol is not a license to invent its address.
export function templateBaseUrl(model: CatalogModel | undefined, protocol: string): string | undefined {
  const profile = model?.protocols.find(p => p.protocol === protocol);
  return profile ? profile.base_url ?? model?.base_url : undefined;
}

// Visibility is value-based, independent of who last edited the field.
export function canRestoreEndpointTemplateUrl(draft: ModelEndpointDraft, model?: CatalogModel): boolean {
  const defaultUrl = templateBaseUrl(model, draft.api_protocol);
  return defaultUrl !== undefined && draft.base_url !== defaultUrl;
}

export function restoreEndpointTemplateUrl(draft: ModelEndpointDraft, model?: CatalogModel): ModelEndpointDraft {
  const base_url = templateBaseUrl(model, draft.api_protocol);
  if (base_url === undefined) return draft;
  const requirements = draft.requirements ?? initialEndpointRequirements();
  return { ...draft, base_url, requirements: { ...requirements,
    field_sources: { ...requirements.field_sources, base_url: "template" } } };
}

export function restoreEndpointTemplateReasoning(
  draft: ModelEndpointDraft,
  model?: CatalogModel,
): ModelEndpointDraft {
  if (!model || draft.catalog_id !== model.id) return draft;
  const requirements = draft.requirements ?? initialEndpointRequirements();
  return {
    ...draft,
    requirements: {
      ...requirements,
      allowed_reasoning: [...model.efforts],
      field_sources: {
        ...requirements.field_sources,
        allowed_reasoning: "template",
      },
    },
  };
}

export function changeEndpointProtocol(draft: ModelEndpointDraft, api_protocol: string, model?: CatalogModel): ModelEndpointDraft {
  const next = editEndpoint(draft, { api_protocol, stream: api_protocol !== "anthropic" });
  return draft.requirements?.field_sources.base_url === "template" && draft.catalog_id === model?.id
    ? restoreEndpointTemplateUrl(next, model) : next;
}

export function applyEndpointTemplate(draft: ModelEndpointDraft, model?: CatalogModel): ModelEndpointDraft {
  if (!model) return { ...draft, catalog_id: null };
  const protocol = model.protocols.find(p => !p.disabled_reason);
  if (!protocol) return draft;
  const requirements = structuredClone(draft.requirements ?? initialEndpointRequirements());
  const next = { ...draft, catalog_id: model.id, requirements };
  const effectiveProtocol = requirements.field_sources.api_protocol === "user" ? draft.api_protocol : protocol.protocol;
  const base_url = templateBaseUrl(model, effectiveProtocol);
  const output = Math.min(draft.max_llm_output_tokens, model.max_output);
  const suggestions: Partial<ModelEndpointDraft> = {
    name: model.label, model: model.model, ...(base_url === undefined ? {} : { base_url }), api_protocol: protocol.protocol,
    stream: true, reasoning_effort: protocol.fixed_effort ?? model.default_effort,
    max_llm_input_tokens: Math.min(Math.max(draft.max_llm_input_tokens, model.min_input), model.max_input, model.context_window - output),
    max_llm_output_tokens: output,
  };
  for (const [key, value] of Object.entries(suggestions)) {
    if (requirements.field_sources[key] !== "user") {
      Object.assign(next, { [key]: value }); requirements.field_sources[key] = "template";
    }
  }
  for (const [key, value] of Object.entries({ provider: model.provider, allowed_reasoning: model.efforts, adaptive_reasoning: model.default_effort !== "none" })) {
    if (requirements.field_sources[key] !== "user") {
      Object.assign(requirements, { [key]: value }); requirements.field_sources[key] = "template";
    }
  }
  return next;
}

const endpointProtocols = ["openai-compatible", "openai-responses", "anthropic"];
export function endpointProtocolOptions(draft: ModelEndpointDraft, model?: CatalogModel): { protocol: string; disabled: boolean }[] {
  const supported = model
    ? model.protocols.filter(p => !p.disabled_reason).map(p => p.protocol)
    : endpointProtocols.filter(p => draft.requirements?.provider === "zhipu" ? p === "openai-compatible"
      : draft.requirements?.provider === "openai" ? p !== "anthropic" : true);
  const options = supported.map(protocol => ({ protocol, disabled: false }));
  // Preserve an invalid persisted/manual value visibly; never silently repair it.
  if (!supported.includes(draft.api_protocol)) options.push({ protocol: draft.api_protocol, disabled: true });
  return options;
}
