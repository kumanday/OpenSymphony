import type { SchemaVersion } from "./version.js";

export type GatewayHealth = "healthy" | "degraded" | "failed" | "starting";

export interface GatewayMetrics {
  running_issue_count: number;
  retry_queue_depth: number;
  total_input_tokens: number;
  total_output_tokens: number;
  total_cache_read_tokens: number;
  total_cost_micros: number;
}

export interface ProjectSummary {
  project_id: string;
  name: string;
  milestone_count: number;
  issue_count: number;
  running_count: number;
  completed_count: number;
  failed_count: number;
}

export type SnapshotEventKind =
  | "worker_started"
  | "workspace_prepared"
  | "stream_attached"
  | "snapshot_published"
  | "worker_completed"
  | "retry_scheduled"
  | "client_attached"
  | "client_detached"
  | "warning";

export interface SnapshotEventSummary {
  happened_at: string;
  issue_identifier?: string;
  kind: SnapshotEventKind;
  summary: string;
}

/** Dashboard-level snapshot delivered over REST or SSE. */
export interface DashboardSnapshot {
  schema_version: SchemaVersion;
  generated_at: string;
  sequence: number;
  health: GatewayHealth;
  metrics: GatewayMetrics;
  projects: ProjectSummary[];
  recent_events: SnapshotEventSummary[];
}
