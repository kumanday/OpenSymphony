/**
 * Browser app entrypoint for Vite.
 *
 * The browser client mounts the shared OpenSymphony app shell and talks to the
 * gateway through the baseline HTTP/SSE transport. It intentionally keeps
 * desktop/Tauri APIs out of the browser bundle. For hosted gateways it wires
 * a `HostedAuthClient`-backed auth integration so the shell can sign a user
 * in, persist the session token across reloads, and sign out (COE-420).
 */

import { HttpGatewayTransport } from "@opensymphony/api-client";
import { renderOpenSymphonyApp } from "@opensymphony/ui-core";
import {
  createWebAuthIntegration,
  readStoredToken,
} from "./auth-integration.js";
import { createWebAppConfig } from "./config.js";
import { createWebProfileController, defaultWebGatewayUrl } from "./profile-controller.js";

const config = createWebAppConfig();
const root = document.getElementById("root");
const defaultGatewayUrl = config.gatewayUrl || defaultWebGatewayUrl();

export function createWebTransport(gatewayUrl = defaultGatewayUrl, authToken?: string) {
  return new HttpGatewayTransport({
    baseUri: gatewayUrl,
    transport: "loopback_http",
    authToken,
  });
}

if (root) {
  const initialToken = readStoredToken();
  const authIntegration = createWebAuthIntegration(defaultGatewayUrl, {
    buildTransport: createWebTransport,
  });
  renderOpenSymphonyApp({
    root,
    mode: "web",
    title: "OpenSymphony Web",
    transport: createWebTransport(defaultGatewayUrl, initialToken),
    profileController: createWebProfileController({ defaultGatewayUrl }),
    // Keep the auth client's base URI in sync with the transport so
    // login/logout/session target the same gateway as reads.
    onGatewayUrlChanged: async (gatewayUrl) => {
      authIntegration.setGatewayUrl(gatewayUrl);
      return createWebTransport(gatewayUrl, readStoredToken());
    },
    authIntegration,
  });
}

export { config as webConfig };
