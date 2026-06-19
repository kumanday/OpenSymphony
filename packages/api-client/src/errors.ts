/**
 * Typed gateway request errors.
 *
 * Gateway reads and mutations can fail for several reasons. Transport
 * adapters throw `GatewayRequestError` so the UI shell can distinguish
 * authentication/authorization outcomes from generic connectivity
 * failures and render the matching placeholder state.
 *
 * The shared `AuthState` classification lives in `@opensymphony/gateway-schema`
 * so the UI shell can map errors to auth states without depending on the
 * transport package.
 */

/** Coarse classification of a gateway failure. */
export type GatewayErrorCode =
  /** No valid credentials supplied (HTTP 401). */
  | "unauthenticated"
  /** Authenticated but lacking permission for the resource (HTTP 403, permission). */
  | "unauthorized"
  /** Server explicitly forbids the request (HTTP 403, hard deny). */
  | "forbidden"
  /** Gateway unreachable or returned a non-auth error. */
  | "unavailable";

/** Error thrown by transport adapters for classified gateway failures. */
export class GatewayRequestError extends Error {
  readonly code: GatewayErrorCode;
  readonly status?: number;

  constructor(code: GatewayErrorCode, message: string, status?: number) {
    super(message);
    this.name = "GatewayRequestError";
    this.code = code;
    this.status = status;
  }
}

/** True when the value is a classified gateway request error. */
export function isGatewayRequestError(value: unknown): value is GatewayRequestError {
  return value instanceof GatewayRequestError;
}

/**
 * Classify an HTTP status code into a gateway error code.
 *
 * Returns `undefined` for status codes that are not auth/forbidden related
 * (callers treat those as generic unavailable failures).
 */
export function authErrorCodeForStatus(status: number): GatewayErrorCode | undefined {
  if (status === 401) return "unauthenticated";
  if (status === 403) return "forbidden";
  return undefined;
}

export type { AuthState } from "@opensymphony/gateway-schema";