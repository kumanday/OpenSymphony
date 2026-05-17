/**
 * Web app shell entrypoint (stub).
 *
 * This module defines the adapter interface for the browser client.
 * Uses authenticated HTTPS/WSS transport to the gateway.
 */

import type { GatewayTransport } from "@opensymphony/api-client";

/**
 * Browser transport adapter using WebSocket or SSE.
 */
export interface BrowserTransportAdapter extends GatewayTransport {
  /** Authenticate and establish the transport. */
  connect(token: string): Promise<void>;
}

export function createWebTransport(): BrowserTransportAdapter {
  throw new Error("Browser transport adapter not yet implemented");
}