import { defineConfig } from "vite";
import preact from "@preact/preset-vite";
import { resolve } from "path";

const DEV_ALLOWED_HOSTS = [".ts.net", ".tailnet", ".local"];

export default defineConfig({
  plugins: [preact()],
  resolve: {
    alias: {
      "@": resolve(__dirname, "src"),
    },
  },
  server: {
    allowedHosts: DEV_ALLOWED_HOSTS,
    proxy: {
      "/v1/realtime": {
        target: "http://localhost:3210",
        ws: true,
      },
      "/v1": {
        target: "http://localhost:3210",
      },
    },
  },
  build: {
    outDir: resolve(__dirname, "../dist"),
    emptyOutDir: true,
  },
  publicDir: resolve(__dirname, "public"),
});
