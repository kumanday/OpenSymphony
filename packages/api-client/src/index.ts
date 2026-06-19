import type {
  GatewayEnvelope,
  DashboardSnapshot,
  RunDetail,
  RunEventPage,
  TerminalSnapshot,
  TaskGraphSnapshot,
  GatewayCapabilities,
  TransportProfile,
  ActionDispatch,
  ActionReceipt,
  ApprovalRequest,
  PlanningSessionSummary,
  RunStatus,
  ReleaseReason,
  GatewayHealth,
  StreamCursor,
  PageCursor,
  RunTimeline,
  RunLogPage,
  TerminalSearchResult,
  TerminalJumpResult,
  ChangedFileEntry,
  FileDiffPage,
  RunValidationSummary,
  RunAction,
  RunPhase,
  SafeActions,
  RunLifecycleState,
} from "@opensymphony/gateway-schema";

export {
  HttpGatewayTransport,
  WebSocketTransport,
  TauriChannelTransport,
  TransportFactory,
  createTransportForProfile,
  binaryFramesAdvertised,
  encodeBinaryFrame,
  decodeBinaryFrame,
} from "./transports.js";
export type { TauriChannel, TauriRuntime } from "./transports.js";
export { MockGatewayTransport } from "./mock.js";
export {
  StreamReplayBuffer,
  orderedEvents,
  StreamCorrelator,
  envelopeCorrelationId,
} from "./stream-replay.js";
export type {
  StreamGap,
  StreamDuplicate,
  ReplayEvent,
  StreamReplayBufferOptions,
  OrderedEventsOptions,
  StreamStaleInfo,
} from "./stream-replay.js";

export {
  discoverGateway,
  discoverGatewayWithFallback,
  probeHealth,
  probeCapabilities,
  validateGateway,
  DEFAULT_GATEWAY_URL,
  MIN_COMPATIBLE_API_VERSION,
} from "./discovery.js";
export type { DiscoveryResult } from "./discovery.js";
// Typed gateway request errors (transport-layer). The auth-state
// classification (`AuthState`, `authStateFromError`) lives in
// @opensymphony/gateway-schema; import it directly from there to keep the
// package boundary sharp (ui-core must not depend on api-client).
export {
  GatewayRequestError,
  isGatewayRequestError,
  authErrorCodeForStatus,
} from "./errors.js";
export type { GatewayErrorCode } from "./errors.js";

/** Transport adapter interface for all gateway communication. */
export interface GatewayTransport {
  readonly baseUri: string;

  health(): Promise<GatewayCapabilities>;
  snapshot(): Promise<DashboardSnapshot>;
  taskGraph(projectId: string): Promise<TaskGraphSnapshot>;
  runDetail(runId: string): Promise<RunDetail>;
  runEvents(runId: string, cursor?: PageCursor): Promise<RunEventPage>;
  runTimeline(runId: string): Promise<RunTimeline>;
  runLogs(runId: string, cursor?: number, limit?: number): Promise<RunLogPage>;
  runFiles(runId: string): Promise<ChangedFileEntry[]>;
  runDiffs(runId: string, filePath?: string): Promise<FileDiffPage>;
  runApprovals(runId: string): Promise<ApprovalRequest[]>;
  runValidation(runId: string): Promise<RunValidationSummary>;
  terminalSnapshot(runId: string, terminalId: string, cursor?: number): Promise<TerminalSnapshot>;
  terminalSearch(runId: string, terminalId: string, query: string): Promise<TerminalSearchResult>;
  terminalJumpToEvent(runId: string, terminalId: string, eventId: string): Promise<TerminalJumpResult>;

  /** Subscribe to gateway event stream; returns an async iterable. */
  events(fromCursor?: { sequence: number; partition: string }): AsyncIterable<GatewayEnvelope>;

  /** Subscribe to terminal frame stream for a run. */
  terminalFrames(runId: string): AsyncIterable<GatewayEnvelope>;

  close(): Promise<void>;
}

/** Extended transport with action dispatch support. */
export interface ActionCapableTransport extends GatewayTransport {
  dispatchAction(action: ActionDispatch): Promise<ActionReceipt>;
  cancelRun(runId: string): Promise<ActionReceipt>;
  retryRun(runId: string): Promise<ActionReceipt>;
  resumeRun(runId: string): Promise<ActionReceipt>;
  rehydrateRun(runId: string): Promise<ActionReceipt>;
  commentRun(runId: string, text: string): Promise<ActionReceipt>;
  createFollowup(runId: string, payload: unknown): Promise<ActionReceipt>;
  approvalDecision(approvalId: string, decision: "approved" | "rejected", explanation?: string): Promise<ActionReceipt>;
  openWorkspace(runId: string): Promise<ActionReceipt>;
  debugRun(runId: string): Promise<ActionReceipt>;
}

export interface GatewayTransportConfig {
  baseUri: string;
  authToken?: string;
  transport?: TransportProfile;
  /**
   * Advertised gateway capabilities. Transports use these to select optional
   * features (for example binary WebSocket frames for terminal/log streams)
   * without client-side protocol forks.
   */
  capabilities?: GatewayCapabilities;
}

/** Connection state tracked by the client. */
export type ConnectionState =
  | "disconnected"
  | "connecting"
  | "connected"
  | "degraded"
  | "reconnecting"
  | "failed";

/** Run phase liveness state from the client's perspective. */
export type RunPhaseState =
  | "active"
  | "quiet"
  | "degraded"
  | "stalled"
  | "retry_queued"
  | "cancelled"
  | "detached";

/** Diagnostic info about the current stream health. */
export interface StreamHealth {
  healthy: boolean;
  lastEventAt: string | null;
  reconnectAttempts: number;
  eventsSinceReconnect: number;
}

export type {
  GatewayEnvelope,
  DashboardSnapshot,
  RunDetail,
  RunEventPage,
  TerminalSnapshot,
  TerminalSearchResult,
  TerminalJumpResult,
  TaskGraphSnapshot,
  GatewayCapabilities,
  ActionDispatch,
  ActionReceipt,
  ApprovalRequest,
  PlanningSessionSummary,
  RunStatus,
  ReleaseReason,
  GatewayHealth,
  StreamCursor,
  PageCursor,
  RunTimeline,
  RunLogPage,
  TransportProfile,
  ChangedFileEntry,
  FileDiffPage,
  RunValidationSummary,
  RunAction,
  RunPhase,
  SafeActions,
  RunLifecycleState,
};