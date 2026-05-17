import type {
  GatewayEnvelope,
  DashboardSnapshot,
  RunDetail,
  RunEventPage,
  TerminalSnapshot,
  TaskGraphSnapshot,
  GatewayCapabilities,
  TransportProfile,
} from "@opensymphony/gateway-schema";

export { HttpGatewayTransport } from "./transports.js";

/** Transport adapter interface for all gateway communication. */
export interface GatewayTransport {
  readonly baseUri: string;

  health(): Promise<GatewayCapabilities>;
  snapshot(): Promise<DashboardSnapshot>;
  taskGraph(projectId: string): Promise<TaskGraphSnapshot>;
  runDetail(runId: string): Promise<RunDetail>;
  runEvents(runId: string): Promise<RunEventPage>;
  terminalSnapshot(runId: string, terminalId: string): Promise<TerminalSnapshot>;

  /** Subscribe to gateway event stream; returns an async iterable. */
  events(): AsyncIterable<GatewayEnvelope>;

  /** Subscribe to terminal frame stream for a run. */
  terminalFrames(runId: string): AsyncIterable<GatewayEnvelope>;

  close(): Promise<void>;
}

export interface GatewayTransportConfig {
  baseUri: string;
  authToken?: string;
  transport?: TransportProfile;
}