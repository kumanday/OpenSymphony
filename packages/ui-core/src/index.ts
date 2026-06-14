/**
 * Shared UI core module.
 *
 * Provides terminal rendering utilities, scrollback buffer management,
 * and benchmark harness for high-throughput terminal/log output.
 */

import type {
  DashboardSnapshot,
  TaskGraphNode,
  RunDetail,
  TerminalFrame,
  RunTimeline,
  TimelineEntry,
  TimelineEntryKind,
  TerminalLogAssociation,
  ChangedFileEntry,
  FileDiffPage,
  RunValidationSummary,
  ApprovalRequest,
  RunAction,
  SafeActions,
  ActionReceipt,
} from "@opensymphony/gateway-schema";

// Terminal renderer module
export * from "./terminal-renderer/index.js";

// Timeline renderer module
export {
  renderTimeline,
  filterTimelineEntries,
  findTimelineEntryByEventId,
  findTimelineEntryByEntityId,
} from "./timeline.js";

// Diff/validation/approval/action modules
export {
  renderChangedFileList,
  renderFileDiff,
} from "./diff.js";
export type { DiffFileItem } from "./diff.js";
export { renderValidationSummary } from "./validation.js";
export {
  renderApprovalList,
} from "./approval.js";
export type { ApprovalDecision } from "./approval.js";
export {
  buildActionBarItems,
  renderActionBar,
  renderActionReceipt,
  renderAuditTrailEntry,
} from "./run-actions.js";
export type { ActionBarItem } from "./run-actions.js";

export {
  renderOpenSymphonyApp,
} from "./app-shell.js";
export type {
  EditableProfileInput,
  GatewayReader,
  OpenSymphonyAppHandle,
  OpenSymphonyAppOptions,
  ProfileController,
} from "./app-shell.js";

export {
  addCriterion,
  addMessage,
  addPlanningNode,
  addVerification,
  buildFixturePlanningWorkspaceState,
  computeArtifactDiff,
  emptyPlanningWorkspaceState,
  removePlanningNode,
  removeCriterion,
  removeVerification,
  selectArtifact,
  selectRevision,
  toggleCriterion,
  toggleNodeExpanded,
  toggleVerification,
  updateArtifactContent,
  updateCriterion,
  updateNodeDependencies,
  updatePlanningNode,
  updateVerification,
  validatePlanningWorkspace,
} from "./planning-workspace.js";
export type {
  ConversationMessage,
  PlanningArtifactRevision,
  PlanningArtifactWithRevisions,
  PlanningNode,
  PlanningValidationMessage,
  PlanningWorkspaceState,
  PlanningWorkspaceTab,
  DiffLine,
} from "./planning-workspace.js";

export { renderPlanningWorkspace } from "./planning-workspace-ui.js";
export type { PlanningEditState } from "./planning-workspace-ui.js";

export interface UiTheme {
  mode: "light" | "dark";
  accent?: string;
}

export interface TerminalRenderConfig {
  fontFamily: string;
  fontSize: number;
  lineHeight: number;
  wrapLines: boolean;
  maxVisibleFrames: number;
}

export type TerminalFrameWithMeta = TerminalFrame & {
  renderedAt: string;
};

export type DashboardData = DashboardSnapshot;
export type TaskGraphData = TaskGraphNode[];
export type RunData = RunDetail;
export type RunTimelineData = RunTimeline;
export type TimelineEntryData = TimelineEntry;
export type TimelineEntryKindData = TimelineEntryKind;
export type TerminalLogAssociationData = TerminalLogAssociation;
export type ChangedFileEntryData = ChangedFileEntry;
export type FileDiffPageData = FileDiffPage;
export type RunValidationSummaryData = RunValidationSummary;
export type ApprovalRequestData = ApprovalRequest;
export type RunActionData = RunAction;
export type SafeActionsData = SafeActions;
export type ActionReceiptData = ActionReceipt;
