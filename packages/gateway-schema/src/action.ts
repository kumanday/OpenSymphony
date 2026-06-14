import type { EntityKind } from "./envelope.js";
import type { SchemaVersion } from "./version.js";

export type ActionKind =
  | "retry"
  | "cancel"
  | "pause"
  | "resume"
  | "rehydrate"
  | "comment"
  | "transition_issue"
  | "create_followup"
  | "approval_decision"
  | "publish_plan"
  | "update_node";

export interface ActionTarget {
  entity_kind: EntityKind;
  entity_id: string;
}

/** Action dispatch payload for POST /api/v1/actions/dispatch. */
export interface ActionDispatch {
  schema_version: SchemaVersion;
  correlation_id: string;
  action_kind: ActionKind;
  target_entity: ActionTarget;
  payload?: unknown;
  idempotency_key?: string;
}
