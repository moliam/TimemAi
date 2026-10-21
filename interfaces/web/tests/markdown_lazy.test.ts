import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

// Source contract: the several-hundred-KB highlight.js/KaTeX enhancement and
// their vendor CSS must travel in the lazy chunk and stay off the first-paint
// critical path; the loading shell must be themed and motion-safe.
const mainSource = readFileSync(new URL("../src/main.tsx", import.meta.url), "utf8");
const pluginsSource = readFileSync(new URL("../src/markdown_plugins.ts", import.meta.url), "utf8");
const renderSource = readFileSync(new URL("../src/markdown_render.tsx", import.meta.url), "utf8");
const html = readFileSync(new URL("../index.html", import.meta.url), "utf8");
const styles = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8");
const viteConfig = readFileSync(new URL("../vite.config.ts", import.meta.url), "utf8");

describe("lazy heavy markdown enhancement", () => {
  it("keeps vendor CSS out of the first-paint bundle", () => {
    expect(mainSource).not.toContain('import "highlight.js/styles/github-dark.css"');
    expect(mainSource).not.toContain('import "katex/dist/katex.min.css"');
    expect(pluginsSource).toContain('import "highlight.js/styles/github-dark.css";');
    expect(pluginsSource).toContain('import "katex/dist/katex.min.css";');
  });

  it("gates the chunk request and the render pipeline by content", () => {
    expect(renderSource).toContain("markdownNeedsHeavyEnhancement(normalized)");
    expect(renderSource).toMatch(/const rehypePlugins = \(\(needsHeavy \? heavyPlugins : null\) \?\? \[\]\)/);
    expect(renderSource).toMatch(/if \(!needsHeavy\) return;/);
  });

  it("keeps the enhancement chunk off the static preload list", () => {
    // Static modulepreload would download the enhancement chunk on every
    // first paint and defeat the content gate.
    expect(viteConfig).toContain("resolveDependencies(_filename, deps)");
    expect(viteConfig).toContain("(markdown_plugins|highlighting|math)-");
  });

  it("does not split the lazy enhancement subtree into manual chunks", () => {
    // remark-math is statically imported by renderers and rides with the
    // static markdown chunk. Forcing highlight.js/KaTeX into separate
    // manualChunks while markdown_plugins loads them dynamically split shared
    // unified-ecosystem modules across the static and lazy chunks, creating
    // a circular chunk dependency (markdown -> math -> markdown) that
    // crashed page load with `ReferenceError: Cannot access 'Ta' before
    // initialization` and left the app stuck on the loading screen. Rollup's
    // default grouping keeps the whole dynamic subtree in one acyclic chunk.
    expect(viteConfig).toContain('markdown: ["react-markdown", "remark-gfm", "remark-math"]');
    expect(viteConfig).not.toMatch(/^\s*(highlighting|math):/m);
  });

  it("serves a themed, motion-safe first-paint shell inline", () => {
    expect(html).toContain('class="app-loading"');
    expect(html).toContain('role="status"');
    expect(html).toContain("prefers-reduced-motion: reduce");
    expect(html).toContain("prefers-color-scheme: light");
    expect(html).toContain(':root[data-theme=light] .app-loading');
    // The old unthemed shell block must not come back through styles.css.
    expect(styles).not.toMatch(/\.app-loading \{[^}]*#f6f8fa/);
  });
});
