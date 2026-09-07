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
  100_000, 200_000, 1_000_000,
] as const;
export const MODEL_OUTPUT_TOKEN_OPTIONS = [10_000, 20_000, 50_000] as const;

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
    MODEL_CONTEXT_WINDOW_OPTIONS.includes(
      draft.max_llm_input_tokens as (typeof MODEL_CONTEXT_WINDOW_OPTIONS)[number],
    ) &&
    MODEL_OUTPUT_TOKEN_OPTIONS.includes(
      draft.max_llm_output_tokens as (typeof MODEL_OUTPUT_TOKEN_OPTIONS)[number],
    )
  );
}
