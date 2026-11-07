// Mailbox message body: the panel clamps a mail to five rendered lines so a
// single long message cannot monopolize the whole sidebar. The toggle appears
// only when the rendered content actually overflows the clamp: the overflow is
// measured after layout and re-measured on resize while collapsed.
import { ChevronDown, ChevronUp } from "lucide-react";
import { useId, useLayoutEffect, useRef, useState } from "react";
import { t, useT } from "./i18n";

export function MailboxBody({ text }: { text: string }) {
  useT();
  const [expanded, setExpanded] = useState(false);
  const [overflowing, setOverflowing] = useState(false);
  const bodyRef = useRef<HTMLDivElement | null>(null);
  const bodyId = useId();

  useLayoutEffect(() => {
    const element = bodyRef.current;
    // While expanded the clamp is gone, so a measurement would only report
    // full height. Keep the previous result so the collapse control stays
    // reachable; collapsing re-measures the clamp before paint.
    if (!element || expanded) return;
    const measure = () => {
      setOverflowing(element.scrollHeight > element.clientHeight + 1);
    };
    measure();
    // Older engines without ResizeObserver still get the initial measurement;
    // the observer only refines it when the panel width changes.
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, [expanded, text]);

  const showToggle = overflowing || expanded;
  return (
    <>
      <div
        id={bodyId}
        ref={bodyRef}
        className={`mailbox-item-body${expanded ? "" : " clamped"}`}
      >
        {text}
      </div>
      {showToggle && (
        <button
          type="button"
          className="mailbox-item-body-toggle"
          aria-expanded={expanded}
          aria-controls={bodyId}
          title={expanded ? t("mailbox.collapseBody") : t("mailbox.expandBody")}
          onClick={() => setExpanded((value) => !value)}
        >
          {expanded ? (
            <ChevronUp size={12} aria-hidden="true" />
          ) : (
            <ChevronDown size={12} aria-hidden="true" />
          )}
          <span>{expanded ? t("mailbox.collapseBody") : t("mailbox.expandBody")}</span>
        </button>
      )}
    </>
  );
}
