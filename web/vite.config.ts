import { cpSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import react from "@vitejs/plugin-react";
import type { Plugin } from "vite";
import { defineConfig } from "vitest/config";

/**
 * Copy the PDF.js runtime data into `dist/pdfjs/`.
 *
 * PDF.js fetches CMaps, the standard font programs, its WASM image decoders
 * and ICC profiles at render time. Left unconfigured it reaches for Mozilla's
 * CDN; these are served from our own origin instead, which is what makes the
 * `default-src 'self'` shell CSP survivable and keeps the viewer working with
 * no network at all.
 */
function pdfjsRuntimeAssets(): Plugin {
  const require = createRequire(import.meta.url);
  const packageRoot = dirname(require.resolve("pdfjs-dist/package.json"));
  return {
    name: "archon-pdfjs-runtime-assets",
    apply: "build",
    writeBundle(options) {
      const outDir = options.dir ?? "dist";
      for (const directory of ["cmaps", "standard_fonts", "wasm", "iccs"]) {
        cpSync(join(packageRoot, directory), join(outDir, "pdfjs", directory), {
          recursive: true,
        });
      }
    },
  };
}

export default defineConfig({
  base: "/static/",
  plugins: [react(), pdfjsRuntimeAssets()],
  server: {
    host: "127.0.0.1",
    port: 5173,
    proxy: {
      "/api": "http://127.0.0.1:8421",
      "/health": "http://127.0.0.1:8421",
    },
  },
  build: {
    outDir: "dist",
    emptyOutDir: true,
    sourcemap: false,
    rollupOptions: {
      output: {
        manualChunks(id) {
          const normalizedId = id.replaceAll("\\", "/");
          if (
            normalizedId.includes("/node_modules/react/")
            || normalizedId.includes("/node_modules/react-dom/")
            || normalizedId.includes("/node_modules/react-router/")
          ) {
            return "react";
          }
          if (normalizedId.includes("/node_modules/@tanstack/react-query/")) {
            return "query";
          }
          return undefined;
        },
      },
    },
  },
  test: {
    environment: "jsdom",
    globals: true,
    include: ["src/**/*.test.{ts,tsx}"],
    passWithNoTests: true,
  },
});
