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
  /**
   * Authenticated but lacking permission for this resource. Reached when a
   * 403 response carries an explicit permission-denial body signal. The body
   * is read from an `error_code` (or `code`) string field, and the following
   * values are recognized as permission denials: `"unauthorized"`,
   * `"permission_denied"`, and `"forbidden_resource"`. A 403 without one of
   * these signals is treated as a hard deny (`forbidden`).
   */
  | "unauthorized"
  /** Server hard-denies the request (HTTP 403 without a permission signal). */
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
 * Body signal a gateway may use to distinguish a permission denial
 * (`unauthorized`) from a hard deny (`forbidden`) on an HTTP 403.
 */
const UNAUTHORIZED_BODY_CODES = new Set(["unauthorized", "permission_denied", "forbidden_resource"]);

/**
 * Classify an HTTP status code into a gateway error code.
 *
 * - HTTP 401 -> `unauthenticated`.
 * - HTTP 403 -> `unauthorized` when the (parsed) body carries an explicit
 *   `error_code`/`code` field equal to a permission-denial signal, otherwise
 *   `forbidden`.
 *
 * Pass the raw response body so a 403 can be disambiguated. Returns
 * `undefined` for status codes that are not auth/forbidden related; callers
 * treat those as generic unavailable failures.
 */
export function authErrorCodeForStatus(
  status: number,
  body?: unknown,
): GatewayErrorCode | undefined {
  if (status === 401) return "unauthenticated";
  if (status === 403) {
    const code = bodyErrorCode(body);
    return code && UNAUTHORIZED_BODY_CODES.has(code) ? "unauthorized" : "forbidden";
  }
  return undefined;
}

/** Extract an `error_code`/`code` string from a JSON-parsed body, if present. */
function bodyErrorCode(body: unknown): string | undefined {
  if (body && typeof body === "object") {
    const record = body as Record<string, unknown>;
    const raw = record.error_code ?? record.code;
    if (typeof raw === "string" && raw.length > 0) return raw;
  }
  return undefined;
}