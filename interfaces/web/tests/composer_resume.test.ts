import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const source = readFileSync(new URL("../src/main.tsx", import.meta.url), "utf8");

describe("composer direct resume", () => {
  it("shows state-specific Chinese guidance", () => {
    expect(source).toContain("在 Timem思考时继续输入补充对话...");
    expect(source).toContain("输入问题，或按发送直接继续...");
  });

  it("requires confirmation before an idle empty submit and sends explicit intent", () => {
    expect(source).toContain('window.confirm("未输入内容，是否让Timem强制继续")');
    expect(source).toMatch(
      /activeSession\.state !== "working" && !draft\.trim\(\)[\s\S]*onSendForSession\([\s\S]*clientId\("resume"\)[\s\S]*true,\s*true,/,
    );
    expect(source).not.toMatch(
      /className="send-button"[\s\S]{0,500}!hasDraftText/,
    );
  });

  it("keeps the internal resume instruction out of user message bubbles", () => {
    expect(source.match(/entry\.kind !== "resume_directly"/g)?.length).toBeGreaterThanOrEqual(2);
  });
});
