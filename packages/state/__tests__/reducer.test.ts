/** Reducer unit tests for @opensymphony/state. */

import {
  gatewayReducer,
  initialState,
  deriveRunPhaseState,
  computeLivenessState,
  computeSafeActions,
  LIVENESS_THRESHOLDS,
} from "@opensymphony/state";
import type {
  DashboardSnapshot,
  TaskGraphSnapshot,
  TaskGraphNode,
  RunDetail,
  TerminalFrame,
  ApprovalRequest,
  PlanningSessionSummary,
  GatewayEnvelope,
  ActionReceipt,
  RunEvent,
  SafeActions,
  RunPhase,
  RunStreamLiveness,
} from "@opensymphony/gateway-schema";

/** Deterministic timestamp used by all test actions. */
const NOW = 1_700_000_000_000;

// -- Helpers — typed factories aligned with gateway-schema interfaces --

function makeSnapshot(): DashboardSnapshot {
  return {
    schema_version: { major: 1, minor: 0, patch: 0 },
    generated_at: "2025-01-01T00:00:00Z",
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
  };
}

function makeTaskGraphSnapshot(): TaskGraphSnapshot {
  return {
    schema_version: { major: 1, minor: 0, patch: 0 },
    project_id: "proj-1",
    generated_at: "2025-01-01T00:00:00Z",
    nodes: [],
    root_ids: [],
  };
}

function makeNode(nodeId: string, parentId?: string): TaskGraphNode {
  return {
    schema_version: { major: 1, minor: 0, patch: 0 },
    node_id: nodeId,
    kind: "issue",
    identifier: nodeId.toUpperCase(),
    title: `Issue ${nodeId}`,
    state: "Todo",
    state_category: "todo",
    parent_id: parentId,
    children: [],
    blocked_by: [],
    labels: [],
  };
}

function makeRunDetail(status = "running"): RunDetail {
  return {
    schema_version: { major: 1, minor: 0, patch: 0 },
    run_id: "run-1",
    issue_id: "issue-1",
    issue_identifier: "COE-001",
    worker_id: "worker-1",
    status: status as RunDetail["status"],
    claimed_at: "2025-01-01T00:00:00Z",
    turn_count: 0,
    max_turns: 50,
    input_tokens: 0,
    output_tokens: 0,
    cache_read_tokens: 0,
    runtime_seconds: 0,
  };
}

function makeFrame(sequence: number): TerminalFrame {
  return {
    schema_version: { major: 1, minor: 0, patch: 0 },
    frame_sequence: sequence,
    stream_id: "stream-1",
    run_id: "run-1",
    terminal_session_id: "term-1",
    frame_kind: "stdout",
    encoding: "utf8",
    content: `line ${sequence}`,
    timestamp: "2025-01-01T00:00:00Z",
    association: {
      run_id: "run-1",
      workspace_id: "workspace-1",
    },
  };
}

function makeApproval(id: string): ApprovalRequest {
  return {
    schema_version: { major: 1, minor: 0, patch: 0 },
    approval_id: id,
    run_id: "run-1",
    issue_id: "issue-1",
    kind: "tool_use",
    title: "Approve action",
    description: "Should we proceed?",
    requested_at: "2025-01-01T00:00:00Z",
    status: "pending",
    correlation_id: "corr-1",
  };
}

function makePlanningSummary(): PlanningSessionSummary {
  return {
    schema_version: { major: 1, minor: 0, patch: 0 },
    session_id: "sess-1",
    project_id: "proj-1",
    title: "Planning session",
    status: "draft",
    artifact_count: 0,
    created_at: "2025-01-01T00:00:00Z",
    updated_at: "2025-01-01T00:00:00Z",
  };
}

function makeEnvelope(eventKind = "run_updated"): GatewayEnvelope {
  return {
    schema_version: { major: 1, minor: 0, patch: 0 },
    cursor: { sequence: 1, partition: "p1" },
    entity_ref: { kind: "run", id: "run-1" },
    event_kind: eventKind,
    emitted_at: "2025-01-01T00:00:00Z",
  };
}

function makeRunEvent(sequence: number): RunEvent {
  return {
    sequence,
    event_id: `evt-${sequence}`,
    happened_at: "2025-01-01T00:00:00Z",
    kind: "ConversationStateUpdateEvent",
    summary: `Event ${sequence}`,
  };
}

function makeActionReceipt(correlationId: string): ActionReceipt {
  return {
    schema_version: { major: 1, minor: 0, patch: 0 },
    action_id: "action-1",
    correlation_id: correlationId,
    status: "accepted",
    expected_followup: [],
    issued_at: "2025-01-01T00:00:00Z",
  };
}

// -- Tests --

describe("gatewayReducer", () => {
  it("SNAPSHOT_RECEIVED sets snapshot and clears loading/error", () => {
    const state = gatewayReducer(initialState, {
      type: "SNAPSHOT_RECEIVED",
      nowMs: NOW,
      payload: makeSnapshot(),
    });
    expect(state.dashboard.snapshot).toBeTruthy();
    expect(state.dashboard.loading).toBe(false);
    expect(state.dashboard.error).toBeNull();
    expect(state.dashboard.lastUpdated).toBeTruthy();
  });

  it("TASK_GRAPH_RECEIVED sets nodes and clears loading/error", () => {
    const state = gatewayReducer(initialState, {
      type: "TASK_GRAPH_RECEIVED",
      nowMs: NOW,
      payload: makeTaskGraphSnapshot(),
    });
    expect(state.taskGraph.nodes.size).toBe(0);
    expect(state.taskGraph.loading).toBe(false);
    expect(state.taskGraph.error).toBeNull();
    expect(state.taskGraph.lastUpdated).toBeTruthy();
  });

  it("TASK_GRAPH_NODE_UPDATED updates an existing node and refreshes lastUpdated", () => {
    const graph = makeTaskGraphSnapshot();
    graph.nodes = [makeNode("a"), makeNode("b")];
    graph.root_ids = ["a", "b"];
    let state = gatewayReducer(initialState, {
      type: "TASK_GRAPH_RECEIVED",
      nowMs: NOW,
      payload: graph,
    });
    const updated = { ...state.taskGraph.nodes.get("a")!, title: "Updated A", state: "Done", state_category: "done" as const };
    state = gatewayReducer(state, {
      type: "TASK_GRAPH_NODE_UPDATED",
      nowMs: NOW,
      payload: updated,
    });
    expect(state.taskGraph.nodes.get("a")?.title).toBe("Updated A");
    expect(state.taskGraph.nodes.get("a")?.state).toBe("Done");
    expect(state.taskGraph.nodes.get("b")?.title).toBe("Issue b");
    expect(state.taskGraph.lastUpdated).toBeTruthy();
  });

  it("TASK_GRAPH_NODE_UPDATED is a no-op for an unknown node", () => {
    const graph = makeTaskGraphSnapshot();
    graph.nodes = [makeNode("a")];
    graph.root_ids = ["a"];
    let state = gatewayReducer(initialState, {
      type: "TASK_GRAPH_RECEIVED",
      nowMs: NOW,
      payload: graph,
    });
    const updated = makeNode("z");
    updated.title = "Unknown";
    state = gatewayReducer(state, {
      type: "TASK_GRAPH_NODE_UPDATED",
      nowMs: NOW,
      payload: updated,
    });
    expect(state.taskGraph.nodes.has("z")).toBe(false);
    expect(state.taskGraph.nodes.get("a")?.title).toBe("Issue a");
  });

  it("TASK_GRAPH_NODE_CREATED adds a child node and links it to the parent", () => {
    const graph = makeTaskGraphSnapshot();
    graph.nodes = [makeNode("a")];
    graph.root_ids = ["a"];
    let state = gatewayReducer(initialState, {
      type: "TASK_GRAPH_RECEIVED",
      nowMs: NOW,
      payload: graph,
    });
    const child = makeNode("a-1", "a");
    state = gatewayReducer(state, {
      type: "TASK_GRAPH_NODE_CREATED",
      nowMs: NOW,
      payload: child,
    });
    expect(state.taskGraph.nodes.get("a-1")).toBe(child);
    expect(state.taskGraph.nodes.get("a")?.children).toContain("a-1");
    expect(state.taskGraph.rootIds).not.toContain("a-1");
  });

  it("TASK_GRAPH_NODE_CREATED adds a root node when no parent is provided", () => {
    const graph = makeTaskGraphSnapshot();
    graph.nodes = [makeNode("a")];
    graph.root_ids = ["a"];
    let state = gatewayReducer(initialState, {
      type: "TASK_GRAPH_RECEIVED",
      nowMs: NOW,
      payload: graph,
    });
    const root = makeNode("c");
    state = gatewayReducer(state, {
      type: "TASK_GRAPH_NODE_CREATED",
      nowMs: NOW,
      payload: root,
    });
    expect(state.taskGraph.nodes.get("c")).toBe(root);
    expect(state.taskGraph.rootIds).toContain("c");
  });

  it("RUN_UPDATED stores run and clears loading/error", () => {
    const run = makeRunDetail();
    const state = gatewayReducer(initialState, {
      type: "RUN_UPDATED",
      nowMs: NOW,
      payload: run,
    });
    expect(state.run.runs.get("run-1")).toBe(run);
    expect(state.run.loading).toBe(false);
    expect(state.run.error).toBeNull();
    expect(state.run.lastUpdated).toBeTruthy();
  });

  it("TERMINAL_FRAMES_RECEIVED stores frames and clears loading/error", () => {
    const frame = makeFrame(1);
    const state = gatewayReducer(initialState, {
      type: "TERMINAL_FRAMES_RECEIVED",
      nowMs: NOW,
      runId: "run-1",
      frames: [frame],
    });
    expect(state.terminal.frames.get("run-1")).toHaveLength(1);
    expect(state.terminal.cursor.get("run-1")).toBe(1);
    expect(state.terminal.loading).toBe(false);
    expect(state.terminal.error).toBeNull();
    expect(state.terminal.lastUpdated).toBeTruthy();
  });

  it("TERMINAL_FRAMES_RECEIVED deduplicates frames by sequence", () => {
    const f1 = makeFrame(1);
    const f2 = makeFrame(2);
    let state = gatewayReducer(initialState, {
      type: "TERMINAL_FRAMES_RECEIVED",
      nowMs: NOW,
      runId: "run-1",
      frames: [f1, f2],
    });
    // Replay frame 1 plus a new frame 3.
    const f3 = makeFrame(3);
    state = gatewayReducer(state, {
      type: "TERMINAL_FRAMES_RECEIVED",
      nowMs: NOW,
      runId: "run-1",
      frames: [f1, f3],
    });
    expect(state.terminal.frames.get("run-1")).toHaveLength(3);
    expect(state.terminal.cursor.get("run-1")).toBe(3);
  });

  it("TERMINAL_FRAMES_RECEIVED cursor uses Math.max for out-of-order batches", () => {
    // Batch 1: frames 1-5, cursor = 5.
    let state = gatewayReducer(initialState, {
      type: "TERMINAL_FRAMES_RECEIVED",
      nowMs: NOW,
      runId: "run-1",
      frames: [makeFrame(1), makeFrame(2), makeFrame(3), makeFrame(4), makeFrame(5)],
    });
    expect(state.terminal.cursor.get("run-1")).toBe(5);
    // Batch 2: frames 3-4 arrive late (lower seq), cursor should stay at 5.
    state = gatewayReducer(state, {
      type: "TERMINAL_FRAMES_RECEIVED",
      nowMs: NOW,
      runId: "run-1",
      frames: [makeFrame(3), makeFrame(4)],
    });
    expect(state.terminal.cursor.get("run-1")).toBe(5);
  });

  it("TERMINAL_FRAMES_RECEIVED cursor uses max over unsorted batch", () => {
    // Batch arrives with frames 2, 1 (unsorted within batch).
    const state = gatewayReducer(initialState, {
      type: "TERMINAL_FRAMES_RECEIVED",
      nowMs: NOW,
      runId: "run-1",
      frames: [makeFrame(2), makeFrame(1)],
    });
    // Cursor should be 2 (max of batch), not 1 (last element).
    expect(state.terminal.cursor.get("run-1")).toBe(2);
  });

  it("TERMINAL_FRAMES_RECEIVED does not reset cursor for empty batch", () => {
    let state = gatewayReducer(initialState, {
      type: "TERMINAL_FRAMES_RECEIVED",
      nowMs: NOW,
      runId: "run-1",
      frames: [makeFrame(5)],
    });
    state = gatewayReducer(state, {
      type: "TERMINAL_FRAMES_RECEIVED",
      nowMs: NOW,
      runId: "run-1",
      frames: [],
    });
    expect(state.terminal.cursor.get("run-1")).toBe(5);
  });

  it("APPROVAL_RECEIVED adds approval and clears loading/error", () => {
    const approval = makeApproval("appr-1");
    const state = gatewayReducer(initialState, {
      type: "APPROVAL_RECEIVED",
      nowMs: NOW,
      payload: approval,
    });
    expect(state.approval.pending).toHaveLength(1);
    expect(state.approval.loading).toBe(false);
    expect(state.approval.error).toBeNull();
    expect(state.approval.lastUpdated).toBeTruthy();
  });

  it("APPROVAL_RECEIVED deduplicates by approval_id", () => {
    const approval = makeApproval("appr-1");
    let state = gatewayReducer(initialState, {
      type: "APPROVAL_RECEIVED",
      nowMs: NOW,
      payload: approval,
    });
    state = gatewayReducer(state, {
      type: "APPROVAL_RECEIVED",
      nowMs: NOW,
      payload: approval,
    });
    expect(state.approval.pending).toHaveLength(1);
  });

  it("APPROVAL_RESOLVED moves approval and clears loading/error", () => {
    const approval = makeApproval("appr-1");
    let state = gatewayReducer(initialState, {
      type: "APPROVAL_RECEIVED",
      nowMs: NOW,
      payload: approval,
    });
    state = gatewayReducer(state, {
      type: "APPROVAL_RESOLVED",
      nowMs: NOW,
      approvalId: "appr-1",
      payload: approval,
    });
    expect(state.approval.pending).toHaveLength(0);
    expect(state.approval.resolved.get("appr-1")).toBe(approval);
    expect(state.approval.loading).toBe(false);
    expect(state.approval.error).toBeNull();
    expect(state.approval.lastUpdated).toBeTruthy();
  });

  it("PLANNING_SESSION_UPDATED stores session and clears loading/error", () => {
    const session = makePlanningSummary();
    const state = gatewayReducer(initialState, {
      type: "PLANNING_SESSION_UPDATED",
      nowMs: NOW,
      payload: session,
    });
    expect(state.planning.sessions.get("sess-1")).toBe(session);
    expect(state.planning.loading).toBe(false);
    expect(state.planning.error).toBeNull();
    expect(state.planning.lastUpdated).toBeTruthy();
  });

  it("ENVELOPE_RECEIVED updates entity cache", () => {
    const state = gatewayReducer(initialState, {
      type: "ENVELOPE_RECEIVED",
      payload: makeEnvelope(),
    });
    expect(state.cache.runs.has("run-1")).toBe(true);
  });

  it("ERROR sets error and resets loading on all slices", () => {
    let state = gatewayReducer(initialState, {
      type: "LOADING",
      loading: true,
    });
    expect(state.dashboard.loading).toBe(true);
    expect(state.terminal.loading).toBe(true);
    state = gatewayReducer(state, {
      type: "ERROR",
      error: "Something failed",
    });
    expect(state.dashboard.error).toBe("Something failed");
    expect(state.dashboard.loading).toBe(false);
    expect(state.terminal.error).toBe("Something failed");
    expect(state.terminal.loading).toBe(false);
    expect(state.approval.error).toBe("Something failed");
    expect(state.approval.loading).toBe(false);
    expect(state.planning.error).toBe("Something failed");
    expect(state.planning.loading).toBe(false);
  });

  it("LOADING toggles loading on all slices", () => {
    const state = gatewayReducer(initialState, {
      type: "LOADING",
      loading: true,
    });
    expect(state.dashboard.loading).toBe(true);
    expect(state.taskGraph.loading).toBe(true);
    expect(state.run.loading).toBe(true);
    expect(state.terminal.loading).toBe(true);
    expect(state.approval.loading).toBe(true);
    expect(state.planning.loading).toBe(true);
  });

  it("LOADING true clears prior errors, LOADING false preserves them", () => {
    let state = gatewayReducer(initialState, {
      type: "ERROR",
      error: "Previous failure",
    });
    expect(state.dashboard.error).toBe("Previous failure");
    state = gatewayReducer(state, {
      type: "LOADING",
      loading: true,
    });
    expect(state.dashboard.error).toBeNull();
    expect(state.dashboard.loading).toBe(true);
    state = gatewayReducer(state, {
      type: "LOADING",
      loading: false,
    });
    expect(state.dashboard.error).toBeNull();
    expect(state.dashboard.loading).toBe(false);
  });
});

describe("connection state", () => {
  it("CONNECTION_STATE_CHANGED updates connection slice", () => {
    const state = gatewayReducer(initialState, {
      type: "CONNECTION_STATE_CHANGED",
      nowMs: NOW,
      state: "connecting",
    });
    expect(state.connection.state).toBe("connecting");
  });

  it("CONNECTION_STATE_CHANGED records connected timestamp", () => {
    const state = gatewayReducer(initialState, {
      type: "CONNECTION_STATE_CHANGED",
      nowMs: NOW,
      state: "connected",
    });
    expect(state.connection.lastConnectedAt).toBeTruthy();
  });

  it("CONNECTION_STATE_CHANGED records disconnected timestamp", () => {
    let state = gatewayReducer(initialState, {
      type: "CONNECTION_STATE_CHANGED",
      nowMs: NOW,
      state: "connected",
    });
    state = gatewayReducer(state, {
      type: "CONNECTION_STATE_CHANGED",
      nowMs: NOW,
      state: "disconnected",
    });
    expect(state.connection.lastDisconnectedAt).toBeTruthy();
  });

  it("RECONNECT_ATTEMPTED increments counter", () => {
    const state = gatewayReducer(initialState, {
      type: "RECONNECT_ATTEMPTED",
      attempts: 2,
      nowMs: NOW,
    });
    expect(state.connection.reconnectAttempts).toBe(2);
    expect(state.connection.state).toBe("reconnecting");
  });

  it("ERROR with reconnect keyword sets reconnecting state", () => {
    const state = gatewayReducer(initialState, {
      type: "ERROR",
      error: "reconnect failed",
    });
    expect(state.connection.state).toBe("reconnecting");
  });

  it("ERROR without reconnect keyword sets failed state", () => {
    const state = gatewayReducer(initialState, {
      type: "ERROR",
      error: "something broke",
    });
    expect(state.connection.state).toBe("failed");
  });
});

describe("run events and liveness", () => {
  it("RUN_EVENTS_RECEIVED updates liveness state", () => {
    let state = gatewayReducer(initialState, {
      type: "RUN_UPDATED",
      nowMs: NOW,
      payload: makeRunDetail(),
    });
    state = gatewayReducer(state, {
      type: "RUN_EVENTS_RECEIVED",
      nowMs: NOW,
      runId: "run-1",
      events: [makeRunEvent(1), makeRunEvent(2)],
    });
    const liveness = state.run.liveness.get("run-1");
    expect(liveness).toBeTruthy();
    // eventCount is incremented by RUN_EVENTS_RECEIVED via computeLivenessState.
    expect(liveness?.eventCount).toBe(2);
    expect(liveness?.lastEventAt).toBeTruthy();
    expect(liveness?.gapSeconds).toBe(0);
  });

  it("STREAM_HEALTH_CHECK computes liveness from elapsed time", () => {
    const baseTime = 1_700_000_000_000; // Fixed timestamp for deterministic tests.
    let state = gatewayReducer(initialState, {
      type: "RUN_UPDATED",
      payload: makeRunDetail(),
      nowMs: baseTime,
    });
    state = gatewayReducer(state, {
      type: "RUN_EVENTS_RECEIVED",
      runId: "run-1",
      events: [makeRunEvent(1)],
      nowMs: baseTime,
    });

    // Check health 10 seconds later -> still quiet.
    state = gatewayReducer(state, {
      type: "STREAM_HEALTH_CHECK",
      runId: "run-1",
      nowMs: baseTime + 10_000,
    });
    let updatedLiveness = state.run.liveness.get("run-1");
    expect(updatedLiveness?.phaseState).toBe("quiet");

    // Check health 45 seconds later -> degraded.
    state = gatewayReducer(state, {
      type: "STREAM_HEALTH_CHECK",
      runId: "run-1",
      nowMs: baseTime + 45_000,
    });
    updatedLiveness = state.run.liveness.get("run-1");
    expect(updatedLiveness?.phaseState).toBe("degraded");

    // Check health 90 seconds later -> stalled.
    state = gatewayReducer(state, {
      type: "STREAM_HEALTH_CHECK",
      runId: "run-1",
      nowMs: baseTime + 90_000,
    });
    updatedLiveness = state.run.liveness.get("run-1");
    expect(updatedLiveness?.phaseState).toBe("stalled");

    // Check health 150 seconds later -> detached.
    state = gatewayReducer(state, {
      type: "STREAM_HEALTH_CHECK",
      runId: "run-1",
      nowMs: baseTime + 150_000,
    });
    updatedLiveness = state.run.liveness.get("run-1");
    expect(updatedLiveness?.phaseState).toBe("detached");
  });

  it("STREAM_STALE_DETECTED sets degraded state, not failed", () => {
    let state = gatewayReducer(initialState, {
      type: "RUN_UPDATED",
      nowMs: NOW,
      payload: makeRunDetail(),
    });
    state = gatewayReducer(state, {
      type: "RUN_EVENTS_RECEIVED",
      nowMs: NOW,
      runId: "run-1",
      events: [makeRunEvent(1)],
    });

    state = gatewayReducer(state, {
      type: "STREAM_STALE_DETECTED",
      nowMs: NOW,
      runId: "run-1",
    });

    const liveness = state.run.liveness.get("run-1");
    expect(liveness?.phaseState).toBe("degraded");
    expect(liveness?.isStreamStale).toBe(true);
    expect(liveness?.streamHealth).toBe("stale");
    // IMPORTANT: stale stream should NOT collapse into failed run state.
    expect(state.run.error).toBeNull();
  });

  it("STREAM_RECOVERED restores active state", () => {
    let state = gatewayReducer(initialState, {
      type: "RUN_UPDATED",
      nowMs: NOW,
      payload: makeRunDetail(),
    });
    state = gatewayReducer(state, {
      type: "RUN_EVENTS_RECEIVED",
      nowMs: NOW,
      runId: "run-1",
      events: [makeRunEvent(1)],
    });
    state = gatewayReducer(state, {
      type: "STREAM_STALE_DETECTED",
      nowMs: NOW,
      runId: "run-1",
    });

    state = gatewayReducer(state, {
      type: "STREAM_RECOVERED",
      nowMs: NOW,
      runId: "run-1",
    });

    const liveness = state.run.liveness.get("run-1");
    expect(liveness?.phaseState).toBe("active");
    expect(liveness?.isStreamStale).toBe(false);
    expect(liveness?.streamHealth).toBe("healthy");
  });
});

describe("action receipts", () => {
  it("ACTION_DISPATCHED adds to pending", () => {
    const state = gatewayReducer(initialState, {
      type: "ACTION_DISPATCHED",
      nowMs: NOW,
      correlationId: "corr-1",
    });
    expect(state.actionReceipts.pending.has("corr-1")).toBe(true);
  });

  it("ACTION_RECEIPT_RECEIVED moves from pending to receipts", () => {
    let state = gatewayReducer(initialState, {
      type: "ACTION_DISPATCHED",
      nowMs: NOW,
      correlationId: "corr-1",
    });
    state = gatewayReducer(state, {
      type: "ACTION_RECEIPT_RECEIVED",
      nowMs: NOW,
      receipt: makeActionReceipt("corr-1"),
    });
    expect(state.actionReceipts.pending.has("corr-1")).toBe(false);
    expect(state.actionReceipts.receipts.has("corr-1")).toBe(true);
  });
});

describe("deriveRunPhaseState", () => {
  it("retry_queued run status maps to retry_queued phase", () => {
    expect(deriveRunPhaseState({ status: "retry_queued" } as any, undefined, false)).toBe("retry_queued");
  });

  it("released run status maps to cancelled phase", () => {
    expect(deriveRunPhaseState({ status: "released" } as any, undefined, false)).toBe("cancelled");
  });

  it("stale stream maps to degraded phase when run is still alive", () => {
    expect(deriveRunPhaseState({ status: "running" } as any, undefined, true)).toBe("degraded");
  });

  it("stale stream ignored for unclaimed run", () => {
    expect(deriveRunPhaseState({ status: "unclaimed" } as any, undefined, true)).toBe("active");
  });

  it("stale stream maps to degraded for claimed run", () => {
    expect(deriveRunPhaseState({ status: "claimed" } as any, undefined, true)).toBe("degraded");
  });

  it("active run with fresh stream is active", () => {
    const liveness = {
      runId: "run-1",
      phaseState: "active" as const,
      lastEventAt: null,
      lastStatusUpdateAt: null,
      eventCount: 10,
      gapSeconds: 1,
      isStreamStale: false,
      streamHealth: "healthy" as const,
    };
    expect(deriveRunPhaseState({ status: "running" } as any, liveness, false)).toBe("active");
  });
});

describe("computeLivenessState", () => {
  const baseTime = 1_700_000_000_000;

  it("quiet when gap is within threshold", () => {
    const liveness = computeLivenessState(
      "run-1",
      {
        runId: "run-1",
        phaseState: "quiet",
        lastEventAt: new Date(baseTime).toISOString(),
        lastStatusUpdateAt: null,
        eventCount: 5,
        gapSeconds: 0,
        isStreamStale: false,
        streamHealth: "healthy",
      },
      baseTime + 2000,
      0, // No new events since last health check.
    );
    // With the gap (2s) far below the quiet threshold (300s), stays quiet.
    expect(liveness.phaseState).toBe("quiet");
    expect(liveness.eventCount).toBe(5);
  });

  it("no recent events -> quiet", () => {
    const liveness = computeLivenessState(
      "run-1",
      undefined,
      baseTime,
      0, // No new events.
    );
    expect(liveness.phaseState).toBe("quiet");
  });

  it("fresh events -> active", () => {
    const liveness = computeLivenessState(
      "run-1",
      {
        runId: "run-1",
        phaseState: "quiet",
        lastEventAt: new Date(baseTime).toISOString(),
        lastStatusUpdateAt: null,
        eventCount: 5,
        gapSeconds: 0,
        isStreamStale: false,
        streamHealth: "healthy",
      },
      baseTime + 2000,
      3, // 3 new events since last check.
    );
    // With fresh events and small gap, should be active.
    expect(liveness.phaseState).toBe("active");
    expect(liveness.eventCount).toBe(8);
  });
});

describe("entity cache", () => {
  it("RUN_UPDATED populates entity cache", () => {
    const state = gatewayReducer(initialState, {
      type: "RUN_UPDATED",
      nowMs: NOW,
      payload: makeRunDetail(),
    });
    expect(state.cache.runs.has("run-1")).toBe(true);
    expect(state.cache.runs.get("run-1")?.version).toBe(1);
  });

  it("APPROVAL_RECEIVED populates entity cache", () => {
    const state = gatewayReducer(initialState, {
      type: "APPROVAL_RECEIVED",
      nowMs: NOW,
      payload: makeApproval("appr-1"),
    });
    expect(state.cache.approvals.has("appr-1")).toBe(true);
  });

  it("TERMINAL_FRAMES_RECEIVED populates entity cache", () => {
    const state = gatewayReducer(initialState, {
      type: "TERMINAL_FRAMES_RECEIVED",
      nowMs: NOW,
      runId: "run-1",
      frames: [makeFrame(1)],
    });
    expect(state.cache.terminals.has("run-1")).toBe(true);
  });

  it("PLANNING_SESSION_UPDATED populates entity cache", () => {
    const state = gatewayReducer(initialState, {
      type: "PLANNING_SESSION_UPDATED",
      nowMs: NOW,
      payload: makePlanningSummary(),
    });
    expect(state.cache.planning.has("sess-1")).toBe(true);
  });
});

describe("stream staleness vs failed run", () => {
  it("stale stream does not set run error", () => {
    let state = gatewayReducer(initialState, {
      type: "RUN_UPDATED",
      nowMs: NOW,
      payload: makeRunDetail(),
    });
    state = gatewayReducer(state, {
      type: "STREAM_STALE_DETECTED",
      nowMs: NOW,
      runId: "run-1",
    });
    // Stream is stale but run should not be errored out.
    expect(state.run.error).toBeNull();
    expect(state.terminal.streamStale.get("run-1")).toBe(true);
  });

  it("stream recovery clears staleness flag", () => {
    let state = gatewayReducer(initialState, {
      type: "RUN_UPDATED",
      nowMs: NOW,
      payload: makeRunDetail(),
    });
    state = gatewayReducer(state, {
      type: "STREAM_STALE_DETECTED",
      nowMs: NOW,
      runId: "run-1",
    });
    state = gatewayReducer(state, {
      type: "STREAM_RECOVERED",
      nowMs: NOW,
      runId: "run-1",
    });
    expect(state.terminal.streamStale.get("run-1")).toBe(false);
  });

  it("released/completed run does not collapse to degraded on stream staleness", () => {
    let state = gatewayReducer(initialState, {
      type: "RUN_UPDATED",
      nowMs: NOW,
      payload: {
        ...makeRunDetail("released"),
        release_reason: "completed",
      },
    });
    state = gatewayReducer(state, {
      type: "RUN_EVENTS_RECEIVED",
      nowMs: NOW,
      runId: "run-1",
      events: [makeRunEvent(1)],
    });
    state = gatewayReducer(state, {
      type: "STREAM_STALE_DETECTED",
      nowMs: NOW,
      runId: "run-1",
    });

    const liveness = state.run.liveness.get("run-1");
    // Completed run stays "completed" even when stream is stale.
    expect(liveness?.phaseState).toBe("completed");
    expect(liveness?.isStreamStale).toBe(true);
  });

  it("released/cancelled run stays cancelled on stream staleness", () => {
    let state = gatewayReducer(initialState, {
      type: "RUN_UPDATED",
      nowMs: NOW,
      payload: {
        ...makeRunDetail("released"),
        release_reason: "cancelled",
      },
    });
    state = gatewayReducer(state, {
      type: "RUN_EVENTS_RECEIVED",
      nowMs: NOW,
      runId: "run-1",
      events: [makeRunEvent(1)],
    });
    state = gatewayReducer(state, {
      type: "STREAM_STALE_DETECTED",
      nowMs: NOW,
      runId: "run-1",
    });

    const liveness = state.run.liveness.get("run-1");
    expect(liveness?.phaseState).toBe("cancelled");
    expect(liveness?.isStreamStale).toBe(true);
  });

  it("RUN_EVENTS_RECEIVED does not force stalled run to active", () => {
    // Create a running run and drive it to stalled via health checks.
    const baseTime = 1_700_000_000_000;
    let state = gatewayReducer(initialState, {
      type: "RUN_UPDATED",
      nowMs: baseTime,
      payload: makeRunDetail("running"),
    });
    state = gatewayReducer(state, {
      type: "RUN_EVENTS_RECEIVED",
      nowMs: baseTime,
      runId: "run-1",
      events: [makeRunEvent(1)],
    });
    // Drive liveness to stalled by checking health at 90s (60-120s window).
    state = gatewayReducer(state, {
      type: "STREAM_HEALTH_CHECK",
      runId: "run-1",
      nowMs: baseTime + 90_000,
    });
    expect(state.run.liveness.get("run-1")?.phaseState).toBe("stalled");

    // Now dispatch events — the gap is still large so phase should
    // NOT be forced to "active" by the reducer.
    state = gatewayReducer(state, {
      type: "RUN_EVENTS_RECEIVED",
      nowMs: baseTime + 95_000,
      runId: "run-1",
      events: [makeRunEvent(2)],
    });

    const liveness = state.run.liveness.get("run-1");
    // deriveRunPhaseState correctly preserves "stalled" for a running run with
    // a large event gap, rather than the old hardcoded "active" override.
    expect(liveness?.phaseState).toBe("stalled");
    expect(liveness?.isStreamStale).toBe(false);
  });

  it("STREAM_RECOVERED preserves completed run phase state", () => {
    let state = gatewayReducer(initialState, {
      type: "RUN_UPDATED",
      nowMs: NOW,
      payload: {
        ...makeRunDetail("released"),
        release_reason: "completed",
      },
    });
    state = gatewayReducer(state, {
      type: "RUN_EVENTS_RECEIVED",
      nowMs: NOW,
      runId: "run-1",
      events: [makeRunEvent(1)],
    });
    // Simulate stream going stale then recovering.
    state = gatewayReducer(state, {
      type: "STREAM_STALE_DETECTED",
      nowMs: NOW,
      runId: "run-1",
    });
    expect(state.run.liveness.get("run-1")?.phaseState).toBe("completed");

    state = gatewayReducer(state, {
      type: "STREAM_RECOVERED",
      nowMs: NOW + 1000,
      runId: "run-1",
    });

    const liveness = state.run.liveness.get("run-1");
    expect(liveness?.phaseState).toBe("completed");
    expect(liveness?.isStreamStale).toBe(false);
    expect(liveness?.streamHealth).toBe("healthy");
  });
});

// -- computeSafeActions tests --

describe("computeSafeActions", () => {
  it("allows cancel only for active, healthy runs", () => {
    const actions = computeSafeActions("active", "healthy");
    expect(actions).toEqual<SafeActions>({ retry: false, cancel: true, rehydrate: false, detach: false });
  });

  it("allows cancel and rehydrate for active, stale runs", () => {
    const actions = computeSafeActions("active", "stale");
    expect(actions).toEqual<SafeActions>({ retry: false, cancel: true, rehydrate: true, detach: false });
  });

  it("allows detach only for active, dead runs", () => {
    const actions = computeSafeActions("active", "dead");
    expect(actions).toEqual<SafeActions>({ retry: false, cancel: false, rehydrate: false, detach: true });
  });

  it("allows retry, cancel, rehydrate for stalled, stale runs", () => {
    const actions = computeSafeActions("stalled", "stale");
    expect(actions).toEqual<SafeActions>({ retry: true, cancel: true, rehydrate: true, detach: false });
  });

  it("allows retry only for retry_queued runs", () => {
    const actions = computeSafeActions("retry_queued", "healthy");
    expect(actions).toEqual<SafeActions>({ retry: true, cancel: false, rehydrate: false, detach: false });
  });

  it("allows retry only for cancelled runs", () => {
    const actions = computeSafeActions("cancelled", "dead");
    expect(actions).toEqual<SafeActions>({ retry: true, cancel: false, rehydrate: false, detach: false });
  });

  it("allows cancel and rehydrate for degraded, stale runs", () => {
    const actions = computeSafeActions("degraded", "stale");
    expect(actions).toEqual<SafeActions>({ retry: false, cancel: true, rehydrate: true, detach: false });
  });

  it("allows cancel only for quiet, healthy runs", () => {
    const actions = computeSafeActions("quiet", "healthy");
    expect(actions).toEqual<SafeActions>({ retry: false, cancel: true, rehydrate: false, detach: false });
  });

  it("allows rehydrate only for detached, stale runs", () => {
    const actions = computeSafeActions("detached", "stale");
    expect(actions).toEqual<SafeActions>({ retry: false, cancel: false, rehydrate: true, detach: false });
  });

  it("allows retry and rehydrate for detached, dead runs", () => {
    const actions = computeSafeActions("detached", "dead");
    expect(actions).toEqual<SafeActions>({ retry: true, cancel: false, rehydrate: true, detach: false });
  });

  it("allows detach only for quiet, dead runs", () => {
    const actions = computeSafeActions("quiet", "dead");
    expect(actions).toEqual<SafeActions>({ retry: false, cancel: false, rehydrate: false, detach: true });
  });

  it("allows retry for stalled, healthy runs", () => {
    const actions = computeSafeActions("stalled", "healthy");
    expect(actions).toEqual<SafeActions>({ retry: true, cancel: true, rehydrate: false, detach: false });
  });

  it("returns safe defaults for unknown phase states", () => {
    const actions = computeSafeActions("unknown" as RunPhase, "healthy");
    expect(actions).toEqual<SafeActions>({ retry: false, cancel: false, rehydrate: false, detach: false });
  });

  it("returns safe defaults for unknown stream health", () => {
    const actions = computeSafeActions("active", "unknown" as RunStreamLiveness);
    expect(actions).toEqual<SafeActions>({ retry: false, cancel: false, rehydrate: false, detach: false });
  });
});
