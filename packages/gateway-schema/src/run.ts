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
