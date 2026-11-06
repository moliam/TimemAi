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

export const RESOURCE_CAP_PERCENT_MIN = 1;
export const RESOURCE_CAP_PERCENT_MAX = 100;

export function parseResourceCapPercent(value: string): number | null {
  const trimmed = value.trim();
  if (!/^\d{1,3}(\.\d{1,2})?$/.test(trimmed)) return null;
  const parsed = Number(trimmed);
  if (!Number.isFinite(parsed)) return null;
  if (parsed < RESOURCE_CAP_PERCENT_MIN || parsed > RESOURCE_CAP_PERCENT_MAX) {
    return null;
  }
  return parsed;
}

export const RESOURCE_CAP_PERCENT_OPTIONS = [10, 25, 50, 70, 85, 100] as const;

export type ResourceCapability =
  | "manageable"
  | "needs-permission"
  | "unavailable"
  | "unsupported";

/// Classifies one resource-cap dimension for the settings UI:
/// - "unsupported": the host reports no usable management facility at all, so
///   the switch stays off/greyed and no operation may be offered.
/// - "unavailable": the facility exists but cannot be applied right now (for
///   example a cgroup root shared with other processes). The controls stay
///   inert, but the explanation differs from a permanent platform limitation.
/// - "needs-permission": the facility exists but root is required, so the
///   switch stays interactive and a toggle prompts for a one-shot password.
/// - "manageable": the kernel facility is present and usable now.
export function resourceCapability(
  dimension:
    | { kernel: string; escalation_required: boolean; blocker?: string | null }
    | undefined,
): ResourceCapability {
  if (!dimension) return "manageable";
  if (dimension.kernel === "unsupported") return "unsupported";
  if (dimension.kernel === "unavailable") {
    return dimension.escalation_required ? "needs-permission" : "unavailable";
  }
  return "manageable";
}

export type ResourceCapHintKind =
  | "unsupported"
  | "unavailable"
  | "shared-instance-root";

/// Picks the explanation a dimension should surface. The host reports a
/// structural blocker for limits the kernel can never apply in the current
/// layout (a session cgroup shared with other processes becomes a CPU-only
/// threaded subtree), so the UI can name the real cause and the remedy
/// instead of calling the whole facility unsupported. Permission-gated
/// dimensions return null: their switch opens the one-shot password dialog.
export function resourceCapHintKind(
  dimension:
    | { kernel: string; escalation_required: boolean; blocker?: string | null }
    | undefined,
): ResourceCapHintKind | null {
  const capability = resourceCapability(dimension);
  if (capability === "unsupported") return "unsupported";
  if (capability !== "unavailable") return null;
  return dimension?.blocker === "shared-instance-root"
    ? "shared-instance-root"
    : "unavailable";
}

/// Both non-actionable states render the same inert controls. They differ only
/// in the host-supplied explanation, so callers can share the disabled/opacity
/// handling instead of duplicating the two capability names.
export function resourceCapabilityIsInert(
  capability: ResourceCapability,
): boolean {
  return capability === "unsupported" || capability === "unavailable";
}
