import type { EntityKind } from "./envelope.js";
import type { SchemaVersion } from "./version.js";

export type ActionKind =
  | "retry"
  | "cancel"
  | "pause"
  | "resume"
  | "rehydrate"
  | "comment"
  | "open_workspace"
  | "debug"
  | "transition_issue"
  | "create_followup"
  | "approval_decision"
  | "publish_plan"
  | "task_graph_milestone"
  | "task_graph_issue"
  | "task_graph_sub_issue"
  | "task_graph_relation"
  | "task_graph_evidence"
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

// Action status, expected follow-up events, and permission checks are defined in
// approval.ts so that the ActionReceipt type is kept next to approval request
// metadata and matches the Rust gateway-schema exactly. Re-export the shared
// types here for callers that import from the action module.
export type { ActionStatus, ExpectedFollowup, PermissionResult } from "./approval.js";
