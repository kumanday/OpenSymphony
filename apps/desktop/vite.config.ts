import { defineConfig } from "vite";
import { resolve, dirname } from "path";
import { fileURLToPath } from "url";

const __dirname = dirname(fileURLToPath(import.meta.url));

export default defineConfig({
  root: __dirname,
  base: "./",
  build: {
    outDir: "dist",
    emptyOutDir: true,
    assetsDir: "assets",
    rollupOptions: {
      input: {
        main: resolve(__dirname, "index.html"),
      },
    },
  },
  server: {
    host: "127.0.0.1",
    port: 1420,
    strictPort: true,
  },
  resolve: {
    alias: {
      "@opensymphony/gateway-schema": resolve(
        __dirname,
        "../../packages/gateway-schema/src/index.ts",
      ),
      "@opensymphony/api-client": resolve(
        __dirname,
        "../../packages/api-client/src/index.ts",
      ),
      "@opensymphony/ui-core": resolve(
        __dirname,
        "../../packages/ui-core/src/index.ts",
      ),
      "@opensymphony/state": resolve(
        __dirname,
        "../../packages/state/src/index.ts",
      ),
    },
  },
});
