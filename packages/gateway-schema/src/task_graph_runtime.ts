import type { SchemaVersion } from "./version.js";
import type { RunStatus, ReleaseReason } from "./run.js";

/** Runtime badge kinds that can be rendered on a task graph node. */
export type RuntimeBadgeKind =
  | "failed"
  | "queued"
  | "running"
  | "complete"
  | "stale"
  | "workspace"
  | "harness"
  | "diff_summary"
  | "validation"
  | "retry"
  | "blocker";

/** Summary of an active or historical run attached to a task graph node. */
export interface TaskGraphRuntimeOverlay {
  schema_version: SchemaVersion;
  node_id: string;
  run_id?: string;
  status?: RunStatus;
  release_reason?: ReleaseReason;
  phase?: string;
  /** True when the run is stale (no recent events). */
  is_stale?: boolean;
  /** True when the node is blocked by unresolved dependencies. */
  is_blocked?: boolean;
  workspace_path?: string;
  harness?: string;
  diff_summary?: string;
  validation_status?: "passed" | "failed" | "pending" | "unknown";
  retry_attempt?: number;
  blocked_by_count?: number;
  last_event_at?: string;
  badges: RuntimeBadgeKind[];
}

/** Payload for updating a task graph node through a gateway action. */
export interface TaskGraphUpdatePayload {
  node_id: string;
  title?: string;
  state?: string;
  priority?: number;
  estimate_minutes?: number;
  labels?: string[];
}

/** Payload for creating a task graph child node through a gateway action. */
export interface TaskGraphCreatePayload {
  parent_id?: string;
  kind: "milestone" | "issue" | "sub_issue";
  title: string;
  state?: string;
  identifier?: string;
  priority?: number;
  estimate_minutes?: number;
  labels?: string[];
}

/** Payload for editing task graph dependencies. */
export interface TaskGraphDependencyPayload {
  node_id: string;
  blocked_by: string[];
}

/** Payload for adding a comment or evidence note to a task graph node. */
export interface TaskGraphCommentPayload {
  node_id: string;
  body: string;
  kind: "comment" | "evidence";
}

/** Mutation result returned in the action receipt result field. */
export interface TaskGraphMutationResult {
  node_id: string;
  updated_at: string;
  applied: boolean;
}
