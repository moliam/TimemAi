import { readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

// UI 文案守卫：Interface 源码中禁止内联用户可见文案（i18n 目录除外）。
// 只检查剥离注释后的 CJK 字符；代码注释保持中文不受影响。
// 规则见 docs/web-i18n-architecture.md。

const SRC_ROOT = join(import.meta.dirname, "..", "src");
const ALLOWED_PREFIXES = [join(SRC_ROOT, "i18n")];

function* walk(dir: string): Generator<string> {
  for (const name of readdirSync(dir)) {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) yield* walk(path);
    else if (/\.(ts|tsx|css)$/.test(name)) yield path;
  }
}

function stripComments(source: string): string {
  return source
    .replace(/\/\*[\s\S]*?\*\//g, " ")
    .replace(/(^|[^:])\/\/[^\n]*/g, "$1");
}

describe("i18n source guard", () => {
  it("keeps user-visible CJK literals inside the i18n catalog only", () => {
    const offenders: string[] = [];
    for (const path of walk(SRC_ROOT)) {
      if (ALLOWED_PREFIXES.some((prefix) => path.startsWith(prefix))) continue;
      const stripped = stripComments(readFileSync(path, "utf8"));
      const match = stripped.match(/[\u4e00-\u9fff]/);
      if (match) offenders.push(path);
    }
    expect(offenders).toEqual([]);
  });
});
