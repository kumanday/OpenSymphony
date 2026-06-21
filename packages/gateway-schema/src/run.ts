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
  | "cancel_failed"
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
export type RunStreamLiveness =
  | "healthy"
  | "stale"
  | "dead"
  | "detached"
  | "degraded"
  | "stalled";

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
  harness_acknowledged?: boolean;
  cancel_failed?: boolean;
  detached?: boolean;
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
  /** True when the harness acknowledged a cancel/force-stop request. */
  cancel_acknowledged?: boolean;
  /** True when a cancel/force-stop request was not acknowledged. */
  cancel_failed?: boolean;
}

/** Actions the client may safely invoke in the current run state. */
export interface SafeActions {
  retry: boolean;
  cancel: boolean;
  rehydrate: boolean;
  detach: boolean;
}

/** Lifecycle state of a run from the orchestrator's perspective. */
export type RunLifecycleState =
  | "eligible"
  | "queued"
  | "claimed"
  | "running"
  | "paused"
  | "releasing"
  | "completed"
  | "failed"
  | "canceled"
  | "retry_exhausted";

/** Action a client may dispatch on a run. */
export type RunAction =
  | "retry"
  | "cancel"
  | "pause"
  | "resume"
  | "rehydrate"
  | "detach"
  | "comment"
  | "create_followup"
  | "open_workspace"
  | "debug";

/** Run detail exposed by the gateway. */
export interface RunDetail {
  schema_version: SchemaVersion;
  run_id: string;
  issue_id: string;
  issue_identifier: string;
  worker_id: string;
  status: RunStatus;
  /** Lifecycle state of the run from the orchestrator's perspective. */
  lifecycle_state?: RunLifecycleState;
  claimed_at: string;
  started_at?: string;
  finished_at?: string;
  release_reason?: ReleaseReason;
  turn_count: number;
  /** Configured turn budget. A value of 0 means the budget is unknown. */
  max_turns: number;
  retry_attempt?: number;
  input_tokens: number;
  output_tokens: number;
  cache_read_tokens: number;
  /**
   * Elapsed runtime in whole seconds. A value of 0 means runtime is unknown
   * unless the run is actively running and has a start timestamp.
   */
  runtime_seconds: number;
  conversation_id?: string;
  /** Logical workspace identifier for hosted mode. */
  workspace_id?: string;
  /** Local filesystem path (absent in hosted mode). */
  workspace_path?: string;
  /** Harness type (e.g. "openhands"). */
  harness_type?: string;
  /** Brief human-readable summary of the run. */
  summary?: string;
  /** Blocker description when the run is blocked. */
  blocker?: string;
  error?: string;
  /** Actions the client may perform on this run, regardless of safety state. */
  allowed_actions?: RunAction[];
  /** Liveness envelope describing the phase, stream health, and latest progress. */
  liveness?: RunLivenessEnvelope | null;
  /** Diagnostic hints surfaced when multiple subsystems disagree. */
  diagnostics?: RunDiagnostics | null;
  /** Actions the client may safely invoke in the current state. */
  safe_actions?: SafeActions | null;
  /** True when the harness session has been detached from the run. */
  detached?: boolean;
  /** True when the harness acknowledged a cancel/force-stop request. */
  cancel_acknowledged?: boolean;
  /** True when a cancel/force-stop request was not acknowledged. */
  cancel_failed?: boolean;
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

/** Kind of change for a single file. */
export type FileChangeKind = "created" | "modified" | "removed";

/** Single changed-file entry for `/api/v1/runs/{run_id}/files`. */
export interface ChangedFileEntry {
  /** Workspace-relative path (never a raw absolute local path). */
  path: string;
  change_kind: FileChangeKind;
  lines_added: number;
  lines_removed: number;
  size_bytes?: number;
}

/** Single line inside a diff hunk. */
export type DiffLine =
  | { type: "context"; line: string }
  | { type: "addition"; line: string }
  | { type: "deletion"; line: string };

/** A contiguous hunk inside a unified diff. */
export interface DiffHunk {
  /** Path of the file this hunk belongs to, relative to the workspace root. */
  file_path: string;
  header: string;
  start_line: number;
  old_line_count: number;
  new_line_count: number;
  lines: DiffLine[];
}

/** Paged diff response for `/api/v1/runs/{run_id}/diffs`. */
export interface FileDiffPage {
  schema_version: SchemaVersion;
  run_id: string;
  file_path: string;
  next_cursor?: PageCursor;
  hunks: DiffHunk[];
  total_lines_added: number;
  total_lines_removed: number;
}
