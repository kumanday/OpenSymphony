import type {
  GatewayEnvelope,
  DashboardSnapshot,
  RunDetail,
  RunEventPage,
  TerminalSnapshot,
  TaskGraphSnapshot,
  GatewayCapabilities,
} from "@opensymphony/gateway-schema";
import type { GatewayTransport, GatewayTransportConfig } from "./index.js";

/**
 * Placeholder HTTP-based transport adapter.
 *
 * Real implementation will use fetch() for REST endpoints and EventSource
 * for SSE streams. This stub satisfies the interface for type-checking
 * and allows downstream packages to depend on api-client without errors.
 */
export class HttpGatewayTransport implements GatewayTransport {
  readonly baseUri: string;

  constructor(config: GatewayTransportConfig) {
    this.baseUri = config.baseUri.replace(/\/+$/, "");
  }

  async health(): Promise<GatewayCapabilities> {
    throw new Error("HttpGatewayTransport.health not yet implemented");
  }

  async snapshot(): Promise<DashboardSnapshot> {
    throw new Error("HttpGatewayTransport.snapshot not yet implemented");
  }

  async taskGraph(_projectId: string): Promise<TaskGraphSnapshot> {
    throw new Error("HttpGatewayTransport.taskGraph not yet implemented");
  }

  async runDetail(_runId: string): Promise<RunDetail> {
    throw new Error("HttpGatewayTransport.runDetail not yet implemented");
  }

  async runEvents(_runId: string): Promise<RunEventPage> {
    throw new Error("HttpGatewayTransport.runEvents not yet implemented");
  }

  async terminalSnapshot(
    _runId: string,
    _terminalId: string,
  ): Promise<TerminalSnapshot> {
    throw new Error("HttpGatewayTransport.terminalSnapshot not yet implemented");
  }

  async *events(): AsyncIterable<GatewayEnvelope> {
    throw new Error("HttpGatewayTransport.events not yet implemented");
  }

  async *terminalFrames(_runId: string): AsyncIterable<GatewayEnvelope> {
    throw new Error("HttpGatewayTransport.terminalFrames not yet implemented");
  }

  async close(): Promise<void> {
    // No-op for HTTP transport.
  }
}