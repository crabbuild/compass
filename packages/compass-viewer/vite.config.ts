import path from "node:path";
import { fileURLToPath } from "node:url";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import { defineConfig } from "vite";

const packageDirectory = path.dirname(fileURLToPath(import.meta.url));

export default defineConfig({
  plugins: [react(), tailwindcss()],
  define: {
    "process.env.NODE_ENV": JSON.stringify("production")
  },
  resolve: {
    alias: {
      "@": path.resolve(packageDirectory, "src")
    }
  },
  build: {
    lib: {
      entry: path.resolve(packageDirectory, "src/export-entry.tsx"),
      formats: ["iife"],
      name: "CompassViewer",
      fileName: () => "graph.js"
    },
    cssCodeSplit: false,
    sourcemap: false,
    outDir: process.env.COMPASS_VIEWER_OUT_DIR ?? "dist",
    emptyOutDir: true
  },
  test: {
    environment: "jsdom",
    setupFiles: ["./src/test/setup.ts"]
  }
});
