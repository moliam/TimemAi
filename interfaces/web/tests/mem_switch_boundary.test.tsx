import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import {
  MemSwitchConfirmDialog,
  shellQuoteCommandArgument,
} from "../src/mem_switch_confirm_dialog";

const componentSource = readFileSync(
  new URL("../src/mem_switch_confirm_dialog.tsx", import.meta.url),
  "utf8",
);
const mainSource = readFileSync(
  new URL("../src/main.tsx", import.meta.url),
  "utf8",
);

function render(pending = false) {
  return renderToStaticMarkup(
    createElement(MemSwitchConfirmDialog, {
      candidate: { path: "/tmp/team's mem", runningSessionCount: 2 },
      pending,
      onClose: () => undefined,
      onConfirm: () => undefined,
    }),
  );
}

describe("MEM switch execution boundary", () => {
  it("renders the hard boundary and non-destructive separate-Runtime alternative", () => {
    const html = render();
    expect(html).toContain(
      "Switching MEM will stop all running and queued work in the current MEM.",
    );
    expect(html).toContain("2 affected Sessions will be marked interrupted.");
    expect(html).toContain(
      "Nothing from the current MEM will continue or restart automatically.",
    );
    expect(html).toContain("To keep the current work running");
    expect(html).toContain(
      "timem --space &#x27;/tmp/team&#x27;&quot;&#x27;&quot;&#x27;s mem&#x27;",
    );
  });

  it("locks every dismissal and confirmation action while switching", () => {
    const html = render(true);
    expect(html).toContain(
      'aria-describedby="mem-switch-confirm-description mem-switch-confirm-status"',
    );
    expect(html).toContain('role="status"');
    expect(html).toContain("Stopping current MEM workers and switching…");
    expect((html.match(/disabled=""/g) ?? []).length).toBe(3);
  });

  it("quotes shell arguments without allowing apostrophes to escape", () => {
    expect(shellQuoteCommandArgument("/tmp/plain mem")).toBe("'/tmp/plain mem'");
    expect(shellQuoteCommandArgument("/tmp/team's mem")).toBe(
      "'/tmp/team'\"'\"'s mem'",
    );
  });

  it("keeps Session inspection and switch delivery in the parent composition", () => {
    expect(mainSource).toContain(
      'from "./mem_switch_confirm_dialog"',
    );
    expect(mainSource).toContain("function memSwitchRunningSessionCount");
    expect(mainSource).toContain("const [pendingMemSwitch");
    expect(mainSource).toMatch(
      /<MemSwitchConfirmDialog[\s\S]*candidate=\{memSwitchCandidate\}[\s\S]*onConfirm=\{\(\) => \{[\s\S]*setPendingMemSwitch\(true\)[\s\S]*type: "mem_switch"[\s\S]*stop_running: true/,
    );
    expect(componentSource).not.toContain("WebSocket");
    expect(componentSource).not.toContain("useState");
    expect(componentSource).not.toContain("Session[]");
  });
});
