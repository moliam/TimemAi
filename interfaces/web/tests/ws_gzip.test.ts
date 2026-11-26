import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";

const main = readFileSync(new URL("../src/main.tsx", import.meta.url), "utf8");
const transport = readFileSync(
  new URL("../../../bridges/http_websocket/src/transport.rs", import.meta.url),
  "utf8",
);

// 用户反馈：加载要越快越好。≥64KiB 的 wire 帧（hello/session_turns）以 GZ1\n
// 前缀的 gzip 二进制帧下发；浏览器用原生 DecompressionStream 按到达顺序解码。
describe("gzip wire frames", () => {
  it("server compresses large frames behind a codec prefix", () => {
    expect(transport).toContain("WIRE_FRAME_GZIP_THRESHOLD_BYTES: usize = 64 * 1024");
    expect(transport).toContain('b"GZ1\\n"');
    expect(transport).toContain("flate2::Compression::new(3)");
    expect(transport).toContain("Message::Binary(frame)");
  });

  it("client decodes binary frames in arrival order", () => {
    expect(main).toContain('const WIRE_FRAME_GZIP_PREFIX = "GZ1\\n";');
    expect(main).toContain('new DecompressionStream("gzip")');
    expect(main).toContain("let inboundChain: Promise<void> = Promise.resolve();");
    expect(main).toContain("inboundChain = inboundChain");
    expect(main).toContain("await parseGzipWireEvent(data as Blob | ArrayBuffer)");
  });
});
