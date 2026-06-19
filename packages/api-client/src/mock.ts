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
  TaskGraphNode,
  TaskGraphCreatePayload,
  TaskGraphUpdatePayload,
  TaskGraphDependencyPayload,
  TaskGraphCommentPayload,
  RunTimeline,
  RunLogPage,
  TerminalSearchResult,
  TerminalJumpResult,
  ChangedFileEntry,
  FileDiffPage,
  RunValidationSummary,
  ApprovalRequest,
  AuthErrorCode,
} from "@opensymphony/gateway-schema";
import type { GatewayTransport, ActionCapableTransport } from "./index.js";
import { stableHash, stableHashJson } from "./util.js";
import { GatewayRequestError } from "./errors.js";

/** Deterministic mock transport for tests. */
export class MockGatewayTransport implements GatewayTransport, ActionCapableTransport {
  readonly baseUri: string;

  private mockHealth: GatewayCapabilities;
  private mockSnapshot: DashboardSnapshot;
  private mockTaskGraph: TaskGraphSnapshot;
  private mockRunDetail: Map<string, RunDetail>;
  private mockRunEvents: Map<string, RunEventPage>;
  private mockRunFiles: Map<string, ChangedFileEntry[]> = new Map();
  private mockRunDiffs: Map<string, FileDiffPage> = new Map();
  private mockRunApprovals: Map<string, ApprovalRequest[]> = new Map();
  private mockRunValidation: Map<string, RunValidationSummary> = new Map();
  private mockTerminalSnapshot: Map<string, TerminalSnapshot>;
  private mockEvents: GatewayEnvelope[] = [];
  private mockTerminalFrames: Map<string, GatewayEnvelope[]> = new Map();
  private mockActionReceipts: Map<string, ActionReceipt> = new Map();
  private closedFlag = false;

  // Auth failure simulation for placeholder-state tests.
  private authFailureCode: AuthErrorCode | null = null;
  private authFailureMethods: Set<"health" | "snapshot"> = new Set();

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
    runFiles?: { runId: string; files: ChangedFileEntry[] }[];
    runDiffs?: { runId: string; filePath?: string; diff: FileDiffPage }[];
    runApprovals?: { runId: string; approvals: ApprovalRequest[] }[];
    runValidation?: { runId: string; summary: RunValidationSummary }[];
    terminalSnapshots?: TerminalSnapshot[];
    events?: GatewayEnvelope[];
    terminalFrames?: { runId: string; frames: GatewayEnvelope[] }[];
    actionReceipts?: { correlationId: string; receipt: ActionReceipt }[];
    streamHealthy?: boolean;
    /**
     * Simulate a classified auth failure for one or more read methods.
     * `methods: ["snapshot"]` simulates a gateway that advertises auth
     * (health succeeds) but rejects the dashboard snapshot because the
     * caller is not signed in / lacks access.
     */
    authFailure?: {
      code: AuthErrorCode;
      methods?: Array<"health" | "snapshot">;
    };
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

    for (const entry of opts?.runFiles ?? []) {
      this.mockRunFiles.set(entry.runId, entry.files);
    }

    for (const entry of opts?.runDiffs ?? []) {
      const key = entry.filePath ? `${entry.runId}:${entry.filePath}` : entry.runId;
      this.mockRunDiffs.set(key, entry.diff);
    }

    for (const entry of opts?.runApprovals ?? []) {
      this.mockRunApprovals.set(entry.runId, entry.approvals);
    }

    for (const entry of opts?.runValidation ?? []) {
      this.mockRunValidation.set(entry.runId, entry.summary);
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

    if (opts?.authFailure) {
      this.authFailureCode = opts.authFailure.code;
      this.authFailureMethods = new Set(opts.authFailure.methods ?? ["snapshot"]);
    }
  }

  // -- REST reads --

  async health(): Promise<GatewayCapabilities> {
    this.throwIfAuthFailure("health");
    return this.mockHealth;
  }

  async snapshot(): Promise<DashboardSnapshot> {
    this.throwIfAuthFailure("snapshot");
    return this.mockSnapshot;
  }

  /**
   * Remove the simulated auth failure so a refresh can succeed.
   *
   * Test/simulation-only: `MockGatewayTransport` is not used in production, so
   * this clears the injected failure, not a real gateway failure.
   */
  clearAuthFailure(): void {
    this.authFailureCode = null;
    this.authFailureMethods = new Set();
  }

  private throwIfAuthFailure(method: "health" | "snapshot"): void {
    if (this.authFailureCode && this.authFailureMethods.has(method)) {
      const status = this.authFailureCode === "unauthenticated" ? 401 : 403;
      throw new GatewayRequestError(
        this.authFailureCode,
        `Simulated ${this.authFailureCode} for ${method}`,
        status,
      );
    }
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

  async runTimeline(_runId: string): Promise<RunTimeline> {
    throw new Error("MockGatewayTransport.runTimeline not implemented");
  }

  async runLogs(_runId: string, _cursor?: number, _limit?: number): Promise<RunLogPage> {
    throw new Error("MockGatewayTransport.runLogs not implemented");
  }

  async runFiles(runId: string): Promise<ChangedFileEntry[]> {
    return this.mockRunFiles.get(runId) ?? [];
  }

  async runDiffs(runId: string, filePath?: string): Promise<FileDiffPage> {
    const key = filePath ? `${runId}:${filePath}` : runId;
    return (
      this.mockRunDiffs.get(key) ?? {
        schema_version: { major: 1, minor: 0, patch: 0 },
        run_id: runId,
        file_path: filePath ?? "",
        hunks: [],
        total_lines_added: 0,
        total_lines_removed: 0,
      }
    );
  }

  async runApprovals(runId: string): Promise<ApprovalRequest[]> {
    return this.mockRunApprovals.get(runId) ?? [];
  }

  async runValidation(runId: string): Promise<RunValidationSummary> {
    return (
      this.mockRunValidation.get(runId) ?? {
        schema_version: { major: 1, minor: 0, patch: 0 },
        run_id: runId,
        generated_at: new Date().toISOString(),
        overall_status: "pending",
        commands: [],
        evidence: [],
      }
    );
  }

  async terminalSnapshot(
    runId: string,
    terminalId: string,
    _cursor?: number,
  ): Promise<TerminalSnapshot> {
    const key = `${runId}:${terminalId}`;
    const snap = this.mockTerminalSnapshot.get(key);
    if (!snap) {
      throw new Error(`Mock terminal snapshot not found: ${key}`);
    }
    return snap;
  }

  async terminalSearch(
    _runId: string,
    _terminalId: string,
    _query: string,
  ): Promise<TerminalSearchResult> {
    throw new Error("MockGatewayTransport.terminalSearch not implemented");
  }

  async terminalJumpToEvent(
    _runId: string,
    _terminalId: string,
    _eventId: string,
  ): Promise<TerminalJumpResult> {
    throw new Error("MockGatewayTransport.terminalJumpToEvent not implemented");
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
    if (receipt) return receipt;

    this.actionCounter++;
    const baseReceipt: ActionReceipt = {
      schema_version: { major: 1, minor: 0, patch: 0 },
      action_id: `mock-action-${this.actionCounter}`,
      correlation_id: action.correlation_id,
      status: "accepted",
      expected_followup: [],
      issued_at: new Date(1000000000000 + this.actionCounter).toISOString(),
    };

    // Simulate task graph mutation acknowledgements and event replay.
    const payload = action.payload as Record<string, unknown> | undefined;
    if (action.action_kind === "create_followup" && payload && typeof payload.title === "string") {
      const create = payload as unknown as TaskGraphCreatePayload;
      const nodeId = `mock-${create.kind}-${this.actionCounter}`;
      const newNode: TaskGraphNode = {
        schema_version: { major: 1, minor: 0, patch: 0 },
        node_id: nodeId,
        kind: create.kind,
        identifier: create.identifier ?? nodeId,
        title: create.title,
        state: create.state ?? "Todo",
        state_category: this.stateToCategory(create.state ?? "Todo"),
        parent_id: create.parent_id,
        children: [],
        blocked_by: [],
        labels: create.labels ?? [],
        priority: create.priority,
        estimate_minutes: create.estimate_minutes,
        created_at: new Date().toISOString(),
        updated_at: new Date().toISOString(),
      };
      this.mockTaskGraph.nodes.push(newNode);
      if (create.parent_id) {
        const parent = this.mockTaskGraph.nodes.find((n) => n.node_id === create.parent_id);
        if (parent && !parent.children.includes(nodeId)) {
          parent.children.push(nodeId);
        }
      } else if (!this.mockTaskGraph.root_ids.includes(nodeId)) {
        this.mockTaskGraph.root_ids.push(nodeId);
      }
      baseReceipt.result = { node_id: nodeId, updated_at: newNode.updated_at, applied: true };
      this.emitTaskGraphEvent();
    }

    if (action.action_kind === "update_node" && payload && typeof payload.node_id === "string") {
      const update = payload as unknown as TaskGraphUpdatePayload;
      const node = this.mockTaskGraph.nodes.find((n) => n.node_id === update.node_id);
      if (node) {
        if (update.title !== undefined) node.title = update.title;
        if (update.state !== undefined) {
          node.state = update.state;
          node.state_category = this.stateToCategory(update.state);
        }
        if (update.priority !== undefined) node.priority = update.priority;
        if (update.estimate_minutes !== undefined) node.estimate_minutes = update.estimate_minutes;
        if (update.labels !== undefined) node.labels = update.labels;
        node.updated_at = new Date().toISOString();
        baseReceipt.result = { node_id: node.node_id, updated_at: node.updated_at, applied: true };
        this.emitTaskGraphEvent();
      }
    }

    if (action.action_kind === "comment" && payload && typeof payload.node_id === "string" && typeof payload.body === "string") {
      // Comment / evidence action.
      const comment = payload as unknown as TaskGraphCommentPayload;
      const node = this.mockTaskGraph.nodes.find((n) => n.node_id === comment.node_id);
      if (node) {
        node.comment_count = (node.comment_count ?? 0) + 1;
        node.updated_at = new Date().toISOString();
        baseReceipt.result = { node_id: node.node_id, updated_at: node.updated_at, applied: true };
        this.emitTaskGraphEvent();
      }
    }

    if (action.action_kind === "transition_issue" && payload && typeof payload.node_id === "string") {
      const deps = payload as unknown as TaskGraphDependencyPayload;
      const node = this.mockTaskGraph.nodes.find((n) => n.node_id === deps.node_id);
      if (node) {
        node.blocked_by = deps.blocked_by;
        node.updated_at = new Date().toISOString();
        baseReceipt.result = { node_id: node.node_id, updated_at: node.updated_at, applied: true };
        this.emitTaskGraphEvent();
      }
    }

    return baseReceipt;
  }

  private stateToCategory(state: string): TaskGraphNode["state_category"] {
    const lower = state.toLowerCase();
    if (lower === "done" || lower === "completed") return "done";
    if (lower === "in progress" || lower === "in_progress" || lower === "started") return "in_progress";
    if (lower === "canceled" || lower === "cancelled") return "canceled";
    if (lower === "backlog") return "backlog";
    return "todo";
  }

  private emitTaskGraphEvent(): void {
    this.mockEvents.push({
      schema_version: { major: 1, minor: 0, patch: 0 },
      cursor: { sequence: this.mockEvents.length + 1, partition: "task_graph" },
      entity_ref: { kind: "project", id: this.mockTaskGraph.project_id },
      event_kind: "task_graph_updated",
      emitted_at: new Date().toISOString(),
      payload: { project_id: this.mockTaskGraph.project_id, nodes: this.mockTaskGraph.nodes },
    });
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

  async rehydrateRun(runId: string): Promise<ActionReceipt> {
    return this.dispatchAction({
      schema_version: { major: 1, minor: 0, patch: 0 },
      correlation_id: `rehydrate-${runId}-${crypto.randomUUID()}`,
      action_kind: "rehydrate",
      target_entity: { entity_kind: "run", entity_id: runId },
      idempotency_key: `rehydrate-${runId}`,
    });
  }

  async commentRun(runId: string, text: string): Promise<ActionReceipt> {
    return this.dispatchAction({
      schema_version: { major: 1, minor: 0, patch: 0 },
      correlation_id: `comment-${runId}-${crypto.randomUUID()}`,
      action_kind: "comment",
      target_entity: { entity_kind: "run", entity_id: runId },
      payload: { text },
      idempotency_key: `comment-${runId}-${await stableHash(text)}`,
    });
  }

  async createFollowup(runId: string, payload: unknown): Promise<ActionReceipt> {
    return this.dispatchAction({
      schema_version: { major: 1, minor: 0, patch: 0 },
      correlation_id: `followup-${runId}-${crypto.randomUUID()}`,
      action_kind: "create_followup",
      target_entity: { entity_kind: "run", entity_id: runId },
      payload,
      idempotency_key: `followup-${runId}-${await stableHashJson(payload)}`,
    });
  }

  async approvalDecision(
    approvalId: string,
    decision: "approved" | "rejected",
    explanation?: string,
  ): Promise<ActionReceipt> {
    return this.dispatchAction({
      schema_version: { major: 1, minor: 0, patch: 0 },
      correlation_id: `approval-${approvalId}-${crypto.randomUUID()}`,
      action_kind: "approval_decision",
      target_entity: { entity_kind: "approval", entity_id: approvalId },
      payload: { decision, explanation },
      idempotency_key: `approval-${approvalId}-${decision}`,
    });
  }

  async openWorkspace(runId: string): Promise<ActionReceipt> {
    return this.dispatchAction({
      schema_version: { major: 1, minor: 0, patch: 0 },
      correlation_id: `workspace-${runId}-${crypto.randomUUID()}`,
      action_kind: "open_workspace",
      target_entity: { entity_kind: "run", entity_id: runId },
      idempotency_key: `workspace-${runId}`,
    });
  }

  async debugRun(runId: string): Promise<ActionReceipt> {
    return this.dispatchAction({
      schema_version: { major: 1, minor: 0, patch: 0 },
      correlation_id: `debug-${runId}-${crypto.randomUUID()}`,
      action_kind: "debug",
      target_entity: { entity_kind: "run", entity_id: runId },
      idempotency_key: `debug-${runId}`,
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

  /** Set mock changed files for a run. */
  setRunFiles(runId: string, files: ChangedFileEntry[]): void {
    this.mockRunFiles.set(runId, files);
  }

  /** Set mock diff for a run (or run+file path). */
  setRunDiff(runId: string, filePath: string | undefined, diff: FileDiffPage): void {
    const key = filePath ? `${runId}:${filePath}` : runId;
    this.mockRunDiffs.set(key, diff);
  }

  /** Set mock approvals for a run. */
  setRunApprovals(runId: string, approvals: ApprovalRequest[]): void {
    this.mockRunApprovals.set(runId, approvals);
  }

  /** Set mock validation summary for a run. */
  setRunValidation(runId: string, summary: RunValidationSummary): void {
    this.mockRunValidation.set(runId, summary);
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
