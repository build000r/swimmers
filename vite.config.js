import { defineConfig } from "vite";

export default defineConfig({
  appType: "custom",
  publicDir: false,
  build: {
    manifest: true,
    outDir: "target/web-vite",
    emptyOutDir: true,
    sourcemap: false,
    target: "es2022",
    rollupOptions: {
      input: {
        app: "src/web/app.js",
      },
      output: {
        entryFileNames: "assets/[name]-[hash].js",
        chunkFileNames: "assets/[name]-[hash].js",
        assetFileNames: "assets/[name]-[hash][extname]",
      },
    },
  },
});
