/**
 * Hosted auth login flow client (COE-420).
 *
 * Wraps the gateway auth endpoints (`/api/v1/auth/{login,session,logout}`) so
 * the web client shell can sign a user in, persist the session token, probe a
 * stored token, and sign out. Errors are classified into `GatewayRequestError`
 * with the same codes the transport layer uses, so the shell maps a failed
 * login to the matching auth state via `authStateFromError`.
 */

import type {
  LoginRequest,
  LoginResponse,
  SessionResponse,
} from "@opensymphony/gateway-schema";
import { GatewayRequestError, authErrorCodeForStatus } from "./errors.js";

/** Configuration for {@link HostedAuthClient}. */
export interface HostedAuthClientConfig {
  /** Gateway base URL (e.g. `https://hosted.opensymphony.example`). */
  baseUri: string;
  /**
   * Optional session token for authenticated calls (session probe/logout).
   * The login call does not require a token.
   */
  sessionToken?: string;
  /** Optional fetch implementation (defaults to the global `fetch`). */
  fetchImpl?: typeof fetch;
}

/**
 * Client for the hosted gateway auth endpoints.
 *
 * The shell owns token persistence; this client performs the network calls and
 * returns the session token + resolved identity on login. On a non-2xx
 * response it throws a `GatewayRequestError` whose `code` the shell maps to an
 * auth state (401 -> `unauthenticated`, 403 permission signal ->
 * `unauthorized`, else `forbidden`/`unavailable`).
 */
export class HostedAuthClient {
  private readonly baseUri: string;
  private sessionToken: string | undefined;
  private readonly fetchImpl: typeof fetch;

  constructor(config: HostedAuthClientConfig) {
    this.baseUri = config.baseUri.replace(/\/+$/, "");
    this.sessionToken = config.sessionToken;
    this.fetchImpl = config.fetchImpl ?? fetch;
  }

  /** The currently held session token (updated by {@link login}). */
  get token(): string | undefined {
    return this.sessionToken;
  }

  /** Replace the held session token (e.g. after restoring from storage). */
  setToken(token: string | undefined): void {
    this.sessionToken = token;
  }

  /**
   * Sign in with email/password and resolve the session token + identity.
   *
   * On success the client holds the returned session token for subsequent
   * authenticated calls (session probe/logout).
   */
  async login(request: LoginRequest): Promise<LoginResponse> {
    const body = await this.sendJson<LoginResponse>("/api/v1/auth/login", "POST", request, false);
    this.sessionToken = body.session_token;
    return body;
  }

  /** Probe a persisted token and recover the authenticated identity/tenant. */
  async session(): Promise<SessionResponse> {
    return this.sendJson<SessionResponse>("/api/v1/auth/session", "GET", undefined, true);
  }

  /** Invalidate the current session on the gateway. Resolves on success. */
  async logout(): Promise<void> {
    await this.sendJson<void>("/api/v1/auth/logout", "POST", undefined, true);
    this.sessionToken = undefined;
  }

  private async sendJson<T>(
    path: string,
    method: string,
    payload: unknown,
    authenticate: boolean,
  ): Promise<T> {
    const headers: Record<string, string> = {
      Accept: "application/json",
    };
    if (method !== "GET" && method !== "HEAD" && payload !== undefined) {
      headers["Content-Type"] = "application/json";
    }
    if (authenticate && this.sessionToken) {
      headers.Authorization = `Bearer ${this.sessionToken}`;
    }
    const init: RequestInit =
      payload !== undefined
        ? { method, headers, body: JSON.stringify(payload) }
        : { method, headers };

    const response = await this.fetchImpl(`${this.baseUri}${path}`, init);
    if (!response.ok) {
      const rawBody = await response.text().catch(() => "");
      const authCode = authErrorCodeForStatus(response.status, tryParseJson(rawBody));
      if (authCode) {
        throw new GatewayRequestError(
          authCode,
          `HTTP ${response.status} ${response.statusText}: ${rawBody}`,
          response.status,
        );
      }
      throw new GatewayRequestError(
        "unavailable",
        `HTTP ${response.status} ${response.statusText}: ${rawBody}`,
        response.status,
      );
    }
    if (response.status === 204 || method === "POST" && path.endsWith("/logout")) {
      return undefined as T;
    }
    return (await response.json()) as T;
  }
}

/** Parse a JSON body for error-code classification; returns undefined on failure. */
function tryParseJson(raw: string): unknown {
  try {
    return JSON.parse(raw);
  } catch {
    return undefined;
  }
}