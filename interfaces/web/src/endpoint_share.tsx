import { memo, useEffect, useId, useRef, useState } from "react";
import { createPortal } from "react-dom";
import {
  ClipboardPaste,
  Copy,
  KeyRound,
  LoaderCircle,
  Settings2,
  Share2,
  SlidersHorizontal,
  TriangleAlert,
  X,
} from "lucide-react";
import { t, useT } from "./i18n";
import { clientId, type ClientCommand } from "./protocol";

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

export const EndpointSharePanel = memo(function EndpointSharePanel({ endpoint, transport, onClose }: {
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
  const [exportData, setExportData] = useState("");
  const [hasImportData, setHasImportData] = useState(false);
  const [message, setMessage] = useState("");
  const [failed, setFailed] = useState(false);
  const [busy, setBusy] = useState(false);
  const importRef = useRef<HTMLTextAreaElement>(null);
  const hasImportDataRef = useRef(false);
  const cancel = useRef<() => void>(() => {});
  useEffect(() => () => cancel.current(), []);
  const close = () => {
    cancel.current();
    cancel.current = () => {};
    if (importRef.current) importRef.current.value = "";
    onClose();
  };
  const invalidateExport = () => {
    cancel.current();
    cancel.current = () => {};
    setBusy(false);
    setFailed(false);
    setExportData("");
    setMessage("");
  };
  const setImportPresence = (present: boolean) => {
    if (hasImportDataRef.current === present) return;
    hasImportDataRef.current = present;
    setHasImportData(present);
  };
  const clearImport = () => {
    if (importRef.current) importRef.current.value = "";
    setImportPresence(false);
  };
  const finish = (result: ShareResult) => {
    cancel.current = () => {};
    setBusy(false);
    setFailed(!!result.error);
    if (result.error) {
      setMessage(`${t("endpoints.shareFailed")} ${endpointShareErrorMessage(result.error)}`);
    } else if (result.data !== undefined) {
      setExportData(result.data);
    } else if (result.name !== undefined) {
      clearImport();
      setMessage(t("endpoints.shareImported", { name: result.name }));
    }
  };
  const submit = () => {
    cancel.current();
    cancel.current = () => {};
    const importData = endpoint ? "" : importRef.current?.value ?? "";
    if (!endpoint && !importData.trim()) return;
    setBusy(true);
    setFailed(false);
    setMessage("");
    try {
      const request_id = clientId("endpoint-share");
      let completedSynchronously = false;
      const stop = transport(endpoint ? {
        type: "model_endpoint_share_export", request_id, endpoint_id: endpoint.id,
        basic, advanced, personal,
      } : { type: "model_endpoint_share_import", request_id, data: importData }, result => {
        completedSynchronously = true;
        finish(result);
      });
      // A test transport or an immediate local failure may complete before it
      // returns its cancellation function. Do not resurrect a settled request.
      cancel.current = completedSynchronously ? () => {} : stop;
    } catch {
      finish({ error: "endpoint_share_client_failed" });
    }
  };
  const pasteImport = async () => {
    try {
      const clipboard = await navigator.clipboard.readText();
      if (!importRef.current) return;
      importRef.current.value = clipboard.slice(0, 262144);
      setImportPresence(/\S/.test(importRef.current.value));
      setFailed(false);
      setMessage(t("endpoints.sharePasted"));
      importRef.current.focus();
    } catch {
      setFailed(true);
      setMessage(t("endpoints.sharePasteFailed"));
    }
  };
  if (typeof document === "undefined") return null;
  const workingLabel = t(endpoint ? "endpoints.shareGenerating" : "endpoints.shareImporting");
  const optionRows = endpoint ? [
    { key: "basic", checked: basic, set: setBasic, icon: Settings2, label: t("endpoints.shareBasic") },
    { key: "advanced", checked: advanced, set: setAdvanced, icon: SlidersHorizontal, label: t("endpoints.shareAdvanced") },
    { key: "personal", checked: personal, set: setPersonal, icon: KeyRound, label: t("endpoints.sharePersonal"), sensitive: true },
  ] : [];
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
          <div className="endpoint-share-title">
            <span className="endpoint-share-title-icon"><Share2 size={17} aria-hidden="true" /></span>
            <div>
              <strong id={titleId}>{t(endpoint ? "endpoints.shareExport" : "endpoints.shareImport")}</strong>
              {endpoint && <span>{endpoint.name}</span>}
            </div>
          </div>
          <button type="button" className="icon-button" onClick={close} aria-label={t("common.close")} autoFocus><X size={16} /></button>
        </div>
        <div className="endpoint-share-panel">
          {endpoint && <div className="endpoint-share-options">
            {optionRows.map(({ key, checked, set, icon: Icon, label, sensitive }) => <label key={key} className={sensitive ? "sensitive" : ""}>
              <span className="endpoint-share-option-icon"><Icon size={15} aria-hidden="true" /></span>
              <span className="endpoint-share-option-label">{label}{sensitive && <TriangleAlert size={14} aria-hidden="true" />}</span>
              <input type="checkbox" checked={checked} onChange={event => { invalidateExport(); set(event.target.checked); }} />
            </label>)}
            {!basic && <small>{t("endpoints.shareBasicRequired")}</small>}
          </div>}
          <p className="endpoint-share-warning" id={warningId}><TriangleAlert size={16} aria-hidden="true" /><span>{t("endpoints.shareWarning")}</span></p>
          {endpoint && exportData && <div className="endpoint-share-data endpoint-share-export-data">
            <span>{t("endpoints.shareString")}</span>
            <output className="endpoint-share-code" aria-label={t("endpoints.shareString")} tabIndex={0}><code>{exportData}</code></output>
          </div>}
          {!endpoint && <label className="endpoint-share-data endpoint-share-import">
            <span>{t("endpoints.shareString")}</span>
            <textarea
              ref={importRef}
              aria-label={t("endpoints.shareString")}
              rows={5}
              wrap="soft"
              maxLength={262144}
              spellCheck={false}
              autoComplete="off"
              readOnly={busy}
              placeholder={t("endpoints.shareImportHint")}
              onInput={event => {
                setImportPresence(/\S/.test(event.currentTarget.value));
                if (message) {
                  setFailed(false);
                  setMessage("");
                }
              }}
            />
          </label>}
          {message && <p className="endpoint-share-message" role={failed ? "alert" : "status"}>{message}</p>}
          <div className="endpoint-share-actions">
            <button type="button" className={`primary compact ${busy ? "sending" : ""}`} aria-live="polite" disabled={busy || (endpoint ? !basic && !advanced && !personal : !hasImportData)} onClick={submit}>
              {busy && <LoaderCircle size={14} aria-hidden="true" />}
              {busy ? workingLabel : t(endpoint ? "endpoints.shareGenerate" : "endpoints.shareImport")}
            </button>
            {endpoint && exportData && <button type="button" className="secondary compact" onClick={async () => {
              try { await navigator.clipboard.writeText(exportData); setFailed(false); setMessage(t("endpoints.shareCopied")); }
              catch { setFailed(true); setMessage(t("endpoints.shareCopyFailed")); }
            }}><Copy size={14} />{t("endpoints.shareCopy")}</button>}
            {!endpoint && <button type="button" className="secondary compact" disabled={busy} onClick={pasteImport}><ClipboardPaste size={14} />{t("endpoints.sharePaste")}</button>}
          </div>
        </div>
      </section>
    </div>,
    document.body,
  );
});
