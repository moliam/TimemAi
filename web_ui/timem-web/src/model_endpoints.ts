export type ModelEndpoint = {
  id: string;
  name: string;
  model: string;
  api_protocol: string;
  response_protocol: string;
  base_url: string;
  api_key_configured: boolean;
};

export type ModelEndpointDraft = {
  id?: string;
  name: string;
  model: string;
  api_protocol: string;
  response_protocol: string;
  base_url: string;
  api_key?: string;
};

type ModelEndpointProfile = {
  model: string;
  api_protocol: string;
  response_protocol: string;
  base_url: string;
  api_key_configured: boolean;
};

export function endpointMatchesProfile(endpoint: ModelEndpoint, profile: ModelEndpointProfile | undefined): boolean {
  return !!profile
    && endpoint.model === profile.model
    && endpoint.api_protocol === profile.api_protocol
    && endpoint.response_protocol === profile.response_protocol
    && endpoint.base_url === profile.base_url
    && endpoint.api_key_configured === profile.api_key_configured;
}

export function endpointNameForProfile(
  endpoints: readonly ModelEndpoint[],
  profile: ModelEndpointProfile | undefined,
): string | undefined {
  return endpoints.find((endpoint) => endpointMatchesProfile(endpoint, profile))?.name;
}

export function endpointDraftValid(draft: ModelEndpointDraft): boolean {
  return !!draft.name.trim()
    && !!draft.model.trim()
    && !!draft.api_protocol.trim()
    && !!draft.response_protocol.trim()
    && !!draft.base_url.trim();
}
