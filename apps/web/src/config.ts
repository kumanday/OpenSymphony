/**
 * Runtime configuration for the web client.
 *
 * Resolves the gateway URL from environment variables baked into the
 * build or from runtime environment variables. Supports two modes:
 *   1. Gateway-served (default): web app is served by the gateway;
 *      API calls target the same origin.
 *   2. Separately deployed: web app points to an explicit gateway URL.
 */

/** Compile-time gateway URL injected by Vite (optional). */
declare const __GATEWAY_URL__: string | undefined;

export interface WebAppConfig {
  /** Full gateway base URL for API calls. */
  gatewayUrl: string;
  /** True when the web app is served by the same gateway. */
  gatewayServed: boolean;
  /** Base path for static assets. */
  basePath: string;
}

/**
 * Resolve the gateway URL from compile-time or runtime config.
 */
function resolveGatewayUrl(): string {
  // 1. Runtime environment variable (Vite dev server).
  const envUrl = typeof import.meta !== "undefined"
    ? (import.meta.env.VITE_GATEWAY_URL as string | undefined)
    : undefined;
  if (envUrl) return envUrl;

  // 2. Compile-time define (Vite build).
  if (__GATEWAY_URL__) return __GATEWAY_URL__;

  // 3. Default: same origin (gateway-served mode).
  return "";
}

/** Create the web app configuration. */
export function createWebAppConfig(): WebAppConfig {
  const gatewayUrl = resolveGatewayUrl();
  const gatewayServed = !gatewayUrl;

  return {
    gatewayUrl: gatewayUrl,
    gatewayServed,
    basePath: (import.meta.env.VITE_APP_BASE_PATH as string) ?? "/app/",
  };
}
