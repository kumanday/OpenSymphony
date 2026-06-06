// version
export { GATEWAY_SCHEMA_VERSION, schemaVersionV1, schemaVersionToString, schemaVersionFromString } from "./version.js";
export type { SchemaVersion } from "./version.js";

// cursor
export { streamCursor, pageCursorFirst } from "./cursor.js";
export type { StreamCursor, PageCursor } from "./cursor.js";

// envelope
export { entityRefIssue, entityRefRun, entityRefTerminal } from "./envelope.js";
export type { EntityKind, EntityRef, GatewayEnvelope } from "./envelope.js";

// snapshot
export type { GatewayHealth, GatewayMetrics, ProjectSummary, SnapshotEventKind, SnapshotEventSummary, DashboardSnapshot } from "./snapshot.js";

// task_graph
export type { TaskGraphNodeKind, TaskGraphStateCategory, TaskGraphNode, TaskGraphSnapshot } from "./task_graph.js";

// run
export type { RunStatus, ReleaseReason, RunDetail, RunEventPage, RunEvent } from "./run.js";

// terminal
export type { TerminalFrameKind, TerminalEncoding, TerminalFrame, TerminalSnapshot } from "./terminal.js";

// approval
export type { ApprovalKind, ApprovalStatus, ApprovalRequest, ActionReceiptStatus, ActionReceipt } from "./approval.js";

// planning
export type { PlanningArtifactKind, PlanningArtifact, PlanningSessionStatus, PlanningSessionSummary } from "./planning.js";

// capability
export type { AuthMode, TransportCapability, FeatureCapability, GatewayCapabilities } from "./capability.js";

// action
export type { ActionKind, ActionTarget, ActionDispatch } from "./action.js";

// transport
export type { TransportProfile, TransportRecommendation } from "./transport.js";

// profile
export type {
  ConnectionProfile,
  ConnectionProfileBase,
  ConnectionProfileKind,
  LocalDaemonProfile,
  SupervisedLocalDaemonProfile,
  EmbeddedHostProfile,
  ExternalGatewayProfile,
  HostedGatewayProfile,
} from "./profile.js";
export {
  defaultProfiles,
  createProfile,
} from "./profile.js";

// validation
export {
  isValidSchemaVersion,
  assertCompatibleSchemaVersion,
  isValidGatewayEnvelope,
  assertValidGatewayEnvelope,
  validateEnvelopeBatch,
  assertValidEnvelopeBatch,
  parseGatewayEnvelope,
  getGatewaySchemaVersion,
} from "./validation.js";
