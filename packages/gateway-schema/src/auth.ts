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

import type { SchemaVersion } from "./version.js";

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

/**
 * Auth error code strings carried by classified gateway errors and auth
 * response bodies. Mirrors the Rust `AuthErrorCode` set so a 403 body carrying
 * one of the permission-denial signals maps to the `unauthorized` shell state.
 */
export type AuthErrorCode =
  | "unauthenticated"
  | "unauthorized"
  | "permission_denied"
  | "forbidden_resource"
  | "forbidden"
  | "dev_bypass_disabled";

const AUTH_ERROR_CODES: ReadonlySet<AuthErrorCode> = new Set([
  "unauthenticated",
  "unauthorized",
  "permission_denied",
  "forbidden_resource",
  "forbidden",
  "dev_bypass_disabled",
]);

/** Permission-denial body signals the client classifies as `unauthorized`. */
const UNAUTHORIZED_BODY_CODES: ReadonlySet<AuthErrorCode> = new Set([
  "unauthorized",
  "permission_denied",
  "forbidden_resource",
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
 * Permission-denial body codes (`unauthorized`, `permission_denied`,
 * `forbidden_resource`) collapse to the `unauthorized` state; a hard 403
 * (`forbidden`, `dev_bypass_disabled`) maps to `forbidden`. Returns `"open"`
 * when the error is not auth-related, so callers fall back to the normal
 * connection-failure path.
 */
export function authStateFromError(error: unknown): AuthState {
  const code = readErrorCode(error);
  if (!code) return "open";
  if (code === "unauthenticated") return "unauthenticated";
  if (UNAUTHORIZED_BODY_CODES.has(code)) return "unauthorized";
  return "forbidden";
}

// --- Hosted identity schema (COE-420) ----------------------------------------

/** Hosted role ranking. Mirrors the Rust `Role` enum (owner > admin > member > viewer). */
export type Role = "owner" | "admin" | "member" | "viewer";

/** A hosted user identity. */
export interface HostedUser {
  schema_version: SchemaVersion;
  user_id: string;
  email: string;
  display_name: string;
  handle: string;
}

/** A hosted organization / tenant. */
export interface Organization {
  schema_version: SchemaVersion;
  organization_id: string;
  slug: string;
  display_name: string;
}

/** Request body for `POST /api/v1/auth/login`. */
export interface LoginRequest {
  email: string;
  password: string;
  /** Organization the user is signing into (tenant selection). */
  organization_slug?: string;
}

/** Response body returned after successful authentication. */
export interface LoginResponse {
  schema_version: SchemaVersion;
  /** Session token; persisted by the client and presented as a bearer token. */
  session_token: string;
  user: HostedUser;
  organization: Organization;
  role: Role;
  /** RFC 3339 expiry timestamp. */
  expires_at: string;
}

/** Response body for `GET /api/v1/auth/session` (token probe). */
export interface SessionResponse {
  schema_version: SchemaVersion;
  user: HostedUser;
  organization: Organization;
  role: Role;
  expires_at: string;
}

/** Standard error body for auth failures, carrying an `error_code`. */
export interface AuthErrorBody {
  error_code: AuthErrorCode;
  message: string;
}