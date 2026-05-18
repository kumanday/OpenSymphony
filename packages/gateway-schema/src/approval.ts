import type { SchemaVersion } from "./version.js";

export type ApprovalKind =
  | "tool_use"
  | "file_write"
  | "command_execution"
  | "plan_publish"
  | "custom";

export type ApprovalStatus =
  | "pending"
  | "approved"
  | "rejected"
  | "expired"
  | "cancelled";

/** Approval request for human-in-the-loop actions. */
export interface ApprovalRequest {
  schema_version: SchemaVersion;
  approval_id: string;
  run_id: string;
  issue_id: string;
  kind: ApprovalKind;
  title: string;
  description: string;
  proposed_action?: unknown;
  requested_at: string;
  expires_at?: string;
  status: ApprovalStatus;
  correlation_id: string;
}

export type ActionReceiptStatus =
  | "accepted"
  | "rejected"
  | "queued"
  | "completed";

/** Action receipt returned after a mutation. */
export interface ActionReceipt {
  schema_version: SchemaVersion;
  action_id: string;
  correlation_id: string;
  status: ActionReceiptStatus;
  reason?: string;
  expected_events: string[];
  result?: unknown;
  issued_at: string;
}
