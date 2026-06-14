import type { SchemaVersion } from "./version.js";
import type { EntityKind } from "./envelope.js";
import type { RunPhase, RunStreamLiveness } from "./run.js";

export type TimelineEntryKind =
  | "phase"
  | "tool_call"
  | "command"
  | "token_update"
  | "reconnect"
  | "stall_probe"
  | "progress"
  | "state"
  | "log"
  | "terminal"
  | "file"
  | "unknown";

export interface TimelineEntityRef {
  kind: EntityKind;
  id: string;
  identifier?: string;
}

export interface TokenDelta {
  input: number;
  output: number;
  cache_read: number;
}

export interface RunStateEvidence {
  phase: RunPhase;
  stream: RunStreamLiveness;
  last_activity_at?: string;
  stall_deadline_at?: string;
  explanation: string;
}

export interface TimelineEntry {
  entry_id: string;
  sequence_start: number;
  sequence_end: number;
  happened_at: string;
  kind: TimelineEntryKind;
  phase?: RunPhase;
  title: string;
  summary: string;
  event_ids: string[];
  entity_refs: TimelineEntityRef[];
  command_id?: string;
  tool_name?: string;
  file_paths: string[];
  terminal_session_id?: string;
  log_level?: string;
  token_delta?: TokenDelta;
  state_evidence?: RunStateEvidence;
}

export interface RunTimeline {
  schema_version: SchemaVersion;
  run_id: string;
  generated_at: string;
  entries: TimelineEntry[];
}

export interface TerminalSearchMatch {
  frame_sequence: number;
  frame_timestamp: string;
  snippet: string;
}

export interface TerminalSearchResult {
  schema_version: SchemaVersion;
  terminal_session_id: string;
  query: string;
  matches: TerminalSearchMatch[];
}

export interface TerminalJumpResult {
  schema_version: SchemaVersion;
  terminal_session_id: string;
  event_id: string;
  frame_sequence?: number;
  found: boolean;
}

export interface RunLogEntry {
  sequence: number;
  event_id: string;
  happened_at: string;
  level: string;
  message: string;
  terminal_session_id?: string;
  command_id?: string;
}

export interface RunLogPage {
  schema_version: SchemaVersion;
  run_id: string;
  next_cursor?: number;
  entries: RunLogEntry[];
}
