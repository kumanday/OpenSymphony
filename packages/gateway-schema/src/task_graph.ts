import type { SchemaVersion } from "./version.js";

export type TaskGraphNodeKind = "milestone" | "issue" | "sub_issue";

export type TaskGraphStateCategory =
  | "backlog"
  | "todo"
  | "in_progress"
  | "done"
  | "canceled";

/** Read-only task graph node exposed by the gateway. */
export interface TaskGraphNode {
  schema_version: SchemaVersion;
  node_id: string;
  kind: TaskGraphNodeKind;
  identifier: string;
  title: string;
  state: string;
  state_category: TaskGraphStateCategory;
  priority?: number;
  /** Linear project metadata, present when a project-set snapshot is available. */
  project_id?: string;
  project_slug?: string;
  project_name?: string;
  /** Parent node identifier when the parent is present in this task graph snapshot. */
  parent_id?: string;
  /** Child node identifiers that are present in this task graph snapshot. */
  children: string[];
  /** Blocker node identifiers that are present in this task graph snapshot. */
  blocked_by: string[];
  url?: string;
  branch_name?: string;
  labels: string[];
  created_at?: string;
  updated_at?: string;
  estimate_minutes?: number;
  /** Identifier of the active or last run linked to this node, if any. */
  run_id?: string;
  /** Count of comments / evidence notes attached to this node. */
  comment_count?: number;
  /**
   * Runtime overlay, present exactly when the orchestrator control plane
   * tracks this node (queued/eligible flags, run linkage). Absent for nodes
   * known only from the tracker scan (backlog, freshly promoted issues).
   */
  runtime_overlay?: TaskGraphNodeRuntimeOverlay;
}

/**
 * Runtime overlay embedded on a control-plane-tracked task graph node.
 * Distinct from `TaskGraphRuntimeOverlay` (task_graph_runtime.ts), the
 * separately fetched run-status overlay keyed by run id.
 */
export interface TaskGraphNodeRuntimeOverlay {
  eligible: boolean;
  queued: boolean;
  active_run_id?: string;
  last_outcome?: string;
  retry_count: number;
  workspace_id?: string;
  harness_type?: string;
  conversation_id?: string;
  last_event_at?: string;
  diff_summary?: unknown;
  validation_status?: string;
  blocker_summary?: string;
}

/** Flat list response for a project task graph. */
export interface TaskGraphSnapshot {
  schema_version: SchemaVersion;
  project_id: string;
  generated_at: string;
  nodes: TaskGraphNode[];
  root_ids: string[];
}
