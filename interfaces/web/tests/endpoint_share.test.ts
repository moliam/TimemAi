import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { endpointShareErrorMessage } from "../src/endpoint_share";
import { setLocale } from "../src/i18n";

const component = readFileSync(new URL("../src/endpoint_share.tsx", import.meta.url), "utf8");
const main = readFileSync(new URL("../src/main.tsx", import.meta.url), "utf8");
const styles = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8");

describe("endpoint share dialog", () => {
  it("maps known failures and hides unknown Host details", () => {
    setLocale("zh");
    expect(endpointShareErrorMessage("model_endpoint_share_invalid_base64"))
      .toBe("分享内容无效。");
    expect(endpointShareErrorMessage("endpoint_share_timeout"))
      .toContain("请求超时");
    expect(endpointShareErrorMessage("endpoint_share_disconnected"))
      .toContain("连接不可用");
    expect(endpointShareErrorMessage("unknown_internal_error:/private/path"))
      .toBe("暂时无法完成操作，请稍后重试。");
    expect(endpointShareErrorMessage("unknown_internal_error:/private/path"))
      .not.toContain("private/path");
    setLocale("en");
    expect(endpointShareErrorMessage("model_endpoint_share_basic_required"))
      .toContain("Basic settings are required");
    setLocale("zh");
  });

  it("renders a top-level modal with explicit progress and cancellable close paths", () => {
    expect(component).toContain("createPortal(");
    expect(component).toContain('document.body');
    expect(component).toContain('className="endpoint-share-backdrop"');
    expect(component).toContain('className="endpoint-share-dialog"');
    expect(component).toContain('role="dialog"');
    expect(component).toContain('aria-modal="true"');
    expect(component).toContain('aria-busy={busy}');
    expect(component).toContain('event.key === "Escape"');
    expect(component).toContain('onClick={close}');
    expect(component).toContain('className="endpoint-share-progress" role="status"');
    expect(component).toContain('endpoints.shareGenerating');
    expect(component).toContain('endpoints.shareImporting');
    expect(component).toMatch(/const close = \(\) => \{[\s\S]*cancel\.current\(\);[\s\S]*onClose\(\);/);
    expect(component).not.toContain("errors[result.error] ?? result.error");
  });

  it("keeps the dialog above Settings and responsive on narrow screens", () => {
    expect(styles).toMatch(/\.settings-center-backdrop \{ position: fixed; z-index: 45;/);
    expect(styles).toMatch(/\.endpoint-share-backdrop \{ position: fixed; z-index: 70;/);
    expect(styles).toContain(".endpoint-share-dialog { width: min(520px, 100%);");
    expect(styles).toMatch(/@media \(max-width: 600px\) \{[\s\S]*\.endpoint-share-backdrop \{ padding: 18px 12px;/);
  });
});

describe("endpoint share request lifecycle", () => {
  it("uses one completion path for result, rejection, timeout and disconnect", () => {
    expect(main).toContain("const finishEndpointShare = useCallback");
    expect(main).toContain('finishEndpointShare(command.request_id, { error: "endpoint_share_timeout" })');
    expect(main).toContain('finishEndpointShare(command.request_id, { error: "endpoint_share_disconnected" })');
    expect(main).toContain("finishEndpointShare(\n          event.request_id");
    expect(main).toContain("event.status === \"rejected\" && finishEndpointShare(");
    expect(main).toContain("interruptEndpointShare();");
    expect(main).toMatch(/ws\.onclose = \(\) => \{[\s\S]*interruptEndpointShare\(\);/);
  });

  it("clears timer, command correlation and pending callback exactly before delivery", () => {
    const start = main.indexOf("const finishEndpointShare = useCallback");
    const end = main.indexOf("const interruptEndpointShare", start);
    const finish = main.slice(start, end);
    expect(finish).toContain("window.clearTimeout(pending.timer)");
    expect(finish).toContain("sentCommandsRef.current.delete(requestId)");
    expect(finish).toContain("endpointSharePending.current = null");
    expect(finish.indexOf("endpointSharePending.current = null"))
      .toBeLessThan(finish.indexOf("pending.receive(result)"));
  });

  it("cancels without delivering a late result when the dialog closes", () => {
    const start = main.indexOf("const cancelEndpointShare = useCallback");
    const end = main.indexOf("const finishEndpointShare", start);
    const cancel = main.slice(start, end);
    expect(cancel).toContain("window.clearTimeout(pending.timer)");
    expect(cancel).toContain("sentCommandsRef.current.delete(pending.id)");
    expect(cancel).toContain("endpointSharePending.current = null");
    expect(cancel).not.toContain("pending.receive");
  });
});
