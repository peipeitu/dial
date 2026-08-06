import { resolve } from "node:path";
import { defineConfig } from "vite";
import vue from "@vitejs/plugin-vue";

const rendererRoot = resolve(import.meta.dirname, "src/renderer");

export default defineConfig({
  root: rendererRoot,
  base: "./",
  plugins: [vue()],
  clearScreen: false,
  server: {
    host: "127.0.0.1",
    port: 1420,
    strictPort: true
  },
  build: {
    outDir: resolve(import.meta.dirname, "dist"),
    emptyOutDir: true,
    rollupOptions: {
      input: {
        main: resolve(rendererRoot, "index.html"),
        tray: resolve(rendererRoot, "tray.html")
      }
    }
  }
});
