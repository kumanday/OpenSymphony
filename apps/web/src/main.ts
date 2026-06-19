/**
 * Browser app entrypoint for Vite.
 *
 * The browser client mounts the shared OpenSymphony app shell and talks to the
 * gateway through the baseline HTTP/SSE transport. It intentionally keeps
 * desktop/Tauri APIs out of the browser bundle. For hosted gateways it wires
 * a `HostedAuthClient`-backed auth integration so the shell can sign a user
 * in, persist the session token across reloads, and sign out (COE-420).
 */

import { HttpGatewayTransport, HostedAuthClient } from "@opensymphony/api-client";
import type {
  AppAuthIntegration,
  AppLoginCredentials,
  GatewayReader,
} from "@opensymphony/ui-core";
import { renderOpenSymphonyApp } from "@opensymphony/ui-core";
import { createWebAppConfig } from "./config.js";
import { createWebProfileController, defaultWebGatewayUrl } from "./profile-controller.js";

const config = createWebAppConfig();
const root = document.getElementById("root");
const defaultGatewayUrl = config.gatewayUrl || defaultWebGatewayUrl();

const SESSION_TOKEN_KEY = "opensymphony.session_token";

function readStoredToken(): string | undefined {
  try {
    return window.localStorage.getItem(SESSION_TOKEN_KEY) ?? undefined;
  } catch {
    return undefined;
  }
}

function storeToken(token: string | undefined): void {
  try {
    if (token) {
      window.localStorage.setItem(SESSION_TOKEN_KEY, token);
    } else {
      window.localStorage.removeItem(SESSION_TOKEN_KEY);
    }
  } catch {
    // localStorage may be unavailable (private mode); auth still works in-memory.
  }
}

export function createWebTransport(gatewayUrl = defaultGatewayUrl, authToken?: string) {
  return new HttpGatewayTransport({
    baseUri: gatewayUrl,
    transport: "loopback_http",
    authToken,
  });
}

/**
 * Hosted auth integration adapter for the web shell (COE-420).
 *
 * Wraps `HostedAuthClient` (login/logout/session network calls) with a
 * `localStorage` token store and a transport factory so the shell can
 * authenticate, rebuild the transport with the session token, and sign out.
 * The token is persisted so an authenticated session survives reloads.
 */
function createWebAuthIntegration(gatewayUrl: string): AppAuthIntegration {
  const newClient = (token?: string) =>
    new HostedAuthClient({ baseUri: gatewayUrl, sessionToken: token });
  let client = newClient(readStoredToken());
  return {
    async login(credentials: AppLoginCredentials): Promise<string> {
      const response = await client.login({
        email: credentials.email,
        password: credentials.password,
        organization_slug: credentials.organizationSlug,
      });
      storeToken(response.session_token);
      return response.session_token;
    },
    async logout(): Promise<void> {
      await client.logout().catch(() => undefined);
      storeToken(undefined);
      client = newClient(undefined);
    },
    async applySession(token: string | undefined): Promise<GatewayReader> {
      client.setToken(token);
      storeToken(token);
      return createWebTransport(defaultGatewayUrl, token);
    },
  };
}

if (root) {
  const initialToken = readStoredToken();
  renderOpenSymphonyApp({
    root,
    mode: "web",
    title: "OpenSymphony Web",
    transport: createWebTransport(defaultGatewayUrl, initialToken),
    profileController: createWebProfileController({ defaultGatewayUrl }),
    onGatewayUrlChanged: async (gatewayUrl) =>
      createWebTransport(gatewayUrl, readStoredToken()),
    authIntegration: createWebAuthIntegration(defaultGatewayUrl),
  });
}

export { config as webConfig };
