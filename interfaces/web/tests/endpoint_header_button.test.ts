import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const source = readFileSync(new URL("../src/main.tsx", import.meta.url), "utf8");
const styles = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8");

describe("header endpoint selector", () => {
  it("renders the endpoint name and folding control inside one button", () => {
    expect(source).toMatch(
      /className={`header-model[\s\S]*<span title={headerModelLabel}>{headerModelLabel}<\/span>[\s\S]*<ChevronDown/,
    );
    expect(source).toContain("aria-expanded={showRuntime}");
  });

  it("uses a deep borderless surface with the same height as the ctx/cache readout", () => {
    expect(styles).toContain(".header-context-actions { align-self: end; }");
    expect(styles).toContain(".header-context { min-height: 28px; }");
    expect(styles).toMatch(
      /\.header-session-cluster \.header-model \{[\s\S]*height: 28px;[\s\S]*border: 0;[\s\S]*background: #244a40;[\s\S]*color: #fff;[\s\S]*box-shadow: none;/,
    );
    expect(styles).toMatch(
      /:root\[data-theme="light"\] \.header-session-cluster \.header-model \{[\s\S]*border: 0;[\s\S]*background: #315f52;[\s\S]*color: #fff;[\s\S]*box-shadow: none;/,
    );
  });
  it("moves the current session name out of the header and into the collapsed sidebar", () => {
    expect(source).not.toContain(
      '<strong title={activeSession?.display_name ?? "No session"}>',
    );
    expect(source).toContain('className="collapsed-session-card"');
    expect(source).toContain(
      'title={activeSession?.display_name ?? "No session"}',
    );
    expect(styles).toContain(
      '.sidebar.collapsed > :not(.collapsed-brand, .collapsed-session-card, .sidebar-footer)',
    );
    expect(styles).toMatch(
      /\.collapsed-session-card \{[\s\S]*min-height: 92px;[\s\S]*border: 0;[\s\S]*background: linear-gradient\(180deg, #17372f[\s\S]*box-shadow:/,
    );
    expect(styles).toMatch(
      /\.collapsed-session-card span \{[\s\S]*text-overflow: ellipsis;[\s\S]*transform: translate\(-50%, -50%\) rotate\(90deg\);/,
    );
    expect(styles).not.toContain("writing-mode: vertical-rl");
    expect(styles).toMatch(
      /@media \(max-width: 1050px\) \{[\s\S]*\.collapsed-brand,[\s\S]*\.collapsed-session-card,[\s\S]*display: none;/,
    );
  });

});
