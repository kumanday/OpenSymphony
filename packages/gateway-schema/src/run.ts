import type { PageCursor } from "./cursor.js";
import type { SchemaVersion } from "./version.js";

export type RunStatus =
  | "unclaimed"
  | "claimed"
  | "running"
  | "retry_queued"
  | "released";

export type ReleaseReason =
  | "completed"
  | "tracker_inactive"
  | "tracker_terminal"
  | "cancelled"
  | "retry_exhausted";

/** Operational phase observed by the client for a long-running run. */
export type RunPhase =
  | "active"
  | "quiet"
  | "degraded"
  | "stalled"
  | "retry_queued"
  | "cancelled"
  | "detached"
  | "completed";

/** Stream-level liveness classification. */
export type RunStreamLiveness = "healthy" | "stale" | "dead";

/** Progress event emitted during a long-running run. */
export interface RunProgress {
  sequence: number;
  event_id: string;
  happened_at: string;
  kind: string;
  summary: string;
}

/** Compact snapshot of the current run liveness surface. */
export interface RunLivenessEnvelope {
  phase: RunPhase;
  stream: RunStreamLiveness;
  latest_progress?: RunProgress | null;
}

/** Details of a harness/scheduler disagreement. */
export interface HarnessSchedulerDisagreement {
  scheduler_status: RunStatus;
  harness_status: string;
  detected_at: string;
  resolution_path: string;
}

/** Diagnostic hints surfaced when multiple subsystems disagree. */
export interface RunDiagnostics {
  harness_scheduler_disagreement?: HarnessSchedulerDisagreement | null;
}

/** Actions the client may safely invoke in the current run state. */
export interface SafeActions {
  retry: boolean;
  cancel: boolean;
  rehydrate: boolean;
  detach: boolean;
}

/** Run detail exposed by the gateway. */
export interface RunDetail {
  schema_version: SchemaVersion;
  run_id: string;
  issue_id: string;
  issue_identifier: string;
  worker_id: string;
  status: RunStatus;
  claimed_at: string;
  started_at?: string;
  finished_at?: string;
  release_reason?: ReleaseReason;
  turn_count: number;
  max_turns: number;
  retry_attempt?: number;
  input_tokens: number;
  output_tokens: number;
  cache_read_tokens: number;
  runtime_seconds: number;
  conversation_id?: string;
  workspace_path?: string;
  error?: string;
  /** Liveness envelope describing the phase, stream health, and latest progress. */
  liveness?: RunLivenessEnvelope | null;
  /** Diagnostic hints surfaced when multiple subsystems disagree. */
  diagnostics?: RunDiagnostics | null;
  /** Actions the client may safely invoke in the current state. */
  safe_actions?: SafeActions | null;
}

/** Paged run events. */
export interface RunEventPage {
  schema_version: SchemaVersion;
  run_id: string;
  next_cursor?: PageCursor;
  events: RunEvent[];
}

export interface RunEvent {
  sequence: number;
  event_id: string;
  happened_at: string;
  kind: string;
  summary: string;
  payload?: unknown;
  raw_payload?: unknown;
}
