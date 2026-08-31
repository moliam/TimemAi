import { McpTransport } from "./protocol";

export type McpTransportDrafts = {
  stdio: Extract<McpTransport, { type: "stdio" }>;
  streamable_http: Extract<McpTransport, { type: "streamable_http" }>;
  sse: Extract<McpTransport, { type: "sse" }>;
};

export function createMcpTransportDrafts(transport: McpTransport): McpTransportDrafts {
  return {
    stdio: transport.type === "stdio" ? transport : { type: "stdio", command: "", args: [], env: {} },
    streamable_http: transport.type === "streamable_http" ? transport : { type: "streamable_http", url: "", headers: {} },
    sse: transport.type === "sse" ? transport : { type: "sse", url: "", headers: {} },
  };
}

export function mcpTransportLabel(transport: Pick<McpTransport, "type">) {
  if (transport.type === "stdio") return "Local stdio";
  if (transport.type === "streamable_http") return "Streamable HTTP";
  return "Legacy SSE";
}

export function isSensitiveMcpKey(key: string) {
  return /(?:authorization|api[_-]?key|token|secret|password|credential|bearer)/i.test(key);
}

export function maskSensitiveMcpValues(values: Record<string, string>) {
  return Object.fromEntries(Object.entries(values).map(([key, value]) => [key, isSensitiveMcpKey(key) ? "****" : value]));
}

export function mergeMcpSecrets(values: Record<string, string>, secrets: Record<string, string>) {
  return Object.fromEntries(Object.entries(values).map(([key, value]) => [key, secrets[key] ?? value]));
}
