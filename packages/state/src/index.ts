/**
 * Reducer-driven state management for OpenSymphony clients.
 *
 * This package defines Redux-style reducer functions that keep the
 * client-side state model in sync with gateway snapshots and event
 * streams. The reducer is transport-agnostic and framework-neutral.
 */

import type {
  GatewayEnvelope,
  DashboardSnapshot,
  TaskGraphNode,
  TaskGraphSnapshot,
  RunDetail,
  RunEvent,
  RunStatus,
  TerminalFrame,
  ApprovalRequest,
  PlanningSessionSummary,
  ConnectionProfile,
  ConnectionProfileKind,
  ActionReceipt,
  RunPhase,
  RunStreamLiveness,
  RunLivenessEnvelope,
  RunDiagnostics,
  SafeActions,
  RunTimeline,
  TerminalSearchResult,
  TerminalJumpResult,
} from "@opensymphony/gateway-schema";

// Profile state management
export type {
  ProfileState,
  ProfileAction,
} from "./profiles.js";
export {
  initialProfileState,
  profileReducer,
  getEffectiveGatewayUrl,
  getActiveProfile,
  isManagedProfile,
  isLocalProfile,
} from "./profiles.js";

// Model profile state management
export type {
  ModelProfileAction,
  ModelProfileState,
  ModelProfileStorage,
  ModelProfileStore,
  ModelProfileStoreOptions,
  AsyncModelProfileStoreOptions,
} from "./model-profiles.js";
export {
  createAsyncModelProfileStore,
  createModelProfileStore,
  getActiveModelProfile,
  initialModelProfileState,
  modelProfileReducer,
  normalizeModelProfileState,
  sanitizeModelProfiles,
} from "./model-profiles.js";

// -- Connection state --

export type ConnectionState =
  | "disconnected"
  | "connecting"
  | "connected"
  | "degraded"
  | "reconnecting"
  | "failed";

export interface ConnectionStateSlice {
  state: ConnectionState;
  lastConnectedAt: string | null;
  lastDisconnectedAt: string | null;
  reconnectAttempts: number;
  error: string | null;
}

// -- Entity cache --

export interface EntityCacheEntry {
  lastSeen: string;
  version: number;
  data: unknown;
}

export interface EntityCacheSlice {
  runs: Map<string, EntityCacheEntry>;
  terminals: Map<string, EntityCacheEntry>;
  approvals: Map<string, EntityCacheEntry>;
  planning: Map<string, EntityCacheEntry>;
}

// -- Run phase liveness state --

/**
 * Client-side interpretation of run liveness from the stream perspective.
 * These states are derived from run status, event recency, and stream health.
 *
 * - active:   Run is producing events at a normal rate.
 * - quiet:    Run is still claimed/running but producing no events recently.
 * - degraded: Stream is lagging or losing events but run is still alive.
 * - stalled:  No events for an extended period; run may be stuck.
 * - retry_queued: Run status indicates it is queued for retry.
 * - cancelled: Run was explicitly cancelled.
 * - completed: Run completed successfully (terminal state).
 * - detached: Client lost connection; run may still be executing server-side.
 */
export type RunPhaseState =
  | "active"
  | "quiet"
  | "degraded"
  | "stalled"
  | "retry_queued"
  | "cancelled"
  | "completed"
  | "detached";

export interface RunLivenessState {
  runId: string;
  phaseState: RunPhaseState;
  lastEventAt: string | null;
  lastStatusUpdateAt: string | null;
  eventCount: number;
  gapSeconds: number;
  isStreamStale: boolean;
  streamHealth: "healthy" | "stale" | "dead";
}

// -- State slices --

export interface DashboardSlice {
  snapshot: DashboardSnapshot | null;
  loading: boolean;
  error: string | null;
  lastUpdated: string | null;
}

export interface TaskGraphSlice {
  nodes: Map<string, TaskGraphNode>;
  rootIds: string[];
  loading: boolean;
  error: string | null;
  lastUpdated: string | null;
}

export interface RunSlice {
  runs: Map<string, RunDetail>;
  liveness: Map<string, RunLivenessState>;
  loading: boolean;
  error: string | null;
  lastUpdated: string | null;
}

export interface TerminalSlice {
  frames: Map<string, TerminalFrame[]>;
  cursor: Map<string, number>;
  loading: boolean;
  error: string | null;
  lastUpdated: string | null;
  /** Tracks whether the terminal stream is stale vs the run being failed. */
  streamStale: Map<string, boolean>;
}

export interface ApprovalSlice {
  pending: ApprovalRequest[];
  resolved: Map<string, ApprovalRequest>;
  loading: boolean;
  error: string | null;
  lastUpdated: string | null;
}

export interface PlanningSlice {
  sessions: Map<string, PlanningSessionSummary>;
  loading: boolean;
  error: string | null;
  lastUpdated: string | null;
}

/** Timeline slice for run event grouping. */
export interface TimelineSlice {
  timelines: Map<string, RunTimeline>;
  loading: boolean;
  error: string | null;
  lastUpdated: string | null;
}

/** Action receipts correlated with dispatched mutations. */
export interface ActionReceiptSlice {
  receipts: Map<string, ActionReceipt>;
  pending: Map<string, { correlationId: string; dispatchedAt: string }>;
}

// -- Combined state --

export interface GatewayState {
  connection: ConnectionStateSlice;
  cache: EntityCacheSlice;
  dashboard: DashboardSlice;
  taskGraph: TaskGraphSlice;
  run: RunSlice;
  terminal: TerminalSlice;
  timeline: TimelineSlice;
  approval: ApprovalSlice;
  planning: PlanningSlice;
  actionReceipts: ActionReceiptSlice;
}

/** Liveness thresholds in milliseconds. */
export const LIVENESS_THRESHOLDS = {
  /** Events arriving faster than this interval indicate active work. */
  activeIntervalMs: 5000,
  /** No events for this duration -> quiet state. */
  quietThresholdMs: 30_000,
  /** No events for this duration -> degraded state. */
  degradedThresholdMs: 60_000,
  /** No events for this duration -> stalled state. */
  stalledThresholdMs: 120_000,
};

export const initialState: GatewayState = {
  connection: {
    state: "disconnected",
    lastConnectedAt: null,
    lastDisconnectedAt: null,
    reconnectAttempts: 0,
    error: null,
  },
  cache: {
    runs: new Map(),
    terminals: new Map(),
    approvals: new Map(),
    planning: new Map(),
  },
  dashboard: { snapshot: null, loading: false, error: null, lastUpdated: null },
  taskGraph: { nodes: new Map(), rootIds: [], loading: false, error: null, lastUpdated: null },
  run: { runs: new Map(), liveness: new Map(), loading: false, error: null, lastUpdated: null },
  terminal: {
    frames: new Map(),
    cursor: new Map(),
    loading: false,
    error: null,
    lastUpdated: null,
    streamStale: new Map(),
  },
  timeline: { timelines: new Map(), loading: false, error: null, lastUpdated: null },
  approval: { pending: [], resolved: new Map(), loading: false, error: null, lastUpdated: null },
  planning: { sessions: new Map(), loading: false, error: null, lastUpdated: null },
  actionReceipts: { receipts: new Map(), pending: new Map() },
};

// -- Action types --

export type GatewayAction =
  // Connection actions
  | { type: "CONNECTION_STATE_CHANGED"; state: ConnectionState; error?: string; nowMs: number }
  | { type: "RECONNECT_ATTEMPTED"; attempts: number; nowMs: number }
  // Snapshot/actions
  | { type: "SNAPSHOT_RECEIVED"; payload: DashboardSnapshot; nowMs: number }
  | { type: "TASK_GRAPH_RECEIVED"; payload: TaskGraphSnapshot; nowMs: number }
  | { type: "TASK_GRAPH_NODE_UPDATED"; payload: TaskGraphNode; nowMs: number }
  | { type: "TASK_GRAPH_NODE_CREATED"; payload: TaskGraphNode; nowMs: number }
  | { type: "RUN_UPDATED"; payload: RunDetail; nowMs: number }
  | { type: "TERMINAL_FRAMES_RECEIVED"; runId: string; frames: TerminalFrame[]; nowMs: number }
  | { type: "APPROVAL_RECEIVED"; payload: ApprovalRequest; nowMs: number }
  | { type: "APPROVAL_RESOLVED"; approvalId: string; payload: ApprovalRequest; nowMs: number }
  | { type: "PLANNING_SESSION_UPDATED"; payload: PlanningSessionSummary; nowMs: number }
  | { type: "RUN_EVENTS_RECEIVED"; runId: string; events: RunEvent[]; nowMs: number }
  | { type: "RUN_TIMELINE_RECEIVED"; runId: string; payload: RunTimeline; nowMs: number }
  | { type: "TERMINAL_SEARCH_RESULT"; runId: string; terminalSessionId: string; payload: TerminalSearchResult; nowMs: number }
  | { type: "TERMINAL_JUMP_RESULT"; runId: string; terminalSessionId: string; payload: TerminalJumpResult; nowMs: number }
  // Envelope/actions
  | { type: "ENVELOPE_RECEIVED"; payload: GatewayEnvelope }
  | { type: "ACTION_RECEIPT_RECEIVED"; receipt: ActionReceipt; nowMs: number }
  | { type: "ACTION_DISPATCHED"; correlationId: string; nowMs: number }
  // Liveness/stream health
  | { type: "STREAM_HEALTH_CHECK"; runId: string; nowMs: number }
  | { type: "STREAM_STALE_DETECTED"; runId: string; nowMs: number }
  | { type: "STREAM_RECOVERED"; runId: string; nowMs: number }
  // Generic
  | { type: "ERROR"; error: string }
  | { type: "LOADING"; loading: boolean };

/** Determine the run phase state from run detail and stream activity. */
export function deriveRunPhaseState(
  runDetail: RunDetail | undefined,
  liveness: RunLivenessState | undefined,
  streamStale: boolean,
): RunPhaseState {
  const status = runDetail?.status ?? "unclaimed";
  const releaseReason = runDetail?.release_reason;

  // Terminal run statuses take absolute priority.
  if (status === "released") {
    // Use release_reason to distinguish final state.
    if (releaseReason === "completed" || releaseReason === "tracker_terminal") return "completed";
    if (releaseReason === "cancelled" || releaseReason === "tracker_inactive" || releaseReason === "retry_exhausted") return "cancelled";
    return "cancelled";
  }
  if (status === "retry_queued") return "retry_queued";

  // Stream staleness overrides liveness for active runs — a stale stream should
  // show as degraded rather than collapsing into detached/failed, even when the
  // underlying liveness gap is large.
  if (streamStale && (status === "claimed" || status === "running")) return "degraded";

  // No liveness data yet means the run has not produced events; default to active.
  if (!liveness) return "active";

  return liveness.phaseState;
}

/** Compute liveness state for a run based on event recency. */
export function computeLivenessState(
  runId: string,
  existingLiveness: RunLivenessState | undefined,
  nowMs: number,
  eventsSinceLastCheck: number,
): RunLivenessState {
  const lastEventAt = existingLiveness?.lastEventAt ?? null;
  const lastEventMs = lastEventAt ? new Date(lastEventAt).getTime() : 0;
  const gapSeconds = lastEventMs > 0 ? (nowMs - lastEventMs) / 1000 : 0;

  let phaseState: RunPhaseState;
  let streamHealth: "healthy" | "stale" | "dead";

  if (eventsSinceLastCheck > 0 && gapSeconds < LIVENESS_THRESHOLDS.activeIntervalMs / 1000) {
    phaseState = "active";
    streamHealth = "healthy";
  } else if (gapSeconds < LIVENESS_THRESHOLDS.quietThresholdMs / 1000) {
    phaseState = "quiet";
    streamHealth = "healthy";
  } else if (gapSeconds < LIVENESS_THRESHOLDS.degradedThresholdMs / 1000) {
    phaseState = "degraded";
    streamHealth = "stale";
  } else if (gapSeconds < LIVENESS_THRESHOLDS.stalledThresholdMs / 1000) {
    phaseState = "stalled";
    streamHealth = "stale";
  } else {
    phaseState = "detached";
    streamHealth = "dead";
  }

  // When events arrive, update lastEventAt to the current time so the returned
  // liveness state is self-consistent and callers do not need to override it.
  const resolvedLastEventAt = eventsSinceLastCheck > 0 ? msToIso(nowMs) : lastEventAt;

  return {
    runId,
    phaseState,
    lastEventAt: resolvedLastEventAt,
    lastStatusUpdateAt: existingLiveness?.lastStatusUpdateAt ?? null,
    eventCount: (existingLiveness?.eventCount ?? 0) + eventsSinceLastCheck,
    gapSeconds,
    isStreamStale: streamHealth === "stale",
    streamHealth,
  };
}

// -- Safe Actions --

/**
 * Compute the set of safe actions for a run based on its phase state and
 * stream health. This informs the UI which controls should be enabled.
 *
 * Safety matrix (exactly matching the reviewed specification):
 *
 * | phase        | stream  | retry | cancel | rehydrate | detach |
 * |--------------|---------|-------|--------|-----------|--------|
 * | active       | healthy | false | true   | false     | false  |
 * | active       | stale   | false | true   | true      | false  |
 * | active       | dead    | false | false  | false     | true   |
 * | quiet        | healthy | false | true   | false     | false  |
 * | quiet        | stale   | false | true   | true      | false  |
 * | quiet        | dead    | false | false  | false     | true   |
 * | degraded     | healthy | false | true   | false     | false  |
 * | degraded     | stale   | false | true   | true      | false  |
 * | degraded     | dead    | false | false  | false     | true   |
 * | stalled      | healthy | true  | true   | false     | false  |
 * | stalled      | stale   | true  | true   | true      | false  |
 * | stalled      | dead    | false | false  | false     | true   |
 * | retry_queued | *       | true  | false  | false     | false  |
 * | cancelled    | *       | true  | false  | false     | false  |
 * | detached     | healthy | false | false  | false     | false  |
 * | detached     | stale   | false | false  | true      | false  |
 * | detached     | dead    | true  | false  | true      | false  |
 * | completed    | healthy | false | false  | false     | false  |
 * | completed    | stale   | false | false  | false     | false  |
 * | completed    | dead    | true  | false  | true      | false  |
 */
export function computeSafeActions(
  phase: RunPhase,
  stream: RunStreamLiveness,
): SafeActions {
  // Lookup table implementing the exact safety matrix above.
  const matrix: Record<RunPhase, Record<RunStreamLiveness, SafeActions>> = {
    active: {
      healthy: { retry: false, cancel: true, rehydrate: false, detach: false },
      stale: { retry: false, cancel: true, rehydrate: true, detach: false },
      dead: { retry: false, cancel: false, rehydrate: false, detach: true },
      degraded: { retry: false, cancel: true, rehydrate: true, detach: false },
      stalled: { retry: false, cancel: false, rehydrate: false, detach: true },
      detached: { retry: false, cancel: false, rehydrate: false, detach: true },
    },
    quiet: {
      healthy: { retry: false, cancel: true, rehydrate: false, detach: false },
      stale: { retry: false, cancel: true, rehydrate: true, detach: false },
      dead: { retry: false, cancel: false, rehydrate: false, detach: true },
      degraded: { retry: false, cancel: true, rehydrate: true, detach: false },
      stalled: { retry: false, cancel: false, rehydrate: false, detach: true },
      detached: { retry: false, cancel: false, rehydrate: false, detach: true },
    },
    degraded: {
      healthy: { retry: false, cancel: true, rehydrate: false, detach: false },
      stale: { retry: false, cancel: true, rehydrate: true, detach: false },
      dead: { retry: false, cancel: false, rehydrate: false, detach: true },
      degraded: { retry: false, cancel: true, rehydrate: true, detach: false },
      stalled: { retry: false, cancel: false, rehydrate: false, detach: true },
      detached: { retry: false, cancel: false, rehydrate: false, detach: true },
    },
    stalled: {
      healthy: { retry: true, cancel: true, rehydrate: false, detach: false },
      stale: { retry: true, cancel: true, rehydrate: true, detach: false },
      dead: { retry: false, cancel: false, rehydrate: false, detach: true },
      degraded: { retry: true, cancel: true, rehydrate: true, detach: false },
      stalled: { retry: true, cancel: true, rehydrate: true, detach: false },
      detached: { retry: false, cancel: false, rehydrate: false, detach: true },
    },
    retry_queued: {
      healthy: { retry: true, cancel: false, rehydrate: false, detach: false },
      stale: { retry: true, cancel: false, rehydrate: false, detach: false },
      dead: { retry: true, cancel: false, rehydrate: false, detach: false },
      degraded: { retry: true, cancel: false, rehydrate: false, detach: false },
      stalled: { retry: true, cancel: false, rehydrate: false, detach: false },
      detached: { retry: true, cancel: false, rehydrate: true, detach: false },
    },
    cancelled: {
      healthy: { retry: true, cancel: false, rehydrate: false, detach: false },
      stale: { retry: true, cancel: false, rehydrate: false, detach: false },
      dead: { retry: true, cancel: false, rehydrate: false, detach: false },
      degraded: { retry: true, cancel: false, rehydrate: false, detach: false },
      stalled: { retry: true, cancel: false, rehydrate: false, detach: false },
      detached: { retry: true, cancel: false, rehydrate: true, detach: false },
    },
    detached: {
      healthy: { retry: false, cancel: false, rehydrate: false, detach: false },
      stale: { retry: false, cancel: false, rehydrate: true, detach: false },
      dead: { retry: true, cancel: false, rehydrate: true, detach: false },
      degraded: { retry: false, cancel: false, rehydrate: true, detach: false },
      stalled: { retry: false, cancel: false, rehydrate: true, detach: false },
      detached: { retry: true, cancel: false, rehydrate: true, detach: false },
    },
    completed: {
      healthy: { retry: false, cancel: false, rehydrate: false, detach: false },
      stale: { retry: false, cancel: false, rehydrate: false, detach: false },
      dead: { retry: true, cancel: false, rehydrate: true, detach: false },
      degraded: { retry: false, cancel: false, rehydrate: false, detach: false },
      stalled: { retry: false, cancel: false, rehydrate: false, detach: false },
      detached: { retry: true, cancel: false, rehydrate: true, detach: false },
    },
  };

  return matrix[phase]?.[stream] ?? { retry: false, cancel: false, rehydrate: false, detach: false };
}

// -- Reducer --

/** Convert milliseconds to ISO string deterministically (for reducer purity). */
function msToIso(ms: number): string {
  return new Date(ms).toISOString();
}

export function gatewayReducer(
  state: GatewayState,
  action: GatewayAction,
): GatewayState {
  switch (action.type) {
    // -- Connection state --
    case "CONNECTION_STATE_CHANGED": {
      const connError = action.error ?? null;
      return {
        ...state,
        connection: {
          ...state.connection,
          state: action.state,
          error: connError,
          lastConnectedAt: action.state === "connected" ? msToIso(action.nowMs) : state.connection.lastConnectedAt,
          lastDisconnectedAt: action.state === "disconnected" ? msToIso(action.nowMs) : state.connection.lastDisconnectedAt,
        },
      };
    }

    case "RECONNECT_ATTEMPTED": {
      return {
        ...state,
        connection: {
          ...state.connection,
          reconnectAttempts: action.attempts,
          state: action.attempts > 0 ? "reconnecting" : state.connection.state,
          lastDisconnectedAt: action.attempts > 0 ? msToIso(action.nowMs) : state.connection.lastDisconnectedAt,
        },
      };
    }

    // -- Snapshot received --
    case "SNAPSHOT_RECEIVED":
      return {
        ...state,
        dashboard: {
          snapshot: action.payload,
          loading: false,
          error: null,
          lastUpdated: msToIso(action.nowMs),
        },
      };

    case "TASK_GRAPH_RECEIVED": {
      const nodes = new Map(action.payload.nodes.map((n) => [n.node_id, n]));
      return {
        ...state,
        taskGraph: {
          nodes,
          rootIds: action.payload.root_ids,
          loading: false,
          error: null,
          lastUpdated: msToIso(action.nowMs),
        },
      };
    }

    case "TASK_GRAPH_NODE_UPDATED": {
      const updated = action.payload;
      if (!state.taskGraph.nodes.has(updated.node_id)) {
        return state;
      }
      const nodes = new Map(state.taskGraph.nodes);
      nodes.set(updated.node_id, updated);
      return {
        ...state,
        taskGraph: {
          ...state.taskGraph,
          nodes,
          lastUpdated: msToIso(action.nowMs),
        },
      };
    }

    case "TASK_GRAPH_NODE_CREATED": {
      const created = action.payload;
      const nodes = new Map(state.taskGraph.nodes);
      nodes.set(created.node_id, created);
      const rootIds = Array.from(state.taskGraph.rootIds);
      if (!created.parent_id && !rootIds.includes(created.node_id)) {
        rootIds.push(created.node_id);
      }
      if (created.parent_id && nodes.has(created.parent_id)) {
        const parent = nodes.get(created.parent_id)!;
        if (!parent.children.includes(created.node_id)) {
          nodes.set(created.parent_id, {
            ...parent,
            children: [...parent.children, created.node_id],
          });
        }
      }
      return {
        ...state,
        taskGraph: {
          ...state.taskGraph,
          nodes,
          rootIds,
          lastUpdated: msToIso(action.nowMs),
        },
      };
    }

    case "RUN_UPDATED": {
      const runs = new Map(state.run.runs);
      runs.set(action.payload.run_id, action.payload);

      // Update entity cache.
      const cacheRuns = new Map(state.cache.runs);
      cacheRuns.set(action.payload.run_id, {
        lastSeen: msToIso(action.nowMs),
        version: (cacheRuns.get(action.payload.run_id)?.version ?? 0) + 1,
        data: action.payload,
      });

      // Update liveness state based on run status.
      const liveness = new Map(state.run.liveness);
      const existingLiveness = liveness.get(action.payload.run_id);
      if (existingLiveness) {
        liveness.set(action.payload.run_id, {
          ...existingLiveness,
          lastStatusUpdateAt: msToIso(action.nowMs),
        });
      }

      return {
        ...state,
        run: {
          ...state.run,
          runs,
          liveness,
          loading: false,
          error: null,
          lastUpdated: msToIso(action.nowMs),
        },
        cache: { ...state.cache, runs: cacheRuns },
      };
    }

    case "RUN_EVENTS_RECEIVED": {
      const runs = new Map(state.run.runs);
      const liveness = new Map(state.run.liveness);
      const existingLiveness = liveness.get(action.runId);
      const runDetail = runs.get(action.runId);

      // Use computeLivenessState for consistency with STREAM_HEALTH_CHECK
      // and to eliminate dead code path.
      const computedLiveness = computeLivenessState(
        action.runId,
        existingLiveness,
        action.nowMs,
        action.events.length,
      );

      // Respect deriveRunPhaseState's computation for all phase states — including
      // retry_queued, degraded, stalled, and quiet. Only override when the stream
      // is healthy and we have fresh events.
      const computedPhase = deriveRunPhaseState(runDetail, computedLiveness, false);

      liveness.set(action.runId, {
        ...computedLiveness,
        phaseState: computedPhase,
        isStreamStale: false,
        streamHealth: "healthy",
      });

      return {
        ...state,
        run: {
          ...state.run,
          runs,
          liveness,
          loading: false,
          error: null,
          lastUpdated: msToIso(action.nowMs),
        },
      };
    }

    case "RUN_TIMELINE_RECEIVED": {
      const timelines = new Map(state.timeline.timelines);
      timelines.set(action.runId, action.payload);
      return {
        ...state,
        timeline: {
          ...state.timeline,
          timelines,
          loading: false,
          error: null,
          lastUpdated: msToIso(action.nowMs),
        },
      };
    }

    case "TERMINAL_SEARCH_RESULT": {
      // Store search results in the terminal cache keyed by run+session.
      const key = `${action.runId}:${action.terminalSessionId}`;
      const cacheTerminals = new Map(state.cache.terminals);
      const existing = cacheTerminals.get(key);
      cacheTerminals.set(key, {
        lastSeen: msToIso(action.nowMs),
        version: (existing?.version ?? 0) + 1,
        data: action.payload,
      });
      return {
        ...state,
        cache: { ...state.cache, terminals: cacheTerminals },
      };
    }

    case "TERMINAL_JUMP_RESULT": {
      const key = `${action.runId}:${action.terminalSessionId}:jump`;
      const cacheTerminals = new Map(state.cache.terminals);
      const existing = cacheTerminals.get(key);
      cacheTerminals.set(key, {
        lastSeen: msToIso(action.nowMs),
        version: (existing?.version ?? 0) + 1,
        data: action.payload,
      });
      return {
        ...state,
        cache: { ...state.cache, terminals: cacheTerminals },
      };
    }

    case "TERMINAL_FRAMES_RECEIVED": {
      const frames = new Map(state.terminal.frames);
      const existing = frames.get(action.runId) ?? [];
      // Deduplicate by frame_sequence to handle replayed/overlapping batches.
      const existingSeqs = new Set(existing.map((f) => f.frame_sequence));
      const newFrames = action.frames.filter((f) => !existingSeqs.has(f.frame_sequence));
      frames.set(action.runId, [...existing, ...newFrames]);
      const cursor = new Map(state.terminal.cursor);
      if (newFrames.length > 0) {
        // Use max over ALL new frames to handle unsorted batches.
        const maxSeq = Math.max(...newFrames.map((f) => f.frame_sequence));
        const prevCursor = cursor.get(action.runId) ?? 0;
        cursor.set(action.runId, Math.max(prevCursor, maxSeq));
      }

      // Update entity cache.
      const cacheTerminals = new Map(state.cache.terminals);
      cacheTerminals.set(action.runId, {
        lastSeen: msToIso(action.nowMs),
        version: (cacheTerminals.get(action.runId)?.version ?? 0) + 1,
        data: action.frames,
      });

      return {
        ...state,
        terminal: {
          ...state.terminal,
          frames,
          cursor,
          loading: false,
          error: null,
          lastUpdated: msToIso(action.nowMs),
        },
        cache: { ...state.cache, terminals: cacheTerminals },
      };
    }

    case "APPROVAL_RECEIVED": {
      return {
        ...state,
        approval: {
          ...state.approval,
          pending: state.approval.pending.some(
            (a) => a.approval_id === action.payload.approval_id,
          )
            ? state.approval.pending
            : [...state.approval.pending, action.payload],
          loading: false,
          error: null,
          lastUpdated: msToIso(action.nowMs),
        },
        cache: {
          ...state.cache,
          approvals: new Map(state.cache.approvals).set(action.payload.approval_id, {
            lastSeen: msToIso(action.nowMs),
            version: 1,
            data: action.payload,
          }),
        },
      };
    }

    case "APPROVAL_RESOLVED": {
      const approvalId = action.payload.approval_id;
      const resolved = new Map(state.approval.resolved);
      resolved.set(approvalId, action.payload);

      // Update entity cache.
      const cacheApprovals = new Map(state.cache.approvals);
      cacheApprovals.set(approvalId, {
        lastSeen: msToIso(action.nowMs),
        version: (cacheApprovals.get(approvalId)?.version ?? 0) + 1,
        data: action.payload,
      });

      return {
        ...state,
        approval: {
          ...state.approval,
          pending: state.approval.pending.filter((a) => a.approval_id !== approvalId),
          resolved,
          loading: false,
          error: null,
          lastUpdated: msToIso(action.nowMs),
        },
        cache: { ...state.cache, approvals: cacheApprovals },
      };
    }

    case "PLANNING_SESSION_UPDATED": {
      const sessions = new Map(state.planning.sessions);
      sessions.set(action.payload.session_id, action.payload);

      // Update entity cache.
      const cachePlanning = new Map(state.cache.planning);
      cachePlanning.set(action.payload.session_id, {
        lastSeen: msToIso(action.nowMs),
        version: (cachePlanning.get(action.payload.session_id)?.version ?? 0) + 1,
        data: action.payload,
      });

      return {
        ...state,
        planning: {
          sessions,
          loading: false,
          error: null,
          lastUpdated: msToIso(action.nowMs),
        },
        cache: { ...state.cache, planning: cachePlanning },
      };
    }

    case "ENVELOPE_RECEIVED": {
      // Forward to appropriate slice reducer based on event_kind.
      const envelope = action.payload;
      const eventKind = envelope.event_kind;

      // Update entity cache with the envelope.
      const entityRef = envelope.entity_ref;
      const cacheEntry = {
        lastSeen: envelope.emitted_at,
        version: envelope.cursor.sequence,
        data: envelope.payload,
      };

      if (entityRef.kind === "run") {
        const cacheRuns = new Map(state.cache.runs);
        cacheRuns.set(entityRef.id, cacheEntry);
        return {
          ...state,
          cache: { ...state.cache, runs: cacheRuns },
        };
      } else if (entityRef.kind === "terminal_session") {
        const cacheTerminals = new Map(state.cache.terminals);
        cacheTerminals.set(entityRef.id, cacheEntry);
        return {
          ...state,
          cache: { ...state.cache, terminals: cacheTerminals },
        };
      } else if (entityRef.kind === "planning_session") {
        const cachePlanning = new Map(state.cache.planning);
        cachePlanning.set(entityRef.id, cacheEntry);
        return {
          ...state,
          cache: { ...state.cache, planning: cachePlanning },
        };
      }

      // For other entity kinds, just store the envelope in the cache.
      return state;
    }

    case "ACTION_RECEIPT_RECEIVED": {
      const receipts = new Map(state.actionReceipts.receipts);
      receipts.set(action.receipt.correlation_id, action.receipt);

      // Remove from pending if present.
      const pending = new Map(state.actionReceipts.pending);
      pending.delete(action.receipt.correlation_id);

      return {
        ...state,
        actionReceipts: { receipts, pending },
      };
    }

    case "ACTION_DISPATCHED": {
      const pending = new Map(state.actionReceipts.pending);
      pending.set(action.correlationId, {
        correlationId: action.correlationId,
        dispatchedAt: msToIso(action.nowMs),
      });
      return {
        ...state,
        actionReceipts: { ...state.actionReceipts, pending },
      };
    }

    // -- Liveness/stream health --

    case "STREAM_HEALTH_CHECK": {
      const { runId } = action;
      const nowMs = action.nowMs;
      const existingLiveness = state.run.liveness.get(runId);
      const runDetail = state.run.runs.get(runId);
      const isStreamStale = state.terminal.streamStale.get(runId) ?? false;

      if (!existingLiveness && !runDetail) return state;

      const livenessState = computeLivenessState(
        runId,
        existingLiveness,
        nowMs,
        0, // No new events since last health check.
      );

      // Determine phase state considering stream staleness.
      const phaseState = deriveRunPhaseState(runDetail, livenessState, isStreamStale);

      const liveness = new Map(state.run.liveness);
      liveness.set(runId, { ...livenessState, phaseState });

      return {
        ...state,
        run: { ...state.run, liveness },
      };
    }

    case "STREAM_STALE_DETECTED": {
      const streamStale = new Map(state.terminal.streamStale);
      streamStale.set(action.runId, true);

      // Update liveness to reflect stale stream — but delegate to deriveRunPhaseState
      // so terminal run statuses (released) are not collapsed into degraded.
      const liveness = new Map(state.run.liveness);
      const existing = liveness.get(action.runId);
      const runDetail = state.run.runs.get(action.runId);

      if (existing) {
        const phaseState = deriveRunPhaseState(runDetail, existing, true);
        liveness.set(action.runId, {
          ...existing,
          phaseState,
          isStreamStale: true,
          streamHealth: "stale",
        });
      } else if (runDetail) {
        // Create liveness entry for runs that haven't received events yet.
        const phaseState = deriveRunPhaseState(runDetail, undefined, true);
        liveness.set(action.runId, {
          runId: action.runId,
          phaseState,
          lastEventAt: null,
          lastStatusUpdateAt: null,
          eventCount: 0,
          gapSeconds: 0,
          isStreamStale: true,
          streamHealth: "stale",
        });
      }

      return {
        ...state,
        terminal: { ...state.terminal, streamStale },
        run: { ...state.run, liveness },
      };
    }

    case "STREAM_RECOVERED": {
      const streamStale = new Map(state.terminal.streamStale);
      streamStale.set(action.runId, false);

      // Restore liveness — recompute phaseState based on current event recency
      // so terminal run statuses (completed/cancelled) are preserved correctly
      // and non-terminal runs transition based on actual gap. Recovery implies
      // at least one event has arrived, so we signal 1 event since last check.
      const liveness = new Map(state.run.liveness);
      const existing = liveness.get(action.runId);
      const runDetail = state.run.runs.get(action.runId);

      if (existing) {
        // Update lastEventAt before computing liveness so the gap calculation
        // reflects the recovery timestamp rather than the stale previous one.
        const freshLiveness = { ...existing, lastEventAt: msToIso(action.nowMs) };
        const recomputedLiveness = computeLivenessState(
          action.runId,
          freshLiveness,
          action.nowMs,
          1, // Recovery means activity resumed.
        );
        const phaseState = deriveRunPhaseState(runDetail, recomputedLiveness, false);
        liveness.set(action.runId, {
          ...existing,
          ...recomputedLiveness,
          phaseState,
          isStreamStale: false,
          streamHealth: "healthy",
        });
      } else if (runDetail) {
        // Create liveness entry for runs that haven't received events yet.
        const phaseState = deriveRunPhaseState(runDetail, undefined, false);
        liveness.set(action.runId, {
          runId: action.runId,
          phaseState,
          lastEventAt: msToIso(action.nowMs),
          lastStatusUpdateAt: null,
          eventCount: 0,
          gapSeconds: 0,
          isStreamStale: false,
          streamHealth: "healthy",
        });
      }

      return {
        ...state,
        terminal: { ...state.terminal, streamStale },
        run: { ...state.run, liveness },
      };
    }

    // -- Error/Loading --
    case "ERROR":
      return {
        ...state,
        connection: {
          ...state.connection,
          state: action.error.includes("reconnect") ? "reconnecting" : "failed",
          error: action.error,
        },
        dashboard: { ...state.dashboard, error: action.error, loading: false },
        taskGraph: { ...state.taskGraph, error: action.error, loading: false },
        run: { ...state.run, error: action.error, loading: false },
        terminal: { ...state.terminal, error: action.error, loading: false },
        timeline: { ...state.timeline, error: action.error, loading: false },
        approval: { ...state.approval, error: action.error, loading: false },
        planning: { ...state.planning, error: action.error, loading: false },
      };

    case "LOADING": {
      const { dashboard, taskGraph, run, terminal, timeline, approval, planning, connection } = state;
      return {
        ...state,
        connection: { ...connection, error: action.loading ? null : connection.error },
        dashboard: { ...dashboard, loading: action.loading, error: action.loading ? null : dashboard.error },
        taskGraph: { ...taskGraph, loading: action.loading, error: action.loading ? null : taskGraph.error },
        run: { ...run, loading: action.loading, error: action.loading ? null : run.error },
        terminal: { ...terminal, loading: action.loading, error: action.loading ? null : terminal.error },
        timeline: { ...timeline, loading: action.loading, error: action.loading ? null : timeline.error },
        approval: { ...approval, loading: action.loading, error: action.loading ? null : approval.error },
        planning: { ...planning, loading: action.loading, error: action.loading ? null : planning.error },
      };
    }

    default:
      return state;
  }
}

// -- Re-exported types --

export type {
  GatewayEnvelope,
  DashboardSnapshot,
  TaskGraphNode,
  TaskGraphSnapshot,
  RunDetail,
  RunEvent,
  TerminalFrame,
  ApprovalRequest,
  PlanningSessionSummary,
  ActionReceipt,
  RunStatus,
  RunPhase,
  RunStreamLiveness,
  RunLivenessEnvelope,
  RunDiagnostics,
  SafeActions,
  RunTimeline,
  TerminalSearchResult,
  TerminalJumpResult,
};
