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
          highlighting: ["highlight.js", "rehype-highlight"],
          markdown: ["react-markdown"],
        },
      },
    },
  },
});
