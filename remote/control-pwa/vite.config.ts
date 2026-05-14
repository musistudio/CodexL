import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { fileURLToPath } from "node:url";
import { defineConfig } from "vite";

const pwaDir = fileURLToPath(new URL(".", import.meta.url));
const entry = fileURLToPath(new URL("src/main.tsx", import.meta.url));

export default defineConfig({
  build: {
    cssCodeSplit: false,
    emptyOutDir: false,
    minify: true,
    outDir: pwaDir,
    rollupOptions: {
      input: entry,
      output: {
        assetFileNames: (assetInfo) => (assetInfo.name?.endsWith(".css") ? "react-app.css" : "assets/[name][extname]"),
        entryFileNames: "react-app.js",
        inlineDynamicImports: true,
      },
    },
    target: "es2020",
  },
  plugins: [react(), tailwindcss()],
  publicDir: false,
});
