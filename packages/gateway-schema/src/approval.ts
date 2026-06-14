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
  | "cancelled"
  | "passed"
  | "failed";

/** Actor that requested or approved an action. */
export interface ApprovalActor {
  actor_id: string;
  actor_kind: string;
  display_name?: string;
}

/** Target context for an approval request (file, command, issue, run). */
export interface ApprovalTargetContext {
  file_path?: string;
  command?: string;
  issue_id?: string;
  issue_identifier?: string;
  run_id?: string;
}

/** Risk level for an approval request. */
export type ApprovalRiskLevel = "low" | "medium" | "high" | "unknown";

/** Risk summary for an approval request. */
export interface ApprovalRiskSummary {
  level: ApprovalRiskLevel;
  reasons: string[];
}

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
  /** Actor who requested the approval. */
  actor?: ApprovalActor;
  /** Target context (file, command, issue, run) for the request. */
  target_context?: ApprovalTargetContext;
  /** Risk summary when available. */
  risk_summary?: ApprovalRiskSummary;
  requested_at: string;
  /** ISO timestamp when the approval was decided, if already decided. */
  decided_at?: string;
  expires_at?: string;
  status: ApprovalStatus;
  correlation_id: string;
}

/** Status of an action after gateway validation. */
export type ActionStatus = "accepted" | "rejected";

/** Expected follow-up event type after an action. */
export type ExpectedFollowup =
  | "state_transition"
  | "run_lifecycle"
  | "action_completion"
  | "journal_update"
  | "task_graph_update";

/** Permission check result for hosted mode. */
export interface PermissionResult {
  allowed: boolean;
  required_role: string;
  evaluated: boolean;
}

/** Action receipt returned after a mutation. */
export interface ActionReceipt {
  schema_version: SchemaVersion;
  action_id: string;
  correlation_id: string;
  status: ActionStatus;
  reason?: string;
  expected_followup: ExpectedFollowup[];
  result?: unknown;
  issued_at: string;
  /** Hosted-mode permission check result; omitted in local mode. */
  permission?: PermissionResult;
}

/** @deprecated Use `ActionStatus` instead. */
export type ActionReceiptStatus = ActionStatus;
