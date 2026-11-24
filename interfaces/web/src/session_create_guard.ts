import { t } from "./i18n";

// Session creation must always end in a definite state. The host rejects an
// invalid workspace synchronously (non-existent directory, relative path), and
// a lost reply must not leave the dialog spinning: while `creating` is set the
// close button and backdrop are disabled, so a lost rejection froze the whole
// new-session dialog.
export const SESSION_CREATE_CONFIRM_TIMEOUT_MS = 10_000;

export type SessionCreateFailure = { title: string; detail: string };

/// Maps host rejection codes to a user-facing explanation. Unknown codes are
/// surfaced verbatim so diagnostics are never swallowed.
export function sessionCreateFailure(error: string | undefined): SessionCreateFailure {
  const code = error?.trim() ?? "";
  const detail = code.startsWith("workspace_not_found")
    ? t("sessions.createErrorWorkspaceNotFound")
    : code.startsWith("workspace_must_be_absolute")
      ? t("sessions.createErrorWorkspaceMustBeAbsolute")
      : code.startsWith("workspace_not_directory")
        ? t("sessions.createErrorWorkspaceNotDirectory")
        : t("sessions.createErrorGeneric", { error: code || "unknown" });
  return { title: t("sessions.createFailedTitle"), detail };
}

/// The host never answered the create command (dead transport): give the user
/// a definite failure instead of an endless "Creating session…".
export function sessionCreateUnconfirmed(): SessionCreateFailure {
  return {
    title: t("sessions.createUnconfirmedTitle"),
    detail: t("sessions.createUnconfirmedDetail"),
  };
}
