import { defineConfig } from "vite";
import { resolve, dirname } from "path";
import { fileURLToPath } from "url";

const __dirname = dirname(fileURLToPath(import.meta.url));

/**
 * Vite configuration for the OpenSymphony web client.
 *
 * Supports two deployment modes:
 *   1. Gateway-served (default): assets are built under /app/ and served by
 *      the OpenSymphony Gateway. Used for local and external gateway modes.
 *   2. Separately deployed: assets are built with a configurable base path
 *      and point to a gateway URL via VITE_GATEWAY_URL.
 *
 * Environment variables:
 *   VITE_APP_BASE_PATH  - Base path for static assets (default: "/app/").
 *                         Set to "/" when deploying at the root of a domain.
 *   VITE_GATEWAY_URL    - Gateway base URL for API calls. When omitted the
 *                         web app defaults to the origin (gateway-served mode).
 *   VITE_DEV_GATEWAY_URL - Gateway URL for the Vite dev-server proxy.
 *                          Defaults to "http://127.0.0.1:2468".
 */
export default defineConfig({
  root: __dirname,
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
  base: process.env.VITE_APP_BASE_PATH ?? "/app/",
  server: {
    port: 5173,
    proxy: {
      "/api": {
        target: process.env.VITE_DEV_GATEWAY_URL ?? "http://127.0.0.1:2468",
        changeOrigin: true,
      },
    },
  },
  resolve: {
    alias: {
      "@opensymphony/gateway-schema": resolve(
        __dirname,
        "../../packages/gateway-schema/src/index.ts"
      ),
      "@opensymphony/api-client": resolve(
        __dirname,
        "../../packages/api-client/src/index.ts"
      ),
      "@opensymphony/ui-core": resolve(
        __dirname,
        "../../packages/ui-core/src/index.ts"
      ),
      "@opensymphony/state": resolve(
        __dirname,
        "../../packages/state/src/index.ts"
      ),
    },
  },
  define: {
    __GATEWAY_URL__: JSON.stringify(process.env.VITE_GATEWAY_URL ?? ""),
  },
});
