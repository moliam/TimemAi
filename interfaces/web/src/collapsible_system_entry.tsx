import { memo, useState } from "react";
import { ChevronDown, ChevronUp } from "lucide-react";
import { useT } from "./i18n";

/**
 * System-triggered entry bodies (timer prompts, mailbox deliveries) default to
 * a single truncated line so machine input never crowds the conversation; an
 * inline toggle expands or collapses the full text.
 */
export const CollapsibleSystemEntryText = memo(function CollapsibleSystemEntryText({
  text,
}: {
  text: string;
}) {
  const t = useT();
  const [expanded, setExpanded] = useState(false);
  return (
    <div className={`system-entry-text${expanded ? " expanded" : ""}`}>
      <div className="system-entry-text-body">{text}</div>
      <button
        type="button"
        className="system-entry-toggle"
        aria-expanded={expanded}
        title={expanded ? t("systemEntry.collapse") : t("systemEntry.expand")}
        onClick={() => setExpanded((value) => !value)}
      >
        {expanded ? (
          <ChevronUp size={11} aria-hidden="true" />
        ) : (
          <ChevronDown size={11} aria-hidden="true" />
        )}
        <span>{expanded ? t("systemEntry.collapse") : t("systemEntry.expand")}</span>
      </button>
    </div>
  );
});
