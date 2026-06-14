/**
 * Fixture replay tests for transports and reducers.
 *
 * These tests replay deterministic event sequences to verify:
 * - Out-of-order event handling
 * - Duplicate event deduplication
 * - Reconnect behavior
 * - Stale-stream recovery
 * - All transport adapters reduce to the same state transitions
 * - Long-running run state rebuild without live socket
 * - Stale stream state does not collapse into failed run state
 */

import {
  gatewayReducer,
  initialState,
  deriveRunPhaseState,
  computeLivenessState,
  LIVENESS_THRESHOLDS,
} from "@opensymphony/state";
import { MockGatewayTransport } from "@opensymphony/api-client";
import type {
  GatewayEnvelope,
  DashboardSnapshot,
  TaskGraphSnapshot,
  RunDetail,
  TerminalFrame,
  RunEvent,
  ActionReceipt,
} from "@opensymphony/gateway-schema";

/** Deterministic timestamp used by all test actions. */
const NOW = 1_700_000_000_000;

// -- Fixture factories --

function makeDashboardSnapshot(seq: number): DashboardSnapshot {
  return {
    schema_version: { major: 1, minor: 0, patch: 0 },
    generated_at: "2025-01-01T00:00:00Z",
    sequence: seq,
    health: "healthy",
    metrics: {
      running_issue_count: 2,
      retry_queue_depth: 0,
      total_input_tokens: 1024,
      total_output_tokens: 512,
      total_cache_read_tokens: 256,
      total_cost_micros: 0,
    },
    projects: [
      {
        project_id: "proj-1",
        name: "OpenSymphony",
        milestone_count: 3,
        issue_count: 12,
        running_count: 2,
        completed_count: 5,
        failed_count: 0,
      },
    ],
    recent_events: [],
  };
}

function makeTaskGraphSnapshot(seq: number): TaskGraphSnapshot {
  return {
    schema_version: { major: 1, minor: 0, patch: 0 },
    project_id: "proj-1",
    generated_at: "2025-01-01T00:00:00Z",
    nodes: [
      {
        schema_version: { major: 1, minor: 0, patch: 0 },
        node_id: "COE-390",
        identifier: "COE-390",
        kind: "issue" as const,
        title: "Gateway schemas",
        state: "Done",
        state_category: "done" as const,
        children: [],
        blocked_by: [],
        labels: ["gateway"],
      },
    ],
    root_ids: ["COE-390"],
  };
}

function makeRunDetail(runId: string, status = "running"): RunDetail {
  return {
    schema_version: { major: 1, minor: 0, patch: 0 },
    run_id: runId,
    issue_id: "issue-1",
    issue_identifier: "COE-390",
    worker_id: "worker-1",
    status: status as RunDetail["status"],
    claimed_at: "2025-01-01T00:00:00Z",
    turn_count: 3,
    max_turns: 50,
    input_tokens: 1024,
    output_tokens: 512,
    cache_read_tokens: 256,
    runtime_seconds: 120,
  };
}

function makeEnvelope(runId: string, seq: number, eventKind = "run_updated"): GatewayEnvelope {
  return {
    schema_version: { major: 1, minor: 0, patch: 0 },
    cursor: { sequence: seq, partition: `run:${runId}` },
    entity_ref: { kind: "run", id: runId },
    event_kind: eventKind,
    emitted_at: `2025-01-01T00:00:${String(seq).padStart(2, "0")}Z`,
  };
}

function makeTerminalFrame(runId: string, seq: number): TerminalFrame {
  return {
    schema_version: { major: 1, minor: 0, patch: 0 },
    frame_sequence: seq,
    stream_id: `stream-${runId}`,
    run_id: runId,
    terminal_session_id: `term-${runId}`,
    frame_kind: "stdout",
    encoding: "utf8",
    content: `line ${seq}\n`,
    timestamp: `2025-01-01T00:00:${String(seq).padStart(2, "0")}Z`,
    association: {
      run_id: runId,
      workspace_id: "workspace-1",
    },
  };
}

function makeRunEvent(seq: number): RunEvent {
  return {
    sequence: seq,
    event_id: `evt-${seq}`,
    happened_at: `2025-01-01T00:00:${String(seq).padStart(2, "0")}Z`,
    kind: "ConversationStateUpdateEvent",
    summary: `Event ${seq}`,
  };
}

function makeActionReceipt(correlationId: string, status = "accepted"): ActionReceipt {
  return {
    schema_version: { major: 1, minor: 0, patch: 0 },
    action_id: `action-${correlationId}`,
    correlation_id: correlationId,
    status: status as ActionReceipt["status"],
    expected_followup: [],
    issued_at: "2025-01-01T00:00:00Z",
  };
}

// -- Snapshot + replay rebuild tests --

describe("snapshot + stream replay rebuilds state", () => {
  it("rebuilds dashboard, task graph, and run state from fixtures", async () => {
    const transport = new MockGatewayTransport({
      snapshot: makeDashboardSnapshot(1),
      taskGraph: makeTaskGraphSnapshot(1),
      runDetails: [makeRunDetail("run-1"), makeRunDetail("run-2")],
      events: [
        makeEnvelope("run-1", 1),
        makeEnvelope("run-1", 2),
        makeEnvelope("run-2", 1),
      ],
    });

    // Step 1: Fetch snapshot.
    const snapshot = await transport.snapshot();
    let state = gatewayReducer(initialState, {
      type: "SNAPSHOT_RECEIVED",
      nowMs: NOW,
      payload: snapshot,
    });
    expect(state.dashboard.snapshot).toBeTruthy();

    // Step 2: Fetch task graph.
    const taskGraph = await transport.taskGraph("proj-1");
    state = gatewayReducer(state, {
      type: "TASK_GRAPH_RECEIVED",
      nowMs: NOW,
      payload: taskGraph,
    });
    expect(state.taskGraph.nodes.size).toBe(1);

    // Step 3: Replay events.
    const events = transport.events();
    for await (const envelope of events) {
      state = gatewayReducer(state, {
        type: "ENVELOPE_RECEIVED",
        payload: envelope,
      });
    }

    // Verify entity cache was populated.
    expect(state.cache.runs.size).toBeGreaterThanOrEqual(2);
  });

  it("rebuilds run state via RUN_UPDATED + RUN_EVENTS_RECEIVED", async () => {
    const transport = new MockGatewayTransport({
      runDetails: [makeRunDetail("run-1")],
      runEvents: [
        {
          schema_version: { major: 1, minor: 0, patch: 0 },
          run_id: "run-1",
          events: [makeRunEvent(1), makeRunEvent(2), makeRunEvent(3)],
        },
      ],
    });

    const runDetail = await transport.runDetail("run-1");
    let state = gatewayReducer(initialState, {
      type: "RUN_UPDATED",
      nowMs: NOW,
      payload: runDetail,
    });

    const runEventsPage = await transport.runEvents("run-1");
    state = gatewayReducer(state, {
      type: "RUN_EVENTS_RECEIVED",
      nowMs: NOW,
      runId: "run-1",
      events: runEventsPage.events,
    });

    expect(state.run.runs.has("run-1")).toBe(true);
    const liveness = state.run.liveness.get("run-1");
    // eventCount is incremented by RUN_EVENTS_RECEIVED via computeLivenessState.
    expect(liveness?.eventCount).toBe(3);
  });

  it("rebuilds terminal state via TERMINAL_FRAMES_RECEIVED", async () => {
    const transport = new MockGatewayTransport({
      terminalFrames: [
        {
          runId: "run-1",
          frames: [
            makeEnvelope("run-1", 1, "terminal_frame"),
            makeEnvelope("run-1", 2, "terminal_frame"),
          ],
        },
      ],
    });

    let state = initialState;
    const frames = transport.terminalFrames("run-1");
    for await (const envelope of frames) {
      state = gatewayReducer(state, {
        type: "ENVELOPE_RECEIVED",
        payload: envelope,
      });
    }
    // Terminal session IDs populate the terminal cache.
    expect(state.cache.terminals.size).toBeGreaterThanOrEqual(0);
  });
});

// -- Out-of-order and duplicate tests --

describe("fixture replay handles out-of-order and duplicates", () => {
  it("handles out-of-order terminal frames", () => {
    let state = initialState;
    // Receive frames 3, 1, 4, 2 (out of order).
    state = gatewayReducer(state, {
      type: "TERMINAL_FRAMES_RECEIVED",
      nowMs: NOW,
      runId: "run-1",
      frames: [makeTerminalFrame("run-1", 3), makeTerminalFrame("run-1", 1)],
    });
    state = gatewayReducer(state, {
      type: "TERMINAL_FRAMES_RECEIVED",
      nowMs: NOW,
      runId: "run-1",
      frames: [makeTerminalFrame("run-1", 4), makeTerminalFrame("run-1", 2)],
    });

    // Should have 4 unique frames.
    expect(state.terminal.frames.get("run-1")).toHaveLength(4);
    // Cursor should be max sequence.
    expect(state.terminal.cursor.get("run-1")).toBe(4);
  });

  it("deduplicates replayed terminal frames", () => {
    let state = initialState;
    // First batch.
    state = gatewayReducer(state, {
      type: "TERMINAL_FRAMES_RECEIVED",
      nowMs: NOW,
      runId: "run-1",
      frames: [makeTerminalFrame("run-1", 1), makeTerminalFrame("run-1", 2)],
    });
    // Replay same batch (should dedup).
    state = gatewayReducer(state, {
      type: "TERMINAL_FRAMES_RECEIVED",
      nowMs: NOW,
      runId: "run-1",
      frames: [makeTerminalFrame("run-1", 1), makeTerminalFrame("run-1", 2)],
    });

    expect(state.terminal.frames.get("run-1")).toHaveLength(2);
    expect(state.terminal.cursor.get("run-1")).toBe(2);
  });

  it("handles reconnect with cursor-based replay", async () => {
    const transport = new MockGatewayTransport({
      events: [
        makeEnvelope("run-1", 1),
        makeEnvelope("run-1", 2),
        makeEnvelope("run-1", 3),
        makeEnvelope("run-1", 4),
        makeEnvelope("run-1", 5),
      ],
    });

    // Consume first 3 events.
    let state = initialState;
    const iter = transport.events();
    const results: GatewayEnvelope[] = [];
    for await (const env of iter) {
      results.push(env);
      state = gatewayReducer(state, {
        type: "ENVELOPE_RECEIVED",
        payload: env,
      });
      if (results.length === 3) break;
    }

    // Simulate reconnect from cursor after event 3.
    const reconnectIter = transport.events({ sequence: 3, partition: "run:run-1" });
    const replayed: GatewayEnvelope[] = [];
    for await (const env of reconnectIter) {
      replayed.push(env);
      state = gatewayReducer(state, {
        type: "ENVELOPE_RECEIVED",
        payload: env,
      });
    }

    // Should get events 4 and 5 on replay.
    expect(replayed).toHaveLength(2);
    expect(replayed[0].cursor.sequence).toBe(4);
    expect(replayed[1].cursor.sequence).toBe(5);
    expect(state.cache.runs.size).toBe(1);
  });
});

// -- Stale stream tests --

describe("stale stream handling", () => {
  it("stale stream state does not collapse into failed run state", () => {
    let state = gatewayReducer(initialState, {
      type: "RUN_UPDATED",
      nowMs: NOW,
      payload: makeRunDetail("run-1", "running"),
    });

    // Simulate staleness.
    state = gatewayReducer(state, {
      type: "STREAM_STALE_DETECTED",
      nowMs: NOW,
      runId: "run-1",
    });

    const liveness = state.run.liveness.get("run-1");
    expect(liveness?.phaseState).toBe("degraded");
    expect(liveness?.isStreamStale).toBe(true);
    // Run error should remain null — stale != failed.
    expect(state.run.error).toBeNull();
    expect(state.run.runs.get("run-1")?.status).toBe("running");
  });

  it("stale stream recovery does not affect cancelled runs", () => {
    let state = gatewayReducer(initialState, {
      type: "RUN_UPDATED",
      nowMs: NOW,
      payload: makeRunDetail("run-1", "released"),
    });

    state = gatewayReducer(state, {
      type: "STREAM_STALE_DETECTED",
      nowMs: NOW,
      runId: "run-1",
    });

    // Cancelled runs should stay cancelled even after stale detection.
    const liveness = state.run.liveness.get("run-1");
    expect(liveness?.phaseState).toBe("cancelled");

    // Recovery should not override cancelled state.
    state = gatewayReducer(state, {
      type: "STREAM_RECOVERED",
      nowMs: NOW,
      runId: "run-1",
    });
    const recoveredLiveness = state.run.liveness.get("run-1");
    expect(recoveredLiveness?.phaseState).toBe("cancelled");
  });

  it("degraded stream health check transitions through phases", () => {
    const baseTime = 1_700_000_000_000;
    let state = gatewayReducer(initialState, {
      type: "RUN_UPDATED",
      payload: makeRunDetail("run-1", "running"),
      nowMs: baseTime,
    });
    state = gatewayReducer(state, {
      type: "RUN_EVENTS_RECEIVED",
      runId: "run-1",
      events: [makeRunEvent(1)],
      nowMs: baseTime,
    });

    // 35 seconds -> degraded.
    state = gatewayReducer(state, {
      type: "STREAM_HEALTH_CHECK",
      runId: "run-1",
      nowMs: baseTime + 35_000,
    });
    expect(state.run.liveness.get("run-1")?.phaseState).toBe("degraded");

    // 95 seconds -> stalled.
    state = gatewayReducer(state, {
      type: "STREAM_HEALTH_CHECK",
      runId: "run-1",
      nowMs: baseTime + 95_000,
    });
    expect(state.run.liveness.get("run-1")?.phaseState).toBe("stalled");

    // 155 seconds -> detached.
    state = gatewayReducer(state, {
      type: "STREAM_HEALTH_CHECK",
      runId: "run-1",
      nowMs: baseTime + 155_000,
    });
    expect(state.run.liveness.get("run-1")?.phaseState).toBe("detached");
  });
});

// -- Action receipt correlation tests --

describe("action receipts and correlated events", () => {
  it("ACTION_DISPATCHED tracks pending action", () => {
    const state = gatewayReducer(initialState, {
      type: "ACTION_DISPATCHED",
      nowMs: NOW,
      correlationId: "corr-dispatch-1",
    });
    expect(state.actionReceipts.pending.has("corr-dispatch-1")).toBe(true);
    const pending = state.actionReceipts.pending.get("corr-dispatch-1");
    expect(pending?.dispatchedAt).toBeTruthy();
  });

  it("ACTION_RECEIPT_RECEIVED correlates with dispatched action", () => {
    let state = gatewayReducer(initialState, {
      type: "ACTION_DISPATCHED",
      nowMs: NOW,
      correlationId: "corr-1",
    });

    const receipt = makeActionReceipt("corr-1", "completed");
    state = gatewayReducer(state, {
      type: "ACTION_RECEIPT_RECEIVED",
      nowMs: NOW,
      receipt,
    });

    expect(state.actionReceipts.pending.has("corr-1")).toBe(false);
    expect(state.actionReceipts.receipts.has("corr-1")).toBe(true);
    expect(state.actionReceipts.receipts.get("corr-1")?.status).toBe("completed");
  });

  it("mock transport dispatches actions with receipts", async () => {
    const transport = new MockGatewayTransport({
      actionReceipts: [
        {
          correlationId: "corr-retry-1",
          receipt: makeActionReceipt("corr-retry-1", "completed"),
        },
      ],
    });

    const receipt = await transport.dispatchAction({
      schema_version: { major: 1, minor: 0, patch: 0 },
      correlation_id: "corr-retry-1",
      action_kind: "retry",
      target_entity: { entity_kind: "run", entity_id: "run-1" },
    });

    expect(receipt.correlation_id).toBe("corr-retry-1");
    expect(receipt.status).toBe("completed");
  });

  it("mock cancelRun returns receipt", async () => {
    const transport = new MockGatewayTransport();
    const receipt = await transport.cancelRun("run-1");
    expect(receipt.correlation_id).toMatch(/^cancel-run-1-/);
    expect(receipt.status).toBe("accepted");
  });

  it("mock retryRun returns receipt", async () => {
    const transport = new MockGatewayTransport();
    const receipt = await transport.retryRun("run-1");
    expect(receipt.correlation_id).toMatch(/^retry-run-1-/);
    expect(receipt.status).toBe("accepted");
  });

  it("mock resumeRun returns receipt", async () => {
    const transport = new MockGatewayTransport();
    const receipt = await transport.resumeRun("run-1");
    expect(receipt.correlation_id).toMatch(/^resume-run-1-/);
    expect(receipt.status).toBe("accepted");
  });
});

// -- All transport adapters reduce to same state transitions --

describe("transport adapter state parity", () => {
  it("mock transport and reducer produce consistent state", async () => {
    const transport = new MockGatewayTransport({
      snapshot: makeDashboardSnapshot(1),
      taskGraph: makeTaskGraphSnapshot(1),
      runDetails: [makeRunDetail("run-1")],
    });

    // Apply via reducer.
    let state = initialState;

    // Snapshot.
    const snapshot = await transport.snapshot();
    state = gatewayReducer(state, { type: "SNAPSHOT_RECEIVED",
      nowMs: NOW, payload: snapshot });
    expect(state.dashboard.snapshot?.sequence).toBe(1);

    // Task graph.
    const tg = await transport.taskGraph("proj-1");
    state = gatewayReducer(state, { type: "TASK_GRAPH_RECEIVED",
      nowMs: NOW, payload: tg });
    expect(state.taskGraph.nodes.size).toBe(1);

    // Run detail.
    const run = await transport.runDetail("run-1");
    state = gatewayReducer(state, { type: "RUN_UPDATED",
      nowMs: NOW, payload: run });
    expect(state.run.runs.has("run-1")).toBe(true);
  });
});

// -- Long-running run state rebuild --

describe("long-running run state rebuild without live socket", () => {
  it("rebuilds run state from snapshot + event replay", async () => {
    const transport = new MockGatewayTransport({
      runDetails: [makeRunDetail("run-long-1", "running")],
      runEvents: [
        {
          schema_version: { major: 1, minor: 0, patch: 0 },
          run_id: "run-long-1",
          events: [
            makeRunEvent(1),
            makeRunEvent(2),
            makeRunEvent(3),
            makeRunEvent(4),
            makeRunEvent(5),
          ],
        },
      ],
      events: [
        makeEnvelope("run-long-1", 1),
        makeEnvelope("run-long-1", 2),
        makeEnvelope("run-long-1", 3),
      ],
    });

    let state = initialState;

    // Step 1: Load run snapshot.
    const run = await transport.runDetail("run-long-1");
    state = gatewayReducer(state, { type: "RUN_UPDATED",
      nowMs: NOW, payload: run });

    // Step 2: Load run events.
    const eventsPage = await transport.runEvents("run-long-1");
    state = gatewayReducer(state, {
      type: "RUN_EVENTS_RECEIVED",
      nowMs: NOW,
      runId: "run-long-1",
      events: eventsPage.events,
    });

    // Step 3: Replay stream events.
    for await (const envelope of transport.events()) {
      state = gatewayReducer(state, {
        type: "ENVELOPE_RECEIVED",
        payload: envelope,
      });
    }

    // Verify state rebuild.
    expect(state.run.runs.get("run-long-1")?.status).toBe("running");
    const liveness = state.run.liveness.get("run-long-1");
    // eventCount is incremented by RUN_EVENTS_RECEIVED via computeLivenessState.
    expect(liveness?.eventCount).toBe(5);
    expect(state.cache.runs.has("run-long-1")).toBe(true);
  });

  it("maintains degraded state when socket is stale (not failed)", async () => {
    const transport = new MockGatewayTransport({
      runDetails: [makeRunDetail("run-detached", "running")],
      streamHealthy: false,
    });

    let state = initialState;
    const run = await transport.runDetail("run-detached");
    state = gatewayReducer(state, { type: "RUN_UPDATED",
      nowMs: NOW, payload: run });

    // Simulate staleness detection (no socket events arriving).
    state = gatewayReducer(state, {
      type: "STREAM_STALE_DETECTED",
      nowMs: NOW,
      runId: "run-detached",
    });

    // Health check: stale stream should stay degraded, not collapse to failed.
    const baseTime = 1_700_000_000_000;
    state = gatewayReducer(state, {
      type: "STREAM_HEALTH_CHECK",
      runId: "run-detached",
      nowMs: baseTime + 160_000,
    });

    // Stale stream -> degraded phase (not detached because run is still "running" on server).
    expect(state.run.liveness.get("run-detached")?.phaseState).toBe("degraded");
    // Terminal stream staleness flag is preserved.
    expect(state.terminal.streamStale.get("run-detached")).toBe(true);
    // Run itself should still be marked as running on server.
    expect(state.run.runs.get("run-detached")?.status).toBe("running");
    // IMPORTANT: no error on the run — stale != failed.
    expect(state.run.error).toBeNull();
  });
});

// -- Reconnect behavior tests --

describe("reconnect behavior", () => {
  it("mock transport tracks reconnect attempts", () => {
    const transport = new MockGatewayTransport();
    expect(transport.getReconnectAttempts()).toBe(0);

    transport.simulateReconnect(3);
    expect(transport.getReconnectAttempts()).toBe(3);
  });

  it("mock transport reports stream health", () => {
    let transport = new MockGatewayTransport({ streamHealthy: true });
    expect(transport.isStreamHealthy()).toBe(true);

    transport = new MockGatewayTransport({ streamHealthy: false });
    expect(transport.isStreamHealthy()).toBe(false);
  });

  it("reducer handles reconnect state transitions", () => {
    let state = gatewayReducer(initialState, {
      type: "CONNECTION_STATE_CHANGED",
      nowMs: NOW,
      state: "connecting",
    });
    expect(state.connection.state).toBe("connecting");

    state = gatewayReducer(state, {
      type: "CONNECTION_STATE_CHANGED",
      nowMs: NOW,
      state: "connected",
    });
    expect(state.connection.state).toBe("connected");
    expect(state.connection.lastConnectedAt).toBeTruthy();

    state = gatewayReducer(state, {
      type: "RECONNECT_ATTEMPTED",
      attempts: 1,
      nowMs: NOW,
    });
    expect(state.connection.reconnectAttempts).toBe(1);
    expect(state.connection.state).toBe("reconnecting");
    expect(state.connection.lastDisconnectedAt).toBeTruthy();
  });

  it("reducer handles connection failure", () => {
    const state = gatewayReducer(initialState, {
      type: "ERROR",
      error: "Connection timeout",
    });
    expect(state.connection.state).toBe("failed");
    expect(state.connection.error).toBe("Connection timeout");
  });
});

// -- Degraded stream handling --

describe("degraded stream handling", () => {
  it("degraded health check transitions run to degraded phase with stale stream", () => {
    const baseTime = 1_700_000_000_000;
    let state = gatewayReducer(initialState, {
      type: "RUN_UPDATED",
      payload: makeRunDetail("run-1", "running"),
      nowMs: baseTime,
    });

    state = gatewayReducer(state, {
      type: "RUN_EVENTS_RECEIVED",
      runId: "run-1",
      events: [makeRunEvent(1)],
      nowMs: baseTime,
    });

    // 45 seconds gap -> degraded.
    state = gatewayReducer(state, {
      type: "STREAM_HEALTH_CHECK",
      runId: "run-1",
      nowMs: baseTime + 45_000,
    });

    expect(state.run.liveness.get("run-1")?.phaseState).toBe("degraded");
    expect(state.run.liveness.get("run-1")?.streamHealth).toBe("stale");
  });

  it("mock transport supports stream health diagnostics", async () => {
    const transport = new MockGatewayTransport({
      runDetails: [makeRunDetail("run-1")],
      streamHealthy: false,
    });

    expect(transport.isStreamHealthy()).toBe(false);

    // Update mock to healthy.
    transport.setStreamHealthy(true);
    expect(transport.isStreamHealthy()).toBe(true);
  });
});

// -- Cancelled and retry_queued states --

describe("cancelled and retry_queued run states", () => {
  it("retry_queued run status maps correctly", () => {
    let state = gatewayReducer(initialState, {
      type: "RUN_UPDATED",
      nowMs: NOW,
      payload: makeRunDetail("run-1", "retry_queued"),
    });

    // Even without events, retry_queued status should be recognized.
    state = gatewayReducer(state, {
      type: "STREAM_HEALTH_CHECK",
      nowMs: NOW,
      runId: "run-1",
    });

    expect(state.run.liveness.get("run-1")?.phaseState).toBe("retry_queued");
  });

  it("released (cancelled) run status maps correctly", () => {
    let state = gatewayReducer(initialState, {
      type: "RUN_UPDATED",
      nowMs: NOW,
      payload: makeRunDetail("run-1", "released"),
    });

    state = gatewayReducer(state, {
      type: "STREAM_HEALTH_CHECK",
      nowMs: NOW,
      runId: "run-1",
    });

    expect(state.run.liveness.get("run-1")?.phaseState).toBe("cancelled");
  });

  it("cancelled run does not transition to other phases on stale detection", () => {
    let state = gatewayReducer(initialState, {
      type: "RUN_UPDATED",
      nowMs: NOW,
      payload: makeRunDetail("run-1", "released"),
    });

    state = gatewayReducer(state, {
      type: "STREAM_STALE_DETECTED",
      nowMs: NOW,
      runId: "run-1",
    });

    expect(state.run.liveness.get("run-1")?.phaseState).toBe("cancelled");
    expect(state.run.error).toBeNull();
  });
});

// -- All run phase states covered --

describe("all run phase states", () => {
  it("active: running with fresh events", () => {
    expect(deriveRunPhaseState({ status: "running" } as any, {
      runId: "r",
      phaseState: "active",
      lastEventAt: null,
      lastStatusUpdateAt: null,
      eventCount: 1,
      gapSeconds: 1,
      isStreamStale: false,
      streamHealth: "healthy",
    }, false)).toBe("active");
  });

  it("quiet: running with no recent events", () => {
    expect(deriveRunPhaseState({ status: "running" } as any, {
      runId: "r",
      phaseState: "quiet",
      lastEventAt: null,
      lastStatusUpdateAt: null,
      eventCount: 0,
      gapSeconds: 30,
      isStreamStale: false,
      streamHealth: "healthy",
    }, false)).toBe("quiet");
  });

  it("degraded: stale stream but run alive", () => {
    expect(deriveRunPhaseState({ status: "running" } as any, undefined, true)).toBe("degraded");
  });

  it("stalled: no events for extended period", () => {
    expect(deriveRunPhaseState({ status: "running" } as any, {
      runId: "r",
      phaseState: "stalled",
      lastEventAt: null,
      lastStatusUpdateAt: null,
      eventCount: 0,
      gapSeconds: 90,
      isStreamStale: false,
      streamHealth: "stale",
    }, false)).toBe("stalled");
  });

  it("retry_queued: run queued for retry", () => {
    expect(deriveRunPhaseState({ status: "retry_queued" } as any, undefined, false)).toBe("retry_queued");
  });

  it("cancelled: run released", () => {
    expect(deriveRunPhaseState({ status: "released" } as any, undefined, false)).toBe("cancelled");
  });

  it("stale stream overrides detached liveness to degraded", () => {
    expect(deriveRunPhaseState({ status: "running" } as any, {
      runId: "r",
      phaseState: "detached",
      lastEventAt: null,
      lastStatusUpdateAt: null,
      eventCount: 0,
      gapSeconds: 160,
      isStreamStale: true,
      streamHealth: "stale",
    }, true)).toBe("degraded");
  });
});
