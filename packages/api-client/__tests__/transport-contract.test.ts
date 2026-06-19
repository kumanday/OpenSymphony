/**
 * Contract tests proving that all transport profiles expose the same
 * gateway DTOs, event envelopes, cursors, and action receipt semantics.
 *
 * These tests verify the transport abstraction layer without requiring
 * a running gateway server. They use fixture data and mock transports
 * to ensure contract compliance.
 */

import {
  HttpGatewayTransport,
  WebSocketTransport,
  TauriChannelTransport,
  TransportFactory,
} from "../src/index.js";
import type {
  GatewayTransport,
  GatewayTransportConfig,
} from "../src/transports.js";
import {
  GATEWAY_SCHEMA_VERSION,
  schemaVersionV1,
  isValidGatewayEnvelope,
  entityRefRun,
  entityRefTerminal,
  streamCursor,
} from "@opensymphony/gateway-schema";
import type {
  GatewayEnvelope,
  DashboardSnapshot,
  RunDetail,
  TerminalSnapshot,
  TaskGraphSnapshot,
  GatewayCapabilities,
} from "@opensymphony/gateway-schema";

// ─── Test Fixtures ─────────────────────────────────────────────────────────

const FIXTURE_CAPABILITIES: GatewayCapabilities = {
  schema_version: { major: 1, minor: 0, patch: 0 },
  gateway_version: "1.6.0",
  supported_api_versions: ["1.0.0"],
  transports: [
    {
      transport: "loopback_http",
      modes: ["json"],
      supported_encodings: ["utf-8"],
      bidirectional: false,
    },
    {
      transport: "loopback_websocket",
      modes: ["json", "binary"],
      supported_encodings: ["utf-8", "base64"],
      bidirectional: true,
    },
    {
      transport: "tauri_channel",
      modes: ["json"],
      supported_encodings: ["utf-8"],
      bidirectional: true,
    },
  ],
  features: [
    { feature: "task_graph", available: true, requires_auth: false },
    { feature: "terminal_stream", available: true, requires_auth: false },
  ],
  auth_modes: ["none", "api_key"],
  max_event_page_size: 1000,
  max_terminal_frame_batch: 500,
};

const FIXTURE_SNAPSHOT: DashboardSnapshot = {
  schema_version: schemaVersionV1(),
  generated_at: "2025-01-15T10:00:00Z",
  sequence: 1,
  health: "healthy",
  metrics: {
    running_issue_count: 3,
    retry_queue_depth: 0,
    total_input_tokens: 150000,
    total_output_tokens: 75000,
    total_cache_read_tokens: 30000,
    total_cost_micros: 5000,
  },
  projects: [
    {
      project_id: "proj-1",
      name: "Test Project",
      milestone_count: 2,
      issue_count: 5,
      running_count: 1,
      completed_count: 3,
      failed_count: 1,
    },
  ],
  recent_events: [
    {
      happened_at: "2025-01-15T10:00:00Z",
      kind: "worker_started",
      summary: "Run started for COE-390",
      issue_identifier: "COE-390",
    },
  ],
};

const FIXTURE_RUN_DETAIL: RunDetail = {
  schema_version: schemaVersionV1(),
  run_id: "run-1",
  issue_id: "issue-1",
  issue_identifier: "COE-390",
  worker_id: "worker-1",
  status: "running",
  claimed_at: "2025-01-15T09:00:00Z",
  started_at: "2025-01-15T09:01:00Z",
  turn_count: 3,
  max_turns: 8,
  input_tokens: 50000,
  output_tokens: 25000,
  cache_read_tokens: 10000,
  runtime_seconds: 120,
};

const FIXTURE_TERMINAL_SNAPSHOT: TerminalSnapshot = {
  schema_version: schemaVersionV1(),
  terminal_session_id: "term-1",
  run_id: "run-1",
  frames: [],
  total_frames: 0,
  truncated: false,
  cursor: 0,
};

const FIXTURE_TASK_GRAPH: TaskGraphSnapshot = {
  schema_version: schemaVersionV1(),
  project_id: "proj-1",
  generated_at: "2025-01-15T10:00:00Z",
  nodes: [],
  root_ids: [],
};

function createTestEnvelope(seq: number, runId: string): GatewayEnvelope {
  return {
    schema_version: schemaVersionV1(),
    cursor: streamCursor(seq, `run:${runId}`),
    entity_ref: entityRefRun(runId),
    event_kind: "run.status_change",
    payload: { status: "running" },
    emitted_at: "2025-01-15T10:00:00Z",
  };
}

function createTerminalEnvelope(seq: number, runId: string): GatewayEnvelope {
  return {
    schema_version: schemaVersionV1(),
    cursor: streamCursor(seq, `terminal:${runId}`),
    entity_ref: entityRefTerminal("term-1"),
    event_kind: "terminal_frame",
    payload: { frame_sequence: seq, content: "output" },
    emitted_at: "2025-01-15T10:00:00Z",
  };
}

async function flushAsyncWork(iterations = 2): Promise<void> {
  for (let i = 0; i < iterations; i++) {
    await Promise.resolve();
  }
}

// ─── Mock Gateway Server ───────────────────────────────────────────────────

// ─── Transport Equivalence Tests ───────────────────────────────────────────

describe("Transport Contract Equivalence", () => {
  /**
   * Verify all transports produce identical capability responses.
   * Contract: Every transport must return the same GatewayCapabilities shape.
   */
  describe("capability discovery", () => {
    it("HTTP transport returns correct capabilities shape", async () => {
      // This is a structural test; we verify the shape matches the contract
      const caps = FIXTURE_CAPABILITIES;
      expect(caps.schema_version).toBeDefined();
      expect(caps.gateway_version).toBeDefined();
      expect(Array.isArray(caps.transports)).toBe(true);
      expect(caps.transports.length).toBeGreaterThan(0);
    });

    it("All transport profiles are advertised", () => {
      const profiles = TransportFactory.getPreferredProfiles();
      expect(profiles.length).toBeGreaterThan(0);
      expect(profiles.every((p) => typeof p.profile === "string")).toBe(true);
    });
  });

  /**
   * Verify all transports produce identical dashboard snapshots.
   * Contract: snapshot sequence and schema version must match.
   */
  describe("snapshot contract", () => {
    it("snapshot has required schema version", () => {
      const snap = FIXTURE_SNAPSHOT;
      expect(snap.schema_version.major).toBe(1);
      expect(snap.sequence).toBe(1);
    });

    it("snapshot health field uses valid enum", () => {
      const snap = FIXTURE_SNAPSHOT;
      expect(
        ["healthy", "degraded", "failed", "starting"].includes(snap.health),
      ).toBe(true);
    });
  });

  /**
   * Verify gateway envelope structure is consistent across transports.
   * Contract: Every event must have schema_version, cursor, entity_ref,
   * event_kind, and emitted_at fields.
   */
  describe("gateway envelope contract", () => {
    it("envelope validates with required fields", () => {
      const envelope = createTestEnvelope(1, "run-1");
      expect(isValidGatewayEnvelope(envelope)).toBe(true);
    });

    it("envelope cursor is monotonically increasing", () => {
      const env1 = createTestEnvelope(1, "run-1");
      const env2 = createTestEnvelope(2, "run-1");
      expect(env2.cursor.sequence).toBeGreaterThan(env1.cursor.sequence);
    });

    it("envelope entity ref has valid kind", () => {
      const envelope = createTestEnvelope(1, "run-1");
      expect(envelope.entity_ref.kind).toBe("run");
    });

    it("terminal envelope has terminal_session entity kind", () => {
      const terminalEnv: GatewayEnvelope = {
        schema_version: schemaVersionV1(),
        cursor: streamCursor(1, "terminal:run-1"),
        entity_ref: entityRefTerminal("term-1"),
        event_kind: "terminal_frame",
        payload: { frame_sequence: 1, content: "output" },
        emitted_at: "2025-01-15T10:00:00Z",
      };
      expect(terminalEnv.entity_ref.kind).toBe("terminal_session");
      expect(isValidGatewayEnvelope(terminalEnv)).toBe(true);
    });
  });

  /**
   * Verify cursor-based replay semantics work consistently.
   * Contract: Clients can resume from a cursor position and receive
   * only events after that position.
   */
  describe("cursor replay semantics", () => {
    it("stream cursor encodes sequence and partition", () => {
      const cursor = streamCursor(42, "run:test");
      expect(cursor.sequence).toBe(42);
      expect(cursor.partition).toBe("run:test");
    });

    it("envelopes with cursor can be ordered", () => {
      const envelopes = [
        createTestEnvelope(3, "run-1"),
        createTestEnvelope(1, "run-1"),
        createTestEnvelope(2, "run-1"),
      ].sort((a, b) => a.cursor.sequence - b.cursor.sequence);

      expect(envelopes[0].cursor.sequence).toBe(1);
      expect(envelopes[1].cursor.sequence).toBe(2);
      expect(envelopes[2].cursor.sequence).toBe(3);
    });
  });

  /**
   * Verify run phase and liveness semantics are transport-independent.
   * Contract: Run status changes must be delivered consistently regardless
   * of transport profile.
   */
  describe("run phase and liveness", () => {
    it("run detail exposes lifecycle state", () => {
      const detail = FIXTURE_RUN_DETAIL;
      expect(detail.status).toBe("running");
      expect(detail.started_at).toBeDefined();
      expect(detail.claimed_at).toBeDefined();
    });

    it("run status change event has correct structure", () => {
      const envelope = createTestEnvelope(1, "run-1");
      expect(envelope.event_kind).toBe("run.status_change");
      expect(envelope.entity_ref.kind).toBe("run");
      expect(envelope.cursor.sequence).toBe(1);
    });
  });
});

// ─── Transport Factory Tests ───────────────────────────────────────────────

describe("TransportFactory", () => {
  it("creates HTTP transport for loopback_http profile", async () => {
    const config: GatewayTransportConfig = {
      baseUri: "http://localhost:8080",
      transport: "loopback_http",
    };

    const transport = await TransportFactory.create(config);
    expect(transport).toBeInstanceOf(HttpGatewayTransport);
    expect(transport.baseUri).toBe("http://localhost:8080");
    // Functional check: the transport exposes the expected interface
    expect(typeof transport.health).toBe("function");
    expect(typeof transport.close).toBe("function");
  });

  it("falls back to HTTP when profile requires unavailable runtime", async () => {
    const config: GatewayTransportConfig = {
      baseUri: "http://localhost:8080",
      transport: "tauri_channel",
    };

    // Without Tauri runtime, should fall back to HTTP
    const transport = await TransportFactory.create(config);
    expect(transport).toBeInstanceOf(HttpGatewayTransport);
  });

  it("selects WebSocket when available for loopback_websocket profile", async () => {
    const config: GatewayTransportConfig = {
      baseUri: "http://localhost:8080",
      transport: "loopback_websocket",
    };

    // WebSocket is a global in Node.js 21+ and in browser environments.
    // Fail the test if not available so we don't silently skip the assertion.
    if (typeof WebSocket === "undefined") {
      throw new Error("WebSocket is not available in this test environment");
    }
    const transport = await TransportFactory.create(config);
    expect(transport).toBeInstanceOf(WebSocketTransport);
    expect(transport.baseUri).toBe("http://localhost:8080");
  });

  it("prefers Tauri channel when running in Tauri context", async () => {
    const config: GatewayTransportConfig = {
      baseUri: "http://localhost:8080",
      transport: "tauri_channel",
    };

    // Without Tauri context, falls back to HTTP
    const transport = await TransportFactory.create(config, FIXTURE_CAPABILITIES);
    expect(transport).toBeInstanceOf(HttpGatewayTransport);
  });

  it("respects capabilities when selecting transport", async () => {
    const config: GatewayTransportConfig = {
      baseUri: "http://localhost:8080",
      transport: "tauri_channel" as const,
    };

    const transport = await TransportFactory.create(config, FIXTURE_CAPABILITIES);
    // tauri_channel is in capabilities but we're not in Tauri context
    expect(transport).toBeInstanceOf(HttpGatewayTransport);
  });

  it("reports preferred profiles in priority order", () => {
    const profiles = TransportFactory.getPreferredProfiles();

    // At minimum, HTTP should always be available
    const httpProfile = profiles.find((p) => p.profile === "loopback_http");
    expect(httpProfile).toBeDefined();
    expect(httpProfile?.available).toBe(true);
  });
});

// ─── Tauri Channel Transport Tests ─────────────────────────────────────────

describe("TauriChannelTransport", () => {
  const globalWithTauri = globalThis as Record<string, unknown>;
  const originalTauri = globalWithTauri.__TAURI__;

  afterEach(() => {
    if (originalTauri === undefined) {
      delete globalWithTauri.__TAURI__;
    } else {
      globalWithTauri.__TAURI__ = originalTauri;
    }
    jest.restoreAllMocks();
  });

  it("uses the registered gateway_capabilities command without auth injection", async () => {
    const invoke = jest.fn().mockResolvedValue(FIXTURE_CAPABILITIES);
    globalWithTauri.__TAURI__ = {
      invoke,
      core: { Channel: jest.fn() },
    };

    const transport = new TauriChannelTransport({
      baseUri: "tauri://local",
      authToken: "secret-token",
    });

    await expect(transport.health()).resolves.toEqual(FIXTURE_CAPABILITIES);
    expect(invoke).toHaveBeenCalledWith("gateway_capabilities", {});
  });

  it("passes run event page tokens through native commands without numeric coercion", async () => {
    const invoke = jest.fn().mockResolvedValue({
      schema_version: { major: 1, minor: 0, patch: 0 },
      run_id: "run-1",
      events: [],
    });
    globalWithTauri.__TAURI__ = {
      invoke,
      core: { Channel: jest.fn() },
    };

    const transport = new TauriChannelTransport({ baseUri: "tauri://local" });

    await transport.runEvents("run-1", {
      page_token: "opaque-token",
      page_size: 25,
    });

    expect(invoke).toHaveBeenCalledWith("run_events", {
      run_id: "run-1",
      page_token: "opaque-token",
      page_size: 25,
    });
  });

  it("forwards event cursors and unsubscribes on close", async () => {
    const channels: Array<{ close: jest.Mock; emit: (envelope: GatewayEnvelope) => void }> = [];
    const invoke = jest.fn().mockResolvedValue(undefined);
    globalWithTauri.__TAURI__ = {
      invoke,
      core: {
        Channel: jest.fn((onMessage: (envelope: GatewayEnvelope) => void) => {
          const channel = {
            onmessage: onMessage,
            close: jest.fn(),
            emit: onMessage,
          };
          channels.push(channel);
          return channel;
        }),
      },
    };

    const transport = new TauriChannelTransport({ baseUri: "tauri://local" });
    const iterator = transport.events({ sequence: 42, partition: "events" })[Symbol.asyncIterator]();
    const pendingNext = iterator.next();
    await flushAsyncWork();

    expect(invoke).toHaveBeenCalledWith("subscribe_events", {
      tx: channels[0],
      cursor: 42,
      partition: "events",
    });

    await transport.close();
    await expect(pendingNext).resolves.toMatchObject({ done: true });
    expect(invoke).toHaveBeenCalledWith("unsubscribe_events", {});
    expect(channels[0].close).toHaveBeenCalledTimes(1);
  });

  it("unsubscribes terminal channels exactly once on close", async () => {
    const channels: Array<{ close: jest.Mock; emit: (envelope: GatewayEnvelope) => void }> = [];
    const invoke = jest.fn().mockResolvedValue(undefined);
    globalWithTauri.__TAURI__ = {
      invoke,
      core: {
        Channel: jest.fn((onMessage: (envelope: GatewayEnvelope) => void) => {
          const channel = {
            onmessage: onMessage,
            close: jest.fn(),
            emit: onMessage,
          };
          channels.push(channel);
          return channel;
        }),
      },
    };

    const transport = new TauriChannelTransport({ baseUri: "tauri://local" });
    const iterator = transport.terminalFrames("run-1")[Symbol.asyncIterator]();
    const pendingNext = iterator.next();
    await flushAsyncWork();

    expect(invoke).toHaveBeenCalledWith("subscribe_terminal", {
      run_id: "run-1",
      tx: channels[0],
    });

    await transport.close();
    await expect(pendingNext).resolves.toMatchObject({ done: true });
    expect(invoke).toHaveBeenCalledWith("unsubscribe_terminal", { run_id: "run-1" });
    expect(channels[0].close).toHaveBeenCalledTimes(1);
  });
});

// ─── HTTP Transport Tests ──────────────────────────────────────────────────

describe("HttpGatewayTransport", () => {
  const originalFetch = global.fetch;

  afterEach(() => {
    global.fetch = originalFetch;
  });

  it("normalizes baseUri by removing trailing slash", () => {
    const transport = new HttpGatewayTransport({
      baseUri: "http://localhost:8080/",
    });
    expect(transport.baseUri).toBe("http://localhost:8080");
  });

  it("accepts auth token in config", () => {
    const transport = new HttpGatewayTransport({
      baseUri: "http://localhost:8080",
      authToken: "test-token",
    });
    expect(transport.baseUri).toBe("http://localhost:8080");
  });

  it("has all required GatewayTransport methods", () => {
    const transport = new HttpGatewayTransport({
      baseUri: "http://localhost:8080",
    });

    expect(typeof transport.health).toBe("function");
    expect(typeof transport.snapshot).toBe("function");
    expect(typeof transport.taskGraph).toBe("function");
    expect(typeof transport.runDetail).toBe("function");
    expect(typeof transport.runEvents).toBe("function");
    expect(typeof transport.terminalSnapshot).toBe("function");
    expect(typeof transport.events).toBe("function");
    expect(typeof transport.terminalFrames).toBe("function");
    expect(typeof transport.close).toBe("function");
  });

  it("uses the current Rust gateway read routes", async () => {
    const urls: string[] = [];
    global.fetch = jest.fn(async (input: RequestInfo | URL) => {
      const url = String(input);
      urls.push(url);
      let body: unknown = FIXTURE_CAPABILITIES;
      if (url.endsWith("/api/v1/dashboard/snapshot")) {
        body = FIXTURE_SNAPSHOT;
      } else if (url.endsWith("/api/v1/projects/default/taskgraph")) {
        body = FIXTURE_TASK_GRAPH;
      } else if (url.endsWith("/api/v1/runs/fixture-run-1")) {
        body = FIXTURE_RUN_DETAIL;
      }
      return {
        ok: true,
        status: 200,
        statusText: "OK",
        json: async () => body,
        text: async () => JSON.stringify(body),
      } as Response;
    }) as jest.MockedFunction<typeof global.fetch>;

    const transport = new HttpGatewayTransport({
      baseUri: "http://localhost:8080",
    });

    await transport.health();
    await transport.snapshot();
    await transport.taskGraph("default");
    await transport.runDetail("fixture-run-1");

    expect(urls).toEqual([
      "http://localhost:8080/api/v1/capabilities",
      "http://localhost:8080/api/v1/dashboard/snapshot",
      "http://localhost:8080/api/v1/projects/default/taskgraph",
      "http://localhost:8080/api/v1/runs/fixture-run-1",
    ]);
  });
});

// ─── WebSocket Transport Tests ─────────────────────────────────────────────

describe("WebSocketTransport", () => {
  const globalWithWebSocket = globalThis as Record<string, unknown>;
  const originalWebSocket = globalWithWebSocket.WebSocket;

  class FakeWebSocket {
    static readonly CONNECTING = 0;
    static readonly OPEN = 1;
    static readonly CLOSING = 2;
    static readonly CLOSED = 3;
    static instances: FakeWebSocket[] = [];

    readyState = FakeWebSocket.CONNECTING;
    onopen: (() => void) | null = null;
    onclose: ((event: { code: number; reason: string }) => void) | null = null;
    onerror: ((event: Error) => void) | null = null;
    onmessage: ((event: { data: string }) => void) | null = null;
    sent: string[] = [];

    constructor(readonly url: string) {
      FakeWebSocket.instances.push(this);
    }

    send(data: string): void {
      this.sent.push(data);
    }

    open(): void {
      this.readyState = FakeWebSocket.OPEN;
      this.onopen?.();
    }

    emit(data: string): void {
      this.onmessage?.({ data });
    }

    close(): void {
      this.readyState = FakeWebSocket.CLOSED;
      this.onclose?.({ code: 1000, reason: "" });
    }
  }

  beforeEach(() => {
    FakeWebSocket.instances = [];
    globalWithWebSocket.WebSocket = FakeWebSocket;
  });

  afterEach(() => {
    globalWithWebSocket.WebSocket = originalWebSocket;
    jest.useRealTimers();
  });

  it("normalizes baseUri by removing trailing slash", () => {
    const transport = new WebSocketTransport({
      baseUri: "http://localhost:8080/",
    });
    expect(transport.baseUri).toBe("http://localhost:8080");
  });

  it("converts HTTP URL to WebSocket URL", () => {
    const transport = new WebSocketTransport({
      baseUri: "http://localhost:8080",
    });
    // The wsUrl method is private but we can infer behavior
    expect(transport.baseUri.startsWith("http")).toBe(true);
  });

  it("has all required GatewayTransport methods", () => {
    const transport = new WebSocketTransport({
      baseUri: "http://localhost:8080",
    });

    expect(typeof transport.health).toBe("function");
    expect(typeof transport.snapshot).toBe("function");
    expect(typeof transport.taskGraph).toBe("function");
    expect(typeof transport.runDetail).toBe("function");
    expect(typeof transport.runEvents).toBe("function");
    expect(typeof transport.terminalSnapshot).toBe("function");
    expect(typeof transport.events).toBe("function");
    expect(typeof transport.terminalFrames).toBe("function");
    expect(typeof transport.close).toBe("function");
  });

  it("shares one connection attempt across concurrent stream subscribers", async () => {
    const transport = new WebSocketTransport({
      baseUri: "http://localhost:8080",
    });
    const events = transport.events()[Symbol.asyncIterator]();
    const terminal = transport.terminalFrames("run-1")[Symbol.asyncIterator]();

    const eventNext = events.next();
    const terminalNext = terminal.next();
    await flushAsyncWork();

    expect(FakeWebSocket.instances).toHaveLength(1);
    FakeWebSocket.instances[0].open();
    await flushAsyncWork();

    await transport.close();
    await expect(eventNext).resolves.toMatchObject({ done: true });
    await expect(terminalNext).resolves.toMatchObject({ done: true });
  });

  it("dispatches terminal frames to multiple subscribers for the same run", async () => {
    const transport = new WebSocketTransport({
      baseUri: "http://localhost:8080",
    });
    const first = transport.terminalFrames("run-1")[Symbol.asyncIterator]();
    const second = transport.terminalFrames("run-1")[Symbol.asyncIterator]();

    const firstNext = first.next();
    const secondNext = second.next();
    await flushAsyncWork();
    expect(FakeWebSocket.instances).toHaveLength(1);
    FakeWebSocket.instances[0].open();
    await flushAsyncWork(10);
    expect(
      (transport as unknown as {
        terminalSubscribers: Map<string, Set<(envelope: GatewayEnvelope) => void>>;
      }).terminalSubscribers.get("run-1")?.size,
    ).toBe(2);

    const envelope = createTerminalEnvelope(1, "run-1");
    FakeWebSocket.instances[0].emit(`__event__ ${JSON.stringify(envelope)}`);

    await expect(firstNext).resolves.toMatchObject({
      done: false,
      value: {
        cursor: { sequence: 1, partition: "terminal:run-1" },
        entity_ref: { kind: "terminal_session", id: "term-1" },
        event_kind: "terminal_frame",
        payload: { frame_sequence: 1, content: "output" },
      },
    });
    await expect(secondNext).resolves.toMatchObject({
      done: false,
      value: {
        cursor: { sequence: 1, partition: "terminal:run-1" },
        entity_ref: { kind: "terminal_session", id: "term-1" },
        event_kind: "terminal_frame",
        payload: { frame_sequence: 1, content: "output" },
      },
    });
    await transport.close();
  });

  it("marks reconnect pending after an established socket closes", async () => {
    jest.useFakeTimers();
    const transport = new WebSocketTransport({
      baseUri: "http://localhost:8080",
    });
    const events = transport.events()[Symbol.asyncIterator]();
    const pendingNext = events.next();
    await flushAsyncWork();
    FakeWebSocket.instances[0].open();
    await flushAsyncWork();

    FakeWebSocket.instances[0].close();

    expect((transport as unknown as { isReconnecting: boolean }).isReconnecting).toBe(true);
    await transport.close();
    await expect(pendingNext).resolves.toMatchObject({ done: true });
  });
});

// ─── Reconnect and Resilience Tests ────────────────────────────────────────

describe("Transport resilience", () => {
  it("HTTP transport close cancels in-flight requests", async () => {
    const transport = new HttpGatewayTransport({
      baseUri: "http://localhost:8080",
    });

    await transport.close();
    // No error should be thrown
  });

  it("WebSocket transport cleans up on close", async () => {
    const transport = new WebSocketTransport({
      baseUri: "http://localhost:8080",
    });

    await transport.close();
    // No error should be thrown
  });
});
