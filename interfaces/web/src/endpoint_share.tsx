import { useEffect, useId, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { Copy, LoaderCircle, TriangleAlert, X } from "lucide-react";
import { t, useT } from "./i18n";
import type { ClientCommand } from "./protocol";

export type ShareCommand = Extract<ClientCommand, { type: "model_endpoint_share_export" | "model_endpoint_share_import" }>;
export type ShareResult = { data?: string; name?: string; error?: string };
export type ShareTransport = (command: ShareCommand, receive: (result: ShareResult) => void) => () => void;

export function endpointShareErrorMessage(error: string): string {
  switch (error.split(":", 1)[0]) {
    case "model_endpoint_share_invalid_base64": return t("endpoints.shareInvalidBase64");
    case "model_endpoint_share_invalid_format": return t("endpoints.shareInvalidFormat");
    case "model_endpoint_share_unsupported_version": return t("endpoints.shareUnsupportedVersion");
    case "model_endpoint_share_basic_required": return t("endpoints.shareBasicRequired");
    case "model_endpoint_share_invalid_config": return t("endpoints.shareInvalidConfig");
    case "model_endpoint_share_too_large": return t("endpoints.shareTooLarge");
    case "endpoint_share_timeout": return t("endpoints.shareTimeout");
    case "endpoint_share_disconnected": return t("endpoints.shareDisconnected");
    default: return t("endpoints.shareRetry");
  }
}

export function EndpointSharePanel({ endpoint, transport, onClose }: {
  endpoint?: { id: string; name: string };
  transport: ShareTransport;
  onClose: () => void;
}) {
  useT();
  const titleId = useId();
  const warningId = useId();
  const [basic, setBasic] = useState(true);
  const [advanced, setAdvanced] = useState(false);
  const [personal, setPersonal] = useState(false);
  const [data, setData] = useState("");
  const [message, setMessage] = useState("");
  const [failed, setFailed] = useState(false);
  const [busy, setBusy] = useState(false);
  const cancel = useRef<() => void>(() => {});
  useEffect(() => () => cancel.current(), []);
  const close = () => {
    cancel.current();
    cancel.current = () => {};
    onClose();
  };
  const invalidate = () => {
    cancel.current();
    cancel.current = () => {};
    setBusy(false);
    setFailed(false);
    setData("");
    setMessage("");
  };
  const submit = () => {
    cancel.current();
    setBusy(true);
    setFailed(false);
    setMessage("");
    const request_id = crypto.randomUUID();
    cancel.current = transport(endpoint ? {
      type: "model_endpoint_share_export", request_id, endpoint_id: endpoint.id,
      basic, advanced, personal,
    } : { type: "model_endpoint_share_import", request_id, data }, result => {
      cancel.current = () => {};
      setBusy(false);
      setFailed(!!result.error);
      if (result.error) {
        setMessage(`${t("endpoints.shareFailed")} ${endpointShareErrorMessage(result.error)}`);
      } else if (result.data !== undefined) {
        setData(result.data);
      } else if (result.name !== undefined) {
        setData("");
        setMessage(t("endpoints.shareImported", { name: result.name }));
      }
    });
  };
  if (typeof document === "undefined") return null;
  const workingLabel = t(endpoint ? "endpoints.shareGenerating" : "endpoints.shareImporting");
  return createPortal(
    <div className="endpoint-share-backdrop" role="presentation" onClick={close}>
      <section
        className="endpoint-share-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        aria-describedby={warningId}
        aria-busy={busy}
        tabIndex={-1}
        onClick={event => event.stopPropagation()}
        onKeyDown={event => {
          if (event.key === "Escape") {
            event.preventDefault();
            close();
          }
        }}
      >
        <div className="endpoint-share-heading">
          <strong id={titleId}>{t(endpoint ? "endpoints.shareExport" : "endpoints.shareImport")}{endpoint ? ` · ${endpoint.name}` : ""}</strong>
          <button type="button" className="icon-button" onClick={close} aria-label={t("common.close")} autoFocus><X size={16} /></button>
        </div>
        <div className="endpoint-share-panel">
          {endpoint && <div className="endpoint-share-options">
            <label><input type="checkbox" checked={basic} onChange={event => { invalidate(); setBasic(event.target.checked); }} />{t("endpoints.shareBasic")}</label>
            <label><input type="checkbox" checked={advanced} onChange={event => { invalidate(); setAdvanced(event.target.checked); }} />{t("endpoints.shareAdvanced")}</label>
            <label><input type="checkbox" checked={personal} onChange={event => { invalidate(); setPersonal(event.target.checked); }} />{t("endpoints.sharePersonal")}<TriangleAlert size={16} aria-hidden="true" /></label>
            {!basic && <small>{t("endpoints.shareBasicRequired")}</small>}
          </div>}
          <p className="endpoint-share-warning" id={warningId}><TriangleAlert size={16} aria-hidden="true" />{t("endpoints.shareWarning")}</p>
          {(!endpoint || data) && <label className="endpoint-share-data">
            {t("endpoints.shareString")}
            <textarea aria-label={t("endpoints.shareString")} rows={4} maxLength={262144} spellCheck={false} autoComplete="off" readOnly={!!endpoint || busy} value={data} onChange={event => { setData(event.target.value); setFailed(false); setMessage(""); }} />
          </label>}
          <div className="endpoint-share-actions">
            <button type="button" className={`primary compact ${busy ? "sending" : ""}`} disabled={busy || (endpoint ? !basic && !advanced && !personal : !data.trim())} onClick={submit}>
              {busy && <LoaderCircle size={14} aria-hidden="true" />}
              {busy ? workingLabel : t(endpoint ? "endpoints.shareGenerate" : "endpoints.shareImport")}
            </button>
            {endpoint && data && <button type="button" className="secondary compact" onClick={async () => {
              try { await navigator.clipboard.writeText(data); setFailed(false); setMessage(t("endpoints.shareCopied")); }
              catch { setFailed(true); setMessage(t("endpoints.shareCopyFailed")); }
            }}><Copy size={14} />{t("endpoints.shareCopy")}</button>}
          </div>
          {busy && <p className="endpoint-share-progress" role="status"><LoaderCircle size={14} aria-hidden="true" />{workingLabel}</p>}
          {message && <p className="endpoint-share-message" role={failed ? "alert" : "status"}>{message}</p>}
        </div>
      </section>
    </div>,
    document.body,
  );
}
