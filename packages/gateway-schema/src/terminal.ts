import type { SchemaVersion } from "./version.js";

export type TerminalFrameKind =
  | "stdout"
  | "stderr"
  | "log"
  | "prompt"
  | "status"
  | "end_of_stream";

export type TerminalEncoding = "utf8" | "base64";

/** Association context for a terminal/log frame. */
export interface TerminalLogAssociation {
  run_id: string;
  workspace_id: string;
  command_id?: string;
  issue_id?: string;
  sub_issue_id?: string;
  harness_session_id?: string;
}

/** Terminal or log frame delivered over a high-volume stream. */
export interface TerminalFrame {
  schema_version: SchemaVersion;
  frame_sequence: number;
  stream_id: string;
  run_id: string;
  terminal_session_id: string;
  frame_kind: TerminalFrameKind;
  encoding: TerminalEncoding;
  content: string;
  timestamp: string;
  association: TerminalLogAssociation;
  correlation_id?: string;
  source_event_id?: string;
  frame_id?: string;
}

/** Terminal snapshot for REST endpoint. */
export interface TerminalSnapshot {
  schema_version: SchemaVersion;
  terminal_session_id: string;
  run_id: string;
  frames: TerminalFrame[];
  total_frames: number;
  truncated: boolean;
  cursor: number;
}

/** Terminal/log session metadata. */
export interface TerminalSession {
  schema_version: SchemaVersion;
  terminal_session_id: string;
  run_id: string;
  association: TerminalLogAssociation;
  frame_count: number;
  total_bytes: number;
  created_at: string;
  updated_at: string;
  current_cursor: number;
}
