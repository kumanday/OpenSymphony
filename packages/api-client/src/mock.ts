import type {
  GatewayEnvelope,
  DashboardSnapshot,
  RunDetail,
  RunEventPage,
  TerminalSnapshot,
  TaskGraphSnapshot,
  GatewayCapabilities,
  ActionDispatch,
  ActionReceipt,
  PageCursor,
} from "@opensymphony/gateway-schema";
import type { GatewayTransport, ActionCapableTransport } from "./index.js";

/** Deterministic mock transport for tests. */
export class MockGatewayTransport implements GatewayTransport, ActionCapableTransport {
  readonly baseUri: string;

  private mockHealth: GatewayCapabilities;
  private mockSnapshot: DashboardSnapshot;
  private mockTaskGraph: TaskGraphSnapshot;
  private mockRunDetail: Map<string, RunDetail>;
  private mockRunEvents: Map<string, RunEventPage>;
  private mockTerminalSnapshot: Map<string, TerminalSnapshot>;
  private mockEvents: GatewayEnvelope[] = [];
  private mockTerminalFrames: Map<string, GatewayEnvelope[]> = new Map();
  private mockActionReceipts: Map<string, ActionReceipt> = new Map();
  private closedFlag = false;

  // Stream health simulation.
  private streamHealthyFlag = true;
  private lastEventTimestamp: number | null = null;
  private reconnectAttemptsCount = 0;
  private actionCounter = 0;

  constructor(opts?: {
    baseUri?: string;
    health?: Partial<GatewayCapabilities>;
    snapshot?: Partial<DashboardSnapshot>;
    taskGraph?: Partial<TaskGraphSnapshot>;
    runDetails?: RunDetail[];
    runEvents?: RunEventPage[];
    terminalSnapshots?: TerminalSnapshot[];
    events?: GatewayEnvelope[];
    terminalFrames?: { runId: string; frames: GatewayEnvelope[] }[];
    actionReceipts?: { correlationId: string; receipt: ActionReceipt }[];
    streamHealthy?: boolean;
  }) {
    this.baseUri = opts?.baseUri ?? "http://mock-gateway.local";

    this.mockHealth = {
      schema_version: { major: 1, minor: 0, patch: 0 },
      gateway_version: "1.0.0-mock",
      supported_api_versions: ["v1"],
      transports: [
        {
          transport: "loopback_http",
          modes: ["local"],
          supported_encodings: ["utf8"],
          bidirectional: false,
        },
      ],
      features: [
        { feature: "events", available: true, requires_auth: false },
        { feature: "terminal", available: true, requires_auth: false },
        { feature: "actions", available: true, requires_auth: true },
      ],
      auth_modes: ["none", "api_key"],
      max_event_page_size: 1000,
      max_terminal_frame_batch: 500,
      ...(opts?.health ?? {}),
    };

    this.mockSnapshot = {
      schema_version: { major: 1, minor: 0, patch: 0 },
      generated_at: new Date().toISOString(),
      sequence: 1,
      health: "healthy",
      metrics: {
        running_issue_count: 0,
        retry_queue_depth: 0,
        total_input_tokens: 0,
        total_output_tokens: 0,
        total_cache_read_tokens: 0,
        total_cost_micros: 0,
      },
      projects: [],
      recent_events: [],
      ...(opts?.snapshot ?? {}),
    };

    this.mockTaskGraph = {
      schema_version: { major: 1, minor: 0, patch: 0 },
      project_id: "mock-project",
      generated_at: new Date().toISOString(),
      nodes: [],
      root_ids: [],
      ...(opts?.taskGraph ?? {}),
    };

    this.mockRunDetail = new Map();
    for (const run of opts?.runDetails ?? []) {
      this.mockRunDetail.set(run.run_id, run);
    }

    this.mockRunEvents = new Map();
    for (const page of opts?.runEvents ?? []) {
      this.mockRunEvents.set(page.run_id, page);
    }

    this.mockTerminalSnapshot = new Map();
    for (const snap of opts?.terminalSnapshots ?? []) {
      this.mockTerminalSnapshot.set(`${snap.run_id}:${snap.terminal_session_id}`, snap);
    }

    this.mockEvents = opts?.events ?? [];
    for (const tf of opts?.terminalFrames ?? []) {
      this.mockTerminalFrames.set(tf.runId, tf.frames);
    }

    for (const ar of opts?.actionReceipts ?? []) {
      this.mockActionReceipts.set(ar.correlationId, ar.receipt);
    }

    this.streamHealthyFlag = opts?.streamHealthy ?? true;
  }

  // -- REST reads --

  async health(): Promise<GatewayCapabilities> {
    return this.mockHealth;
  }

  async snapshot(): Promise<DashboardSnapshot> {
    return this.mockSnapshot;
  }

  async taskGraph(_projectId: string): Promise<TaskGraphSnapshot> {
    return this.mockTaskGraph;
  }

  async runDetail(runId: string): Promise<RunDetail> {
    const run = this.mockRunDetail.get(runId);
    if (!run) {
      throw new Error(`Mock run not found: ${runId}`);
    }
    return run;
  }

  async runEvents(runId: string, _cursor?: PageCursor): Promise<RunEventPage> {
    const page = this.mockRunEvents.get(runId);
    if (!page) {
      return {
        schema_version: { major: 1, minor: 0, patch: 0 },
        run_id: runId,
        events: [],
      };
    }
    return page;
  }

  async terminalSnapshot(
    runId: string,
    terminalId: string,
  ): Promise<TerminalSnapshot> {
    const key = `${runId}:${terminalId}`;
    const snap = this.mockTerminalSnapshot.get(key);
    if (!snap) {
      throw new Error(`Mock terminal snapshot not found: ${key}`);
    }
    return snap;
  }

  // -- Event streams (deterministic replay) --

  async *events(
    fromCursor?: { sequence: number; partition: string },
  ): AsyncIterable<GatewayEnvelope> {
    // Filter events by partition if specified, otherwise replay all.
    const events = fromCursor?.partition
      ? this.mockEvents.filter((e) => e.cursor.partition === fromCursor.partition)
      : this.mockEvents;

    let startIdx = 0;
    if (fromCursor) {
      // Replay starts AFTER the given cursor (strictly greater).
      startIdx = events.findIndex((e) => e.cursor.sequence > fromCursor.sequence);
      if (startIdx === -1) startIdx = events.length;
    }

    for (let i = startIdx; i < events.length; i++) {
      this.lastEventTimestamp = Date.now();
      this.reconnectAttemptsCount = 0;
      yield events[i];
    }
  }

  async *terminalFrames(runId: string): AsyncIterable<GatewayEnvelope> {
    const frames = this.mockTerminalFrames.get(runId) ?? [];
    for (const frame of frames) {
      this.lastEventTimestamp = Date.now();
      this.reconnectAttemptsCount = 0;
      yield frame;
    }
  }

  // -- Action mutations --

  async dispatchAction(action: ActionDispatch): Promise<ActionReceipt> {
    const receipt = this.mockActionReceipts.get(action.correlation_id);
    if (!receipt) {
      this.actionCounter++;
      return {
        schema_version: { major: 1, minor: 0, patch: 0 },
        action_id: `mock-action-${this.actionCounter}`,
        correlation_id: action.correlation_id,
        status: "accepted",
        expected_events: [],
        issued_at: new Date(1000000000000 + this.actionCounter).toISOString(),
      };
    }
    return receipt;
  }

  async cancelRun(runId: string): Promise<ActionReceipt> {
    return this.dispatchAction({
      schema_version: { major: 1, minor: 0, patch: 0 },
      correlation_id: `cancel-${runId}-${crypto.randomUUID()}`,
      action_kind: "cancel",
      target_entity: { entity_kind: "run", entity_id: runId },
      idempotency_key: `cancel-${runId}`,
    });
  }

  async retryRun(runId: string): Promise<ActionReceipt> {
    return this.dispatchAction({
      schema_version: { major: 1, minor: 0, patch: 0 },
      correlation_id: `retry-${runId}-${crypto.randomUUID()}`,
      action_kind: "retry",
      target_entity: { entity_kind: "run", entity_id: runId },
      idempotency_key: `retry-${runId}`,
    });
  }

  async resumeRun(runId: string): Promise<ActionReceipt> {
    return this.dispatchAction({
      schema_version: { major: 1, minor: 0, patch: 0 },
      correlation_id: `resume-${runId}-${crypto.randomUUID()}`,
      action_kind: "resume",
      target_entity: { entity_kind: "run", entity_id: runId },
      idempotency_key: `resume-${runId}`,
    });
  }

  // -- Lifecycle --

  async close(): Promise<void> {
    this.closedFlag = true;
  }

  // -- Stream health diagnostics --

  isStreamHealthy(): boolean {
    return this.streamHealthyFlag;
  }

  getReconnectAttempts(): number {
    return this.reconnectAttemptsCount;
  }

  // -- Test helpers to modify mock state --

  /** Add an event to the mock event stream. */
  addEvent(event: GatewayEnvelope): void {
    this.mockEvents.push(event);
  }

  /** Add a terminal frame to a mock stream. */
  addTerminalFrame(runId: string, frame: GatewayEnvelope): void {
    const frames = this.mockTerminalFrames.get(runId) ?? [];
    frames.push(frame);
    this.mockTerminalFrames.set(runId, frames);
  }

  /** Set a mock action receipt. */
  setActionReceipt(correlationId: string, receipt: ActionReceipt): void {
    this.mockActionReceipts.set(correlationId, receipt);
  }

  /** Update a run detail. */
  updateRunDetail(run: RunDetail): void {
    this.mockRunDetail.set(run.run_id, run);
  }

  /** Set stream health status for testing degraded scenarios. */
  setStreamHealthy(healthy: boolean): void {
    this.streamHealthyFlag = healthy;
  }

  /** Simulate reconnect attempts. */
  simulateReconnect(attempts: number): void {
    this.reconnectAttemptsCount = attempts;
  }
}
