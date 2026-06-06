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
} from "@opensymphony/gateway-schema";

export { HttpGatewayTransport } from "./transports.js";
export { MockGatewayTransport } from "./mock.js";
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

/** Transport adapter interface for all gateway communication. */
export interface GatewayTransport {
  readonly baseUri: string;

  health(): Promise<GatewayCapabilities>;
  snapshot(): Promise<DashboardSnapshot>;
  taskGraph(projectId: string): Promise<TaskGraphSnapshot>;
  runDetail(runId: string): Promise<RunDetail>;
  runEvents(runId: string, cursor?: PageCursor): Promise<RunEventPage>;
  terminalSnapshot(runId: string, terminalId: string): Promise<TerminalSnapshot>;

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
}

export interface GatewayTransportConfig {
  baseUri: string;
  authToken?: string;
  transport?: TransportProfile;
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
  TransportProfile,
};