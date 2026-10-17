// Browser-local persistence for unsent composer drafts. Pure presentation
// convenience: drafts are keyed by session id, hard-capped, and never replayed
// as commands. Host projections remain the only source of business state.

export const COMPOSER_DRAFTS_STORAGE_KEY = "timem-web-composer-drafts-v1";

const MAX_DRAFT_SESSIONS = 32;
const MAX_DRAFT_CHARS = 100_000;
const MAX_TOTAL_DRAFT_CHARS = 200_000;
const DRAFT_TTL_MS = 7 * 24 * 60 * 60 * 1000;

type StoredComposerDraft = { text: string; savedAtMs: number };
type ComposerDraftStore = Record<string, StoredComposerDraft>;

let lastPersistedJson: string | null = null;

function parseStore(): ComposerDraftStore {
  if (typeof window === "undefined") return {};
  try {
    const raw = window.localStorage.getItem(COMPOSER_DRAFTS_STORAGE_KEY);
    if (!raw) return {};
    const parsed: unknown = JSON.parse(raw);
    if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
      return {};
    }
    const store: ComposerDraftStore = {};
    for (const [sessionId, entry] of Object.entries(
      parsed as Record<string, unknown>,
    )) {
      if (!sessionId || typeof entry !== "object" || entry === null) continue;
      const { text, savedAtMs } = entry as { text?: unknown; savedAtMs?: unknown };
      if (typeof text !== "string" || text.length === 0 || text.length > MAX_DRAFT_CHARS) continue;
      if (typeof savedAtMs !== "number" || !Number.isFinite(savedAtMs)) continue;
      store[sessionId] = { text, savedAtMs };
    }
    return store;
  } catch {
    return {};
  }
}

/** Restores per-session draft text, dropping invalid or oversized entries. */
export function readComposerDraftTexts(): Record<string, string> {
  const texts: Record<string, string> = {};
  for (const [sessionId, entry] of Object.entries(parseStore())) {
    texts[sessionId] = entry.text;
  }
  return texts;
}

/**
 * Persists the current draft set. Entries absent from `texts` are dropped when
 * their session is known live (the draft was cleared or submitted) or when the
 * copy has aged out; otherwise they stay frozen so drafts survive reloads while
 * sessions are still restoring in the background. Hard caps bound the store:
 * most recently saved sessions first, per-draft and total character limits.
 * Fails soft when storage is unavailable.
 */
export function persistComposerDraftTexts(
  texts: Record<string, string>,
  liveSessionIds: ReadonlySet<string> | null,
  everLiveSessionIds: ReadonlySet<string> | null = null,
): void {
  if (typeof window === "undefined") return;
  const previous = parseStore();
  const now = Date.now();
  const merged: ComposerDraftStore = {};
  for (const [sessionId, entry] of Object.entries(previous)) {
    // Entries absent from the current texts are dropped once their session is
    // known (live now or seen live before, e.g. deleted). Only sessions never
    // observed in this visit stay frozen on disk: the Host restores the
    // session list newest-first in the background, so a missing id is not yet
    // proof the session is gone. TTL bounds frozen leftovers.
    const seen =
      (liveSessionIds?.has(sessionId) ?? false) ||
      (everLiveSessionIds?.has(sessionId) ?? false);
    const fresh = now - entry.savedAtMs <= DRAFT_TTL_MS;
    if (!seen && fresh) merged[sessionId] = entry;
  }
  for (const [sessionId, text] of Object.entries(texts)) {
    if (!sessionId || text.length === 0) continue;
    if (text.length > MAX_DRAFT_CHARS) {
      // A single oversized draft is not stored; keep any previous smaller copy.
      const fallback = merged[sessionId] ?? previous[sessionId];
      if (fallback) merged[sessionId] = fallback;
      continue;
    }
    const kept = previous[sessionId];
    merged[sessionId] =
      kept && kept.text === text ? kept : { text, savedAtMs: now };
  }
  const bounded: ComposerDraftStore = {};
  let totalChars = 0;
  const entries = Object.entries(merged).sort(
    (left, right) => right[1].savedAtMs - left[1].savedAtMs,
  );
  for (const [sessionId, entry] of entries) {
    if (Object.keys(bounded).length >= MAX_DRAFT_SESSIONS) break;
    if (totalChars + entry.text.length > MAX_TOTAL_DRAFT_CHARS) continue;
    bounded[sessionId] = entry;
    totalChars += entry.text.length;
  }
  const json = JSON.stringify(bounded);
  try {
    // Skip only when storage already holds this exact content: another tab
    // may have cleared it since the last local write.
    if (
      lastPersistedJson === json &&
      window.localStorage.getItem(COMPOSER_DRAFTS_STORAGE_KEY) === json
    ) {
      return;
    }
    window.localStorage.setItem(COMPOSER_DRAFTS_STORAGE_KEY, json);
    lastPersistedJson = json;
  } catch {
    /* Storage can be unavailable in private or restricted contexts. */
  }
}
