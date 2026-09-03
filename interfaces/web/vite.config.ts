import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  build: {
    outDir: "dist",
    emptyOutDir: true,
    rollupOptions: {
      output: {
        manualChunks: {
          assistantUi: ["@assistant-ui/react"],
          dndKit: [
            "@dnd-kit/core",
            "@dnd-kit/sortable",
            "@dnd-kit/utilities",
          ],
          icons: ["lucide-react"],
          highlighting: ["highlight.js", "rehype-highlight"],
          markdown: ["react-markdown", "remark-gfm"],
          math: ["remark-math", "rehype-katex", "katex"],
        },
      },
    },
  },
});
