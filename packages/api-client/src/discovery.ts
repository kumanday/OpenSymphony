/**
 * Gateway discovery and health probing.
 *
 * Provides functions to probe default loopback gateway endpoints
 * and validate gateway capabilities.
 */

import type { GatewayCapabilities } from "@opensymphony/gateway-schema";

/** Result of a gateway discovery probe. */
export interface DiscoveryResult {
  /** Whether the gateway responded to the health check. */
  healthy: boolean;
  /** Gateway capabilities if the probe succeeded. */
  capabilities: GatewayCapabilities | null;
  /** URL that was probed. */
  probedUrl: string;
  /** Error message if the probe failed. */
  error: string | null;
  /** Time taken for the probe in milliseconds. */
  latencyMs: number;
  /** Whether the gateway API version is compatible. */
  compatible: boolean;
}

/** Default gateway URL to probe for local discovery. */
export const DEFAULT_GATEWAY_URL = "http://127.0.0.1:8080";

/** Minimum compatible API version. */
export const MIN_COMPATIBLE_API_VERSION = "v1";

/**
 * Probe the gateway health endpoint.
 *
 * Calls GET /healthz and returns whether the gateway is responsive.
 */
export async function probeHealth(baseUrl: string): Promise<{ healthy: boolean; error: string | null }> {
  const url = `${baseUrl.replace(/\/+$/, "")}/healthz`;
  const start = Date.now();
  try {
    const response = await fetch(url, { method: "GET", signal: AbortSignal.timeout(5000) });
    const latencyMs = Date.now() - start;
    if (!response.ok) {
      return { healthy: false, error: `HTTP ${response.status} after ${latencyMs}ms` };
    }
    const body = await response.json();
    if (body.status !== "ok" && body.status !== "healthy") {
      return { healthy: false, error: `Unexpected health status: ${body.status}` };
    }
    return { healthy: true, error: null };
  } catch (err) {
    const latencyMs = Date.now() - start;
    return {
      healthy: false,
      error: err instanceof Error ? err.message : String(err),
    };
  }
}

/**
 * Probe the gateway capabilities endpoint.
 *
 * Calls GET /api/v1/capabilities and returns the gateway capabilities
 * or an error if the endpoint is unreachable or returns an data.
 */
export async function probeCapabilities(baseUrl: string): Promise<GatewayCapabilities | null> {
  const url = `${baseUrl.replace(/\/+$/, "")}/api/v1/capabilities`;
  try {
    const response = await fetch(url, { method: "GET", signal: AbortSignal.timeout(5000) });
    if (!response.ok) {
      return null;
    }
    const body = await response.json();
    return body as GatewayCapabilities;
  } catch {
    return null;
  }
}

/**
 * Discover and validate a gateway at the given URL.
 *
 * Probes /healthz and /api/v1/capabilities, then returns a structured
 * result with health status, capabilities, and compatibility info.
 */
export async function discoverGateway(baseUrl: string = DEFAULT_GATEWAY_URL): Promise<DiscoveryResult> {
  const start = Date.now();
  
  // Probe health
  const healthResult = await probeHealth(baseUrl);
  const healthLatency = Date.now() - start;
  
  if (!healthResult.healthy) {
    return {
      healthy: false,
      capabilities: null,
      probedUrl: baseUrl,
      error: healthResult.error,
      latencyMs: healthLatency,
      compatible: false,
    };
  }
  
  // Probe capabilities
  const capabilities = await probeCapabilities(baseUrl);
  const totalLatency = Date.now() - start;
  
  if (!capabilities) {
    return {
      healthy: true,
      capabilities: null,
      probedUrl: baseUrl,
      error: "Capabilities endpoint unreachable",
      latencyMs: totalLatency,
      compatible: false,
    };
  }
  
  // Check API version compatibility
  const compatible = (capabilities.supported_api_versions ?? []).some(
    (version) => version === MIN_COMPATIBLE_API_VERSION || version.startsWith(`${MIN_COMPATIBLE_API_VERSION}.`),
  );
  
  return {
    healthy: true,
    capabilities,
    probedUrl: baseUrl,
    error: null,
    latencyMs: totalLatency,
    compatible,
  };
}

/**
 * Validate that a gateway URL is reachable and returns a compatible API version.
 *
 * Returns true if the gateway is healthy and compatible, false otherwise.
 */
export async function validateGateway(baseUrl: string): Promise<boolean> {
  const result = await discoverGateway(baseUrl);
  return result.healthy && result.compatible;
}

/**
 * Discover gateway with fallback URLs.
 *
 * Tries each URL in order until one responds with a healthy, compatible gateway.
 */
export async function discoverGatewayWithFallback(
  urls: string[] = [DEFAULT_GATEWAY_URL, "http://localhost:8080"],
): Promise<DiscoveryResult> {
  for (const url of urls) {
    const result = await discoverGateway(url);
    if (result.healthy && result.compatible) {
      return result;
    }
  }
  
  // Return the last result if none succeeded
  return discoverGateway(urls[urls.length - 1]);
}
