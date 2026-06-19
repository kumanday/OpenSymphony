/**
 * Auth-facing UI state for the OpenSymphony client shell.
 *
 * The gateway advertises auth requirements through `GatewayCapabilities.auth_modes`.
 * Transport adapters signal auth outcomes by throwing errors that carry a
 * `code` field (see `GatewayRequestError` in `@opensymphony/api-client`). The
 * shell maps those outcomes to an `AuthState` so it can render distinct
 * placeholder surfaces (sign-in, unauthorized, forbidden) without depending
 * on the transport package.
 */

/** Auth-facing state rendered by the client shell. */
export type AuthState =
  /** Gateway requires no auth (local unauthenticated development mode). */
  | "open"
  /** No valid credentials supplied (HTTP 401). */
  | "unauthenticated"
  /** Authenticated but lacking permission for the resource. */
  | "unauthorized"
  /** Server explicitly forbids the request (HTTP 403 hard deny). */
  | "forbidden";

/** Auth error code strings carried by classified gateway errors. */
export type AuthErrorCode = "unauthenticated" | "unauthorized" | "forbidden";

const AUTH_ERROR_CODES: ReadonlySet<AuthErrorCode> = new Set([
  "unauthenticated",
  "unauthorized",
  "forbidden",
]);

interface ErrorWithCode {
  code?: string;
}

function readErrorCode(error: unknown): AuthErrorCode | undefined {
  if (error && typeof error === "object" && "code" in error) {
    const code = (error as ErrorWithCode).code;
    if (typeof code === "string" && AUTH_ERROR_CODES.has(code as AuthErrorCode)) {
      return code as AuthErrorCode;
    }
  }
  return undefined;
}

/**
 * Map a thrown gateway error to an auth-facing state.
 *
 * Returns `"open"` when the error is not auth-related, so callers fall back
 * to the normal connection-failure path.
 */
export function authStateFromError(error: unknown): AuthState {
  const code = readErrorCode(error);
  if (code) return code;
  return "open";
}