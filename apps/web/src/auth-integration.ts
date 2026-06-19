/**
 * Hosted auth integration for the web shell (COE-420).
 *
 * Wraps `HostedAuthClient` (login/logout/session network calls) with a
 * `localStorage` token store and a transport factory so the shell can
 * authenticate, rebuild the transport with the session token, and sign out.
 * The token is persisted so an authenticated session survives reloads.
 *
 * The integration tracks the active gateway URL so the auth client stays in
 * sync with the transport: when the shell switches profiles
 * (`onGatewayUrlChanged`), `setGatewayUrl` rebuilds the client against the new
 * base URI (preserving the current session token), and `applySession` builds
 * the transport from the current URL rather than a captured default. Otherwise
 * login/logout/session would keep targeting the original gateway while reads
 * moved to the new one.
 *
 * This module is side-effect free (no DOM mounting) so it can be unit-tested
 * in isolation; `main.ts` wires it into the app shell.
 */

import { HostedAuthClient } from "@opensymphony/api-client";
import type {
  AppAuthIntegration,
  AppLoginCredentials,
  GatewayReader,
} from "@opensymphony/ui-core";

/**
 * Session token storage key for the hosted web client.
 *
 * Security note: the session token is persisted in `localStorage` so an
 * authenticated session survives reloads. `localStorage` is readable by any
 * script running in the page origin, so it is exposed to XSS payloads. This is
 * an explicit, documented alpha trade-off: the hosted alpha has no cookie-based
 * session mechanism yet, and a bearer token in `localStorage` is the simplest
 * contract that works across the HTTP and WebSocket (`?token=`) auth paths.
 * Production hardening should move session issuance to an `httpOnly`,
 * `SameSite` cookie set by the gateway so the token is never visible to
 * client-side JavaScript; the web auth integration surface (`AppAuthIntegration`)
 * is transport-agnostic and can adopt that without changing the shell. Until
 * then, the web client is served from a trusted, CSP-locked origin and must not
 * embed untrusted content, which bounds the XSS surface.
 */
export const SESSION_TOKEN_KEY = "opensymphony.session_token";

export function readStoredToken(): string | undefined {
  try {
    return window.localStorage.getItem(SESSION_TOKEN_KEY) ?? undefined;
  } catch {
    return undefined;
  }
}

export function storeToken(token: string | undefined): void {
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

export interface WebAuthIntegrationOptions {
  /** Build a gateway reader carrying `token` against `gatewayUrl`. */
  buildTransport: (gatewayUrl: string, token?: string) => GatewayReader;
  /** Optional fetch implementation forwarded to `HostedAuthClient` (tests). */
  fetchImpl?: typeof fetch;
}

export type WebAuthIntegration = AppAuthIntegration & {
  /** Rebuild the auth client against a new gateway base URI (profile switch). */
  setGatewayUrl: (gatewayUrl: string) => void;
};

/**
 * Build the web auth integration. The returned object implements
 * `AppAuthIntegration` and additionally exposes `setGatewayUrl` so the shell
 * can keep the auth client's base URI in sync with the active transport.
 */
export function createWebAuthIntegration(
  initialGatewayUrl: string,
  options: WebAuthIntegrationOptions,
): WebAuthIntegration {
  let gatewayUrl = initialGatewayUrl;
  let storedToken = readStoredToken();
  const newClient = (token?: string) =>
    new HostedAuthClient({
      baseUri: gatewayUrl,
      sessionToken: token,
      fetchImpl: options.fetchImpl,
    });
  let client = newClient(storedToken);
  return {
    async login(credentials: AppLoginCredentials): Promise<string> {
      const response = await client.login({
        email: credentials.email,
        password: credentials.password,
        organization_slug: credentials.organizationSlug,
      });
      storedToken = response.session_token;
      storeToken(storedToken);
      return storedToken;
    },
    async logout(): Promise<void> {
      await client.logout().catch(() => undefined);
      storedToken = undefined;
      storeToken(undefined);
      client = newClient(undefined);
    },
    async applySession(token: string | undefined): Promise<GatewayReader> {
      storedToken = token;
      client.setToken(token);
      storeToken(token);
      return options.buildTransport(gatewayUrl, token);
    },
    setGatewayUrl(nextGatewayUrl: string): void {
      gatewayUrl = nextGatewayUrl;
      // Rebuild against the new base URI, preserving the current session token
      // so an already-authenticated user does not have to sign in again.
      client = newClient(storedToken);
    },
  };
}