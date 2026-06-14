import type { StreamCursor } from "./cursor.js";
import type { SchemaVersion } from "./version.js";

/** Known entity kinds referenced by gateway schemas. */
export type EntityKind =
  | "issue"
  | "sub_issue"
  | "milestone"
  | "run"
  | "workspace"
  | "conversation"
  | "terminal_session"
  | "planning_session"
  | "project"
  | "repository"
  | "agent"
  | "harness"
  | "command"
  | "approval"
  | "unknown";

/** Lightweight reference to an entity. */
export interface EntityRef {
  kind: EntityKind;
  id: string;
  identifier?: string;
}

/** Base envelope for every gateway event or snapshot stream item. */
export interface GatewayEnvelope {
  schema_version: SchemaVersion;
  cursor: StreamCursor;
  entity_ref: EntityRef;
  event_kind: string;
  payload?: unknown;
  raw_payload?: unknown;
  emitted_at: string;
}

export function entityRefIssue(id: string, identifier?: string): EntityRef {
  return { kind: "issue", id, identifier };
}

export function entityRefRun(id: string): EntityRef {
  return { kind: "run", id };
}

export function entityRefTerminal(id: string): EntityRef {
  return { kind: "terminal_session", id };
}
