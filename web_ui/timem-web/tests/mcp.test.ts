import { describe, expect, it } from "vitest";
import { createMcpTransportDrafts, maskSensitiveMcpValues, mcpTransportLabel, mergeMcpSecrets } from "../src/mcp";

describe("MCP editor transport drafts", () => {
  it("keeps independent local, Streamable HTTP, and SSE values across selection changes", () => {
    const initial = createMcpTransportDrafts({
      type: "stdio",
      command: "npx",
      args: ["-y", "server-package"],
      env: { LOCAL_KEY: "local-value" },
    });
    const edited = {
      ...initial,
      streamable_http: {
        ...initial.streamable_http,
        url: "https://example.test/mcp",
        headers: { Authorization: "Bearer ${MCP_TOKEN}" },
      },
      sse: {
        ...initial.sse,
        url: "https://legacy.test/sse",
        headers: { "X-Client": "timem" },
      },
    };

    expect(edited.stdio.command).toBe("npx");
    expect(edited.stdio.args).toEqual(["-y", "server-package"]);
    expect(edited.streamable_http.url).toBe("https://example.test/mcp");
    expect(edited.sse.url).toBe("https://legacy.test/sse");
  });

  it("names every supported transport explicitly", () => {
    expect(mcpTransportLabel({ type: "stdio" })).toBe("Local stdio");
    expect(mcpTransportLabel({ type: "streamable_http" })).toBe("Streamable HTTP");
    expect(mcpTransportLabel({ type: "sse" })).toBe("Legacy SSE");
  });
});

describe("MCP secret presentation", () => {
  it("masks sensitive headers without hiding ordinary headers", () => {
    expect(maskSensitiveMcpValues({ Authorization: "Bearer secret", Accept: "application/json" })).toEqual({
      Authorization: "****",
      Accept: "application/json",
    });
  });

  it("merges only explicitly revealed values into the editor draft", () => {
    expect(mergeMcpSecrets({ Authorization: "****", Accept: "application/json" }, { Authorization: "Bearer secret" })).toEqual({
      Authorization: "Bearer secret",
      Accept: "application/json",
    });
  });
});
