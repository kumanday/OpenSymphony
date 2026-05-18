/**
 * Desktop app shell entrypoint (stub).
 *
 * This module defines the adapter interface that bridges the Tauri
 * backend (Rust) to the shared gateway transport layer. The actual
 * Tauri commands and channels will be implemented in a future ticket.
 */

import type { GatewayTransport } from "@opensymphony/api-client";

/**
 * Tauri-specific transport adapter that uses local channels or
 * loopback HTTP to talk to the OpenSymphony gateway.
 */
export interface TauriTransportAdapter extends GatewayTransport {
  /** Attach to the running local daemon via Tauri IPC. */
  attach(): Promise<void>;
}

export function createDesktopTransport(): TauriTransportAdapter {
  throw new Error("Tauri transport adapter not yet implemented");
}