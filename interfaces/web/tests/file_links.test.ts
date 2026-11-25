import { describe, expect, it } from "vitest";
import { localFileTarget } from "../src/file_links";

const cwd = "/home/alice/project";

describe("localFileTarget", () => {
  it("recognizes explicit file URLs and decodes paths", () => {
    expect(localFileTarget("file:///home/alice/project/a.md", cwd)).toBe(
      "/home/alice/project/a.md",
    );
    expect(localFileTarget("file:///home/alice/project/a%20b.md", cwd)).toBe(
      "/home/alice/project/a b.md",
    );
    expect(localFileTarget("file:///C:/Users/alice/a.md", cwd)).toBe(
      "C:/Users/alice/a.md",
    );
    expect(localFileTarget("C:\\Users\\alice\\a.md", cwd)).toBe(
      "C:/Users/alice/a.md",
    );
    expect(localFileTarget("/home/alice/project/a%20b.md#L2", cwd)).toBe(
      "/home/alice/project/a b.md",
    );
    expect(localFileTarget("file://server/share/a.md", cwd)).toBeUndefined();
  });

  it("recognizes absolute workspace paths and likely files outside it", () => {
    expect(localFileTarget("/home/alice/project/README", cwd)).toBe(
      "/home/alice/project/README",
    );
    expect(localFileTarget("/home/alice/project/sub/a.md", cwd)).toBe(
      "/home/alice/project/sub/a.md",
    );
    expect(localFileTarget("/tmp/screenshot.png", cwd)).toBe(
      "/tmp/screenshot.png",
    );
    expect(localFileTarget("/home/alice/project", cwd)).toBe(
      "/home/alice/project",
    );
  });

  it("leaves app routes, external URLs and anchors alone", () => {
    expect(localFileTarget("/docs", cwd)).toBeUndefined();
    expect(localFileTarget("/settings/general", cwd)).toBeUndefined();
    expect(localFileTarget("https://example.com/a.md", cwd)).toBeUndefined();
    expect(localFileTarget("mailto:a@example.com", cwd)).toBeUndefined();
    expect(localFileTarget("#section", cwd)).toBeUndefined();
    expect(localFileTarget("//example.com/a.md", cwd)).toBeUndefined();
  });

  it("supports relative workspace paths and hidden files", () => {
    expect(localFileTarget("./src/main.ts", cwd)).toBe("./src/main.ts");
    expect(localFileTarget("../shared/a.md", cwd)).toBe("../shared/a.md");
    expect(localFileTarget("src/main.ts", cwd)).toBe("src/main.ts");
    expect(localFileTarget("src/a%20b.ts#L4", cwd)).toBe("src/a b.ts");
    expect(localFileTarget(".env", cwd)).toBe(".env");
    expect(localFileTarget("README", cwd)).toBeUndefined();
  });
});
