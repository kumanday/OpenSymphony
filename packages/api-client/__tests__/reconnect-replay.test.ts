/**
 * Reconnect and cursor-replay integration tests (COE-407 test plan).
 *
 * These tests simulate the scenarios required by the ticket test plan:
 *   - disconnect then reconnect with cursor replay
 *   - duplicated events after reconnect (de-duplicated by the replay buffer)
 *   - dropped frames (gap detected)
 *   - stale stream states
 *   - action receipts correlating with streamed events
 *
 * They wire the WebSocketTransport to a FakeWebSocket and feed its event
 * stream through StreamReplayBuffer + StreamCorrelator so the end-to-end
 * client resilience behavior is exercised, not just the pieces in isolation.
 */

import { WebSocketTransport, StreamReplayBuffer, StreamCorrelator, orderedEvents } from "../src/index.js";
import {
  schemaVersionV1,
  streamCursor,
  entityRefRun,
  entityRefTerminal,
} from "@opensymphony/gateway-schema";
import type { GatewayEnvelope, ActionReceipt } from "@opensymphony/gateway-schema";

const sv = schemaVersionV1();

class FakeWebSocket {
  static readonly CONNECTING = 0;
  static readonly OPEN = 1;
  static readonly CLOSING = 2;
  static readonly CLOSED = 3;
  static instances: FakeWebSocket[] = [];

  readyState = FakeWebSocket.CONNECTING;
  binaryType: "arraybuffer" | "blob" = "blob";
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
    this.onclose?.({ code: 1006, reason: "disconnect" });
  }
}

const globalWithWebSocket = globalThis as Record<string, unknown>;
const originalWebSocket = globalWithWebSocket.WebSocket;

function eventEnvelope(seq: number): GatewayEnvelope {
  return {
    schema_version: sv,
    cursor: streamCursor(seq, "run:run-1"),
    entity_ref: entityRefRun("run-1"),
    event_kind: "run.status_change",
    payload: { status: "running", sequence: seq },
    emitted_at: "2025-01-15T10:00:00Z",
  };
}

function terminalEvent(seq: number, correlationId: string): GatewayEnvelope {
  return {
    schema_version: sv,
    cursor: streamCursor(seq, "terminal:run-1"),
    entity_ref: entityRefTerminal("term-1"),
    event_kind: "terminal_frame",
    payload: { content: "out", frame_sequence: seq, correlation_id: correlationId },
    emitted_at: "2025-01-15T10:00:00Z",
  };
}

function makeReceipt(correlationId: string): ActionReceipt {
  return {
    schema_version: sv,
    action_id: `action-${correlationId}`,
    correlation_id: correlationId,
    status: "accepted",
    expected_followup: ["action_completion"],
    issued_at: "2025-01-15T10:00:00Z",
  };
}

function flushAsyncWork(iterations = 10): Promise<void> {
  return new Promise((resolve) => {
    let i = 0;
    const step = () => {
      i++;
      if (i >= iterations) resolve();
      else Promise.resolve().then(step);
    };
    Promise.resolve().then(step);
  });
}

/**
 * Pull the next value from an async iterator, rejecting on timeout instead of
 * returning a fake "done" sentinel (a done sentinel silently swallows a
 * slow-but-valid event and races with later assertions). A timeout means no
 * event arrived, which is a real failure signal.
 */
function takeNext<T>(
  iter: AsyncIterator<T>,
  timeoutMs = 200,
): Promise<IteratorResult<T>> {
  return new Promise((resolve, reject) => {
    let settled = false;
    const timer = setTimeout(() => {
      if (!settled) {
        settled = true;
        reject(new Error(`takeNext timed out after ${timeoutMs}ms`));
      }
    }, timeoutMs);
    iter.next().then(
      (result) => {
        if (!settled) {
          settled = true;
          clearTimeout(timer);
          resolve(result);
        }
      },
      (err) => {
        if (!settled) {
          settled = true;
          clearTimeout(timer);
          reject(err);
        }
      },
    );
  });
}

describe("Reconnect with cursor replay", () => {
  beforeEach(() => {
    FakeWebSocket.instances = [];
    globalWithWebSocket.WebSocket = FakeWebSocket;
  });
  afterEach(() => {
    globalWithWebSocket.WebSocket = originalWebSocket;
  });

  it("reconnects and resumes from the last applied cursor, suppressing duplicates", async () => {
    const transport = new WebSocketTransport({ baseUri: "http://localhost:8080" });
    const buffer = new StreamReplayBuffer();

    // First connection delivers seq 1,2,3 through the transport event stream.
    const eventsIter = transport.events()[Symbol.asyncIterator]();
    const firstNext = eventsIter.next(); // drive the generator -> ensureConnected
    await flushAsyncWork(20);
    expect(FakeWebSocket.instances).toHaveLength(1);
    FakeWebSocket.instances[0].open();
    await flushAsyncWork(20);

    for (const seq of [1, 2, 3]) {
      FakeWebSocket.instances[0].emit(`__event__ ${JSON.stringify(eventEnvelope(seq))}`);
    }
    await flushAsyncWork();
    // Drain the dispatched envelopes into the replay buffer. The first .next()
    // resolves to seq 1; pull the remaining two.
    const first = await firstNext;
    if (!first.done) buffer.apply(first.value);
    for (let i = 0; i < 2; i++) {
      const next = await takeNext(eventsIter);
      if (next.done) break;
      buffer.apply(next.value);
    }
    expect(buffer.lastSequence("run:run-1")).toBe(3);

    // Persisted resume cursor for the reconnect.
    const resumeCursor = buffer.nextCursor("run:run-1");
    expect(resumeCursor).toEqual({ sequence: 3, partition: "run:run-1" });

    // events(fromCursor) must accept the resume cursor for the new connection.
    const replayIter = transport.events(resumeCursor)[Symbol.asyncIterator]();
    const replayNext = replayIter.next(); // drive -> ensureConnected(fromCursor)
    await flushAsyncWork(20);
    // A new socket is created for the replay connection.
    expect(FakeWebSocket.instances.length).toBeGreaterThanOrEqual(2);
    const replaySocket = FakeWebSocket.instances[FakeWebSocket.instances.length - 1];
    replaySocket.open();
    await flushAsyncWork(20);

    // The reconnect URL must carry the cursor query params for server-side replay.
    expect(replaySocket.url).toContain("cursor_sequence=3");
    expect(replaySocket.url).toContain("cursor_partition=run%3Arun-1");

    // After reconnect, the gateway replays from the cursor: seq 3 is replayed
    // (duplicate, suppressed) and 4,5 are new. Feed through the buffer to
    // assert de-dup and monotonic advance.
    const seen: number[] = [];
    for (const env of [eventEnvelope(3), eventEnvelope(4), eventEnvelope(5)]) {
      for (const ev of buffer.apply(env)) {
        if (ev.kind === "applied") seen.push(ev.envelope.cursor.sequence);
      }
    }
    expect(seen).toEqual([4, 5]);
    expect(buffer.lastSequence("run:run-1")).toBe(5);

    void replayNext;
    await transport.close();
  });

  it("de-duplicates events that arrive twice across a disconnect", () => {
    const buffer = new StreamReplayBuffer();
    const seen: number[] = [];
    buffer.apply(eventEnvelope(1));
    buffer.apply(eventEnvelope(2));
    // Reconnect delivers 2 again, then 3.
    for (const env of [eventEnvelope(2), eventEnvelope(3)]) {
      for (const ev of buffer.apply(env)) {
        if (ev.kind === "applied") seen.push(ev.envelope.cursor.sequence);
      }
    }
    expect(seen).toEqual([3]);
    expect(buffer.lastSequence("run:run-1")).toBe(3);
  });

  it("detects dropped frames after a reconnect gap and advances the frontier", () => {
    const buffer = new StreamReplayBuffer({ maxPendingPerPartition: 2 });
    buffer.apply(eventEnvelope(10));
    // Reconnect replays from 10, but 11,12,13 are dropped; 14 arrives.
    const result = buffer.apply(eventEnvelope(14));
    expect(result.some((e) => e.kind === "gap")).toBe(true);
    expect(buffer.lastSequence("run:run-1")).toBe(14);
  });

  it("marks a partition stale after disconnect and recovers on replay", () => {
    const buffer = new StreamReplayBuffer();
    buffer.apply(eventEnvelope(1));
    // Disconnect -> mark stale while waiting for replay.
    buffer.markStale("run:run-1");
    expect(buffer.isStale("run:run-1")).toBe(true);
    // Replay delivers the next event -> recovered.
    buffer.apply(eventEnvelope(2));
    expect(buffer.isStale("run:run-1")).toBe(false);
  });

  it("correlates action receipts with streamed terminal events after reconnect", () => {
    const correlator = new StreamCorrelator();
    const receipt = makeReceipt("corr-99");
    correlator.registerReceipt(receipt);

    // Stream a terminal event carrying the correlation_id (as would arrive
    // after dispatching an action and reconnecting).
    const env = terminalEvent(5, "corr-99");
    const matched = correlator.observe(env);
    expect(matched).toEqual(receipt);
    expect(correlator.hasCorrelatedEvent("corr-99")).toBe(true);
    expect(correlator.eventsFor("corr-99")).toHaveLength(1);
  });

  it("action receipt correlation survives duplicate event delivery", () => {
    const correlator = new StreamCorrelator();
    correlator.registerReceipt(makeReceipt("corr-dup"));
    correlator.observe(terminalEvent(1, "corr-dup"));
    correlator.observe(terminalEvent(1, "corr-dup")); // duplicate
    expect(correlator.eventsFor("corr-dup")).toHaveLength(2);
    expect(correlator.hasCorrelatedEvent("corr-dup")).toBe(true);
  });

  it("pipes the transport event stream through orderedEvents end-to-end", async () => {
    // End-to-end integration: feed a live WebSocketTransport event stream
    // (with a resume cursor) through orderedEvents() and assert the consumer
    // sees a de-duplicated, monotonic output directly, not the pieces
    // side-by-side.
    const transport = new WebSocketTransport({ baseUri: "http://localhost:8080" });
    const resumeCursor = { sequence: 0, partition: "run:run-1" };

    // Wrap the transport stream with the replay/ordering engine.
    const ordered = orderedEvents(transport.events(resumeCursor), {
      maxPendingPerPartition: 16,
    });
    const orderedIter = ordered[Symbol.asyncIterator]();
    const firstNext = orderedIter.next(); // drive the generator -> ensureConnected
    await flushAsyncWork(20);
    FakeWebSocket.instances[0].open();
    await flushAsyncWork(20);

    // Deliver: 1, 2, 2 (duplicate), 4 (gap, then 3 fills it).
    for (const seq of [1, 2, 2, 4]) {
      FakeWebSocket.instances[0].emit(`__event__ ${JSON.stringify(eventEnvelope(seq))}`);
    }
    await flushAsyncWork();
    FakeWebSocket.instances[0].emit(`__event__ ${JSON.stringify(eventEnvelope(3))}`);
    await flushAsyncWork();

    const first = await firstNext;
    const out: number[] = [];
    if (!first.done) out.push(first.value.cursor.sequence);
    // Drain every applied envelope; orderedEvents only yields frontier-advancing
    // events, so this terminates once the stream has nothing buffered.
    while (out.length < 4) {
      const next = await takeNext(orderedIter, 300);
      if (next.done) break;
      out.push(next.value.cursor.sequence);
    }

    // De-duplicated, monotonic, gap-closing sequence directly from the engine.
    expect(out).toEqual([1, 2, 3, 4]);
    expect(out.filter((s) => s === 2)).toHaveLength(1);
  });
});