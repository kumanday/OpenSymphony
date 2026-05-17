import type { SchemaVersion } from "./version.js";

export type TerminalFrameKind =
  | "stdout"
  | "stderr"
  | "log"
  | "prompt"
  | "status"
  | "end_of_stream";

export type TerminalEncoding = "utf8" | "base64";

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
