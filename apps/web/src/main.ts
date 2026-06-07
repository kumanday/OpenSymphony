/**
 * Browser app entrypoint for Vite.
 *
 * The browser client mounts the shared OpenSymphony app shell and talks to the
 * gateway through the baseline HTTP/SSE transport. It intentionally keeps
 * desktop/Tauri APIs out of the browser bundle.
 */

import { HttpGatewayTransport } from "@opensymphony/api-client";
import { renderOpenSymphonyApp } from "@opensymphony/ui-core";
import { createWebAppConfig } from "./config.js";

const config = createWebAppConfig();
const root = document.getElementById("root");

export function createWebTransport() {
  return new HttpGatewayTransport({
    baseUri: config.gatewayUrl,
    transport: "loopback_http",
  });
}

if (root) {
  renderOpenSymphonyApp({
    root,
    mode: "web",
    title: "OpenSymphony Web",
    transport: createWebTransport(),
    onGatewayUrlChanged: async (gatewayUrl) =>
      new HttpGatewayTransport({
        baseUri: gatewayUrl,
        transport: "loopback_http",
      }),
  });
}

export { config as webConfig };
