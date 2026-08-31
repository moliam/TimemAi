export type RuntimeOption = { key: string; value: string };

type SessionRuntimeProfile = {
  model: string;
  api_protocol: string;
  base_url: string;
  max_llm_input_tokens: number;
  max_llm_output_tokens: number;
  max_rounds: string;
  bash_approval: string;
  work_instructions: string;
};

export function sessionRuntimeOptions(
  profile: SessionRuntimeProfile | undefined,
  available: readonly RuntimeOption[],
): RuntimeOption[] {
  if (!profile) return [];
  const values: Record<string, string> = {
    TIMEM_MODEL: profile.model,
    TIMEM_API_PROTOCOL: profile.api_protocol,
    TIMEM_BASE_URL: profile.base_url,
    TIMEM_MAX_LLM_INPUT: String(profile.max_llm_input_tokens),
    TIMEM_MAX_LLM_OUTPUT: String(profile.max_llm_output_tokens),
    TIMEM_MAX_ROUNDS: profile.max_rounds,
    TIMEM_BASH_APPROVAL: profile.bash_approval,
    TIMEM_WORK_INSTRUCTIONS: profile.work_instructions,
  };
  return available.flatMap((option) => option.key in values
    ? [{ ...option, value: values[option.key] }]
    : []);
}

export function runtimeOptionLabel(key: string): string {
  if (key === "TIMEM_MAX_ROUNDS") return "MAX STEPS";
  return key.startsWith("TIMEM_") ? key.slice("TIMEM_".length) : key;
}

export function shouldAutoRevealSessionApiKey({
  sessionId,
  configured,
  revealedApiKey,
  pending,
  requestedSessionId,
}: {
  sessionId?: string;
  configured: boolean;
  revealedApiKey?: string;
  pending: boolean;
  requestedSessionId: string;
}): boolean {
  return !!sessionId
    && configured
    && revealedApiKey === undefined
    && !pending
    && requestedSessionId !== sessionId;
}

export function updateRevealedSessionApiKeys(
  current: Record<string, string>,
  sessionId: string,
  savedApiKey?: string,
): Record<string, string> {
  if (savedApiKey !== undefined) return { ...current, [sessionId]: savedApiKey };
  if (!(sessionId in current)) return current;
  const next = { ...current };
  delete next[sessionId];
  return next;
}

export function reconcileRuntimeDrafts(
  drafts: Record<string, string>,
  options: readonly RuntimeOption[],
): Record<string, string> {
  const currentValues = new Map(options.map((option) => [option.key, option.value]));
  const next = { ...drafts };
  let changed = false;
  for (const [key, value] of Object.entries(drafts)) {
    if (currentValues.get(key) === value) {
      delete next[key];
      changed = true;
    }
  }
  return changed ? next : drafts;
}
