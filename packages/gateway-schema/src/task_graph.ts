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
}

/** Flat list response for a project task graph. */
export interface TaskGraphSnapshot {
  schema_version: SchemaVersion;
  project_id: string;
  generated_at: string;
  nodes: TaskGraphNode[];
  root_ids: string[];
}
