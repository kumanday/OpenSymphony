/** Reducer unit tests for @opensymphony/state. */

import {
  gatewayReducer,
  initialState,
} from "@opensymphony/state";
import type {
  DashboardSnapshot,
  TaskGraphSnapshot,
  RunDetail,
  TerminalFrame,
  ApprovalRequest,
  PlanningSessionSummary,
  GatewayEnvelope,
} from "@opensymphony/gateway-schema";

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

function makeRunDetail(): RunDetail {
  return {
    schema_version: { major: 1, minor: 0, patch: 0 },
    run_id: "run-1",
    issue_id: "issue-1",
    issue_identifier: "COE-001",
    worker_id: "worker-1",
    status: "running",
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

function makeEnvelope(): GatewayEnvelope {
  return {
    schema_version: { major: 1, minor: 0, patch: 0 },
    cursor: { sequence: 1, partition: "p1" },
    entity_ref: { kind: "run", id: "run-1" },
    event_kind: "run_updated",
    emitted_at: "2025-01-01T00:00:00Z",
  };
}

// -- Tests --

describe("gatewayReducer", () => {
  it("SNAPSHOT_RECEIVED sets snapshot and clears loading/error", () => {
    const state = gatewayReducer(initialState, {
      type: "SNAPSHOT_RECEIVED",
      payload: makeSnapshot(),
    });
    expect(state.dashboard.snapshot).toBeTruthy();
    expect(state.dashboard.loading).toBe(false);
    expect(state.dashboard.error).toBeNull();
  });

  it("TASK_GRAPH_RECEIVED sets nodes and clears loading/error", () => {
    const state = gatewayReducer(initialState, {
      type: "TASK_GRAPH_RECEIVED",
      payload: makeTaskGraphSnapshot(),
    });
    expect(state.taskGraph.nodes.size).toBe(0);
    expect(state.taskGraph.loading).toBe(false);
    expect(state.taskGraph.error).toBeNull();
  });

  it("RUN_UPDATED stores run and clears loading/error", () => {
    const run = makeRunDetail();
    const state = gatewayReducer(initialState, {
      type: "RUN_UPDATED",
      payload: run,
    });
    expect(state.run.runs.get("run-1")).toBe(run);
    expect(state.run.loading).toBe(false);
    expect(state.run.error).toBeNull();
  });

  it("TERMINAL_FRAMES_RECEIVED stores frames and clears loading/error", () => {
    const frame = makeFrame(1);
    const state = gatewayReducer(initialState, {
      type: "TERMINAL_FRAMES_RECEIVED",
      runId: "run-1",
      frames: [frame],
    });
    expect(state.terminal.frames.get("run-1")).toHaveLength(1);
    expect(state.terminal.cursor.get("run-1")).toBe(1);
    expect(state.terminal.loading).toBe(false);
    expect(state.terminal.error).toBeNull();
  });

  it("TERMINAL_FRAMES_RECEIVED deduplicates frames by sequence", () => {
    const f1 = makeFrame(1);
    const f2 = makeFrame(2);
    let state = gatewayReducer(initialState, {
      type: "TERMINAL_FRAMES_RECEIVED",
      runId: "run-1",
      frames: [f1, f2],
    });
    // Replay frame 1 plus a new frame 3.
    const f3 = makeFrame(3);
    state = gatewayReducer(state, {
      type: "TERMINAL_FRAMES_RECEIVED",
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
      runId: "run-1",
      frames: [makeFrame(1), makeFrame(2), makeFrame(3), makeFrame(4), makeFrame(5)],
    });
    expect(state.terminal.cursor.get("run-1")).toBe(5);
    // Batch 2: frames 3-4 arrive late (lower seq), cursor should stay at 5.
    state = gatewayReducer(state, {
      type: "TERMINAL_FRAMES_RECEIVED",
      runId: "run-1",
      frames: [makeFrame(3), makeFrame(4)],
    });
    expect(state.terminal.cursor.get("run-1")).toBe(5);
  });

  it("TERMINAL_FRAMES_RECEIVED cursor uses max over unsorted batch", () => {
    // Batch arrives with frames 2, 1 (unsorted within batch).
    const state = gatewayReducer(initialState, {
      type: "TERMINAL_FRAMES_RECEIVED",
      runId: "run-1",
      frames: [makeFrame(2), makeFrame(1)],
    });
    // Cursor should be 2 (max of batch), not 1 (last element).
    expect(state.terminal.cursor.get("run-1")).toBe(2);
  });

  it("TERMINAL_FRAMES_RECEIVED does not reset cursor for empty batch", () => {
    let state = gatewayReducer(initialState, {
      type: "TERMINAL_FRAMES_RECEIVED",
      runId: "run-1",
      frames: [makeFrame(5)],
    });
    state = gatewayReducer(state, {
      type: "TERMINAL_FRAMES_RECEIVED",
      runId: "run-1",
      frames: [],
    });
    expect(state.terminal.cursor.get("run-1")).toBe(5);
  });

  it("APPROVAL_RECEIVED adds approval and clears loading/error", () => {
    const approval = makeApproval("appr-1");
    const state = gatewayReducer(initialState, {
      type: "APPROVAL_RECEIVED",
      payload: approval,
    });
    expect(state.approval.pending).toHaveLength(1);
    expect(state.approval.loading).toBe(false);
    expect(state.approval.error).toBeNull();
  });

  it("APPROVAL_RECEIVED deduplicates by approval_id", () => {
    const approval = makeApproval("appr-1");
    let state = gatewayReducer(initialState, {
      type: "APPROVAL_RECEIVED",
      payload: approval,
    });
    state = gatewayReducer(state, {
      type: "APPROVAL_RECEIVED",
      payload: approval,
    });
    expect(state.approval.pending).toHaveLength(1);
  });

  it("APPROVAL_RESOLVED moves approval and clears loading/error", () => {
    const approval = makeApproval("appr-1");
    let state = gatewayReducer(initialState, {
      type: "APPROVAL_RECEIVED",
      payload: approval,
    });
    state = gatewayReducer(state, {
      type: "APPROVAL_RESOLVED",
      approvalId: "appr-1",
      payload: approval,
    });
    expect(state.approval.pending).toHaveLength(0);
    expect(state.approval.resolved.get("appr-1")).toBe(approval);
    expect(state.approval.loading).toBe(false);
    expect(state.approval.error).toBeNull();
  });

  it("PLANNING_SESSION_UPDATED stores session and clears loading/error", () => {
    const session = makePlanningSummary();
    const state = gatewayReducer(initialState, {
      type: "PLANNING_SESSION_UPDATED",
      payload: session,
    });
    expect(state.planning.sessions.get("sess-1")).toBe(session);
    expect(state.planning.loading).toBe(false);
    expect(state.planning.error).toBeNull();
  });

  it("ENVELOPE_RECEIVED is a no-op placeholder", () => {
    const state = gatewayReducer(initialState, {
      type: "ENVELOPE_RECEIVED",
      payload: makeEnvelope(),
    });
    expect(state).toBe(initialState);
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
