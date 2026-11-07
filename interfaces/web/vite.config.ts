import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  build: {
    outDir: "dist",
    emptyOutDir: true,
    // The heavy markdown enhancement chunk (highlight.js + KaTeX) is
    // content-gated: MarkdownContent requests it only when the rendered
    // text actually contains code fences or math. Static modulepreload would
    // download the whole chunk on every first paint and defeat that gate.
    modulePreload: {
      resolveDependencies(_filename, deps) {
        return deps.filter(
          (dep) => !/[\\/](markdown_plugins|highlighting|math)-[^\\/]+\.js$/.test(dep),
        );
      },
    },
    rollupOptions: {
      output: {
        // The enhancement plugins load through the dynamic `markdown_plugins`
        // import. Keep them out of manualChunks and let rollup group the whole
        // dynamically imported subtree into its own chunk: forcing the
        // unified-ecosystem packages into separate `highlighting`/`math`
        // chunks while `remark-math` rides with the static `markdown` chunk
        // created a circular chunk dependency (markdown -> math -> markdown),
        // whose execution order crashed at page load with
        // `ReferenceError: Cannot access 'Ta' before initialization` and left
        // the app stuck on the loading screen. Rollup's default grouping
        // hoists every statically-and-dynamically shared module into the
        // static ancestor chunk, which keeps the chunk graph acyclic.
        manualChunks: {
          assistantUi: ["@assistant-ui/react"],
          dndKit: [
            "@dnd-kit/core",
            "@dnd-kit/sortable",
            "@dnd-kit/utilities",
          ],
          icons: ["lucide-react"],
          markdown: ["react-markdown", "remark-gfm", "remark-math"],
        },
      },
    },
  },
});
