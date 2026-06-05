import { defineConfig } from "vite";

const appCssEntryId = "virtual:swimmers-app-css.css";
const resolvedAppCssEntryId = `\0${appCssEntryId}`;
const appCssParts = [
  "src/web/app.css",
  "src/web/app_trogdor.css",
  "src/web/app_sheets.css",
  "src/web/app_create_console.css",
  "src/web/app_sheet_results.css",
  "src/web/app_mobile.css",
  "src/web/app_reduced_motion.css",
  "src/web/app_scrollbar.css",
];

export default defineConfig({
  appType: "custom",
  publicDir: false,
  plugins: [
    {
      name: "swimmers-app-css-entry",
      resolveId(id) {
        return id === appCssEntryId ? resolvedAppCssEntryId : null;
      },
      load(id) {
        if (id !== resolvedAppCssEntryId) {
          return null;
        }
        return appCssParts.map((path) => `@import "/${path}";`).join("\n");
      },
    },
  ],
  build: {
    manifest: true,
    outDir: "target/web-vite",
    emptyOutDir: true,
    sourcemap: false,
    target: "es2022",
    rollupOptions: {
      input: {
        app: "src/web/app.js",
        appCss: appCssEntryId,
      },
      output: {
        entryFileNames: "assets/[name]-[hash].js",
        chunkFileNames: "assets/[name]-[hash].js",
        assetFileNames: "assets/[name]-[hash][extname]",
        manualChunks(id) {
          const normalized = id.replace(/\\/g, "/");
          if (normalized.includes("/src/web/trogdor_")) {
            return "trogdor";
          }
          if (
            normalized.endsWith("/src/web/input_support.js") ||
            normalized.endsWith("/src/web/rendered_surface.js") ||
            normalized.endsWith("/src/web/surface_model.js")
          ) {
            return "surface";
          }
          return null;
        },
      },
    },
  },
});
