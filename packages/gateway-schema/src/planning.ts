import type { SchemaVersion } from "./version.js";

export type PlanningArtifactKind =
  | "intake"
  | "requirements"
  | "milestone_draft"
  | "issue_draft"
  | "sub_issue_draft"
  | "dependency_map"
  | "acceptance_criteria"
  | "verification_plan"
  | "research_summary"
  | "codebase_analysis";

/** Planning session artifact. */
export interface PlanningArtifact {
  schema_version: SchemaVersion;
  artifact_id: string;
  session_id: string;
  kind: PlanningArtifactKind;
  title: string;
  content: string;
  created_at: string;
  updated_at: string;
  generated_by?: string;
  approved: boolean;
  published_to_tracker: boolean;
}

export type PlanningSessionStatus =
  | "draft"
  | "in_review"
  | "approved"
  | "published"
  | "archived";

/** Planning session summary for listing. */
export interface PlanningSessionSummary {
  schema_version: SchemaVersion;
  session_id: string;
  project_id: string;
  title: string;
  status: PlanningSessionStatus;
  artifact_count: number;
  created_at: string;
  updated_at: string;
}
