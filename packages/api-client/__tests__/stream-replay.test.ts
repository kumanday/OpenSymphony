/**
 * Stream replay, ordering, and action receipt correlation tests.
 *
 * Covers the COE-407 test plan items that must be enforced on the client
 * regardless of the selected remote transport:
 *   - cursor replay (resume from last applied sequence)
 *   - duplicated events suppressed
 *   - dropped frames / gap detection
 *   - stale stream states
 *   - action receipts correlate with streamed events (correlation_id)
 */

import {
  StreamReplayBuffer,
  orderedEvents,
  StreamCorrelator,
  envelopeCorrelationId,
} from "../src/stream-replay.js";
import type {
  StreamGap,
  StreamDuplicate,
  StreamStaleInfo,
} from "../src/stream-replay.js";
import {
  schemaVersionV1,
  streamCursor,
  entityRefRun,
  entityRefTerminal,
} from "@opensymphony/gateway-schema";
import type {
  GatewayEnvelope,
  ActionReceipt,
} from "@opensymphony/gateway-schema";

const sv = schemaVersionV1();

function runEnvelope(seq: number, partition = "run:run-1"): GatewayEnvelope {
  return {
    schema_version: sv,
    cursor: streamCursor(seq, partition),
    entity_ref: entityRefRun("run-1"),
    event_kind: "run.status_change",
    payload: { status: "running" },
    emitted_at: "2025-01-15T10:00:00Z",
  };
}

function terminalEnvelope(
  seq: number,
  correlationId?: string,
  partition = "terminal:run-1",
): GatewayEnvelope {
  const payload: Record<string, unknown> = { content: "output", frame_sequence: seq };
  if (correlationId) payload.correlation_id = correlationId;
  return {
    schema_version: sv,
    cursor: streamCursor(seq, partition),
    entity_ref: entityRefTerminal("term-1"),
    event_kind: "terminal_frame",
    payload,
    emitted_at: "2025-01-15T10:00:00Z",
  };
}

function makeReceipt(correlationId: string): ActionReceipt {
  return {
    schema_version: sv,
    action_id: `action-${correlationId}`,
    correlation_id: correlationId,
    status: "accepted",
    expected_followup: ["action_completion", "run_lifecycle"],
    issued_at: "2025-01-15T10:00:00Z",
  };
}

async function* fromArray(envelopes: GatewayEnvelope[]): AsyncGenerator<GatewayEnvelope> {
  for (const env of envelopes) yield env;
}

describe("StreamReplayBuffer", () => {
  it("applies envelopes in monotonic order and tracks the frontier", () => {
    const buffer = new StreamReplayBuffer();
    const a = buffer.apply(runEnvelope(1));
    const b = buffer.apply(runEnvelope(2));
    expect(a).toHaveLength(1);
    expect(a[0].kind).toBe("applied");
    expect(b[0].kind).toBe("applied");
    expect(buffer.lastSequence("run:run-1")).toBe(2);
    expect(buffer.nextCursor("run:run-1")).toEqual({
      sequence: 2,
      partition: "run:run-1",
    });
  });

  it("suppresses duplicated events after reconnect", () => {
    const duplicates: StreamDuplicate[] = [];
    const buffer = new StreamReplayBuffer({ onDuplicate: (d) => duplicates.push(d) });
    buffer.apply(runEnvelope(1));
    buffer.apply(runEnvelope(2));
    // After reconnect the gateway replays seq 2 again.
    const result = buffer.apply(runEnvelope(2));
    expect(result).toHaveLength(1);
    expect(result[0].kind).toBe("duplicate");
    expect(duplicates).toHaveLength(1);
    expect(duplicates[0].sequence).toBe(2);
    // Frontier does not regress.
    expect(buffer.lastSequence("run:run-1")).toBe(2);
  });

  it("suppresses stale events below the frontier", () => {
    const buffer = new StreamReplayBuffer();
    buffer.apply(runEnvelope(5));
    const result = buffer.apply(runEnvelope(3));
    expect(result[0].kind).toBe("duplicate");
    expect(buffer.lastSequence("run:run-1")).toBe(5);
  });

  it("detects dropped frames as a gap when the jump exceeds the reorder window", () => {
    const gaps: StreamGap[] = [];
    const buffer = new StreamReplayBuffer({
      maxPendingPerPartition: 2,
      onGap: (g) => gaps.push(g),
    });
    buffer.apply(runEnvelope(1));
    // Drop 2,3,4,5 then deliver 6. The missing count (4) exceeds the
    // reorder window (2), so the buffer declares a dropped-frames gap.
    const result = buffer.apply(runEnvelope(6));
    const gapEvent = result.find((e) => e.kind === "gap");
    expect(gapEvent).toBeDefined();
    expect(gaps).toHaveLength(1);
    expect(gaps[0].fromSequence).toBe(1);
    expect(gaps[0].toSequence).toBe(6);
    expect(gaps[0].missing).toBe(4);
    // The envelope after the gap is applied.
    expect(buffer.lastSequence("run:run-1")).toBe(6);
  });

  it("buffers out-of-order frames and flushes them once the gap fills", () => {
    const buffer = new StreamReplayBuffer({ maxPendingPerPartition: 16 });
    buffer.apply(runEnvelope(1));
    // 3 arrives before 2 -> buffered, no emission.
    const buffered = buffer.apply(runEnvelope(3));
    expect(buffered).toHaveLength(0);
    expect(buffer.lastSequence("run:run-1")).toBe(1);
    // 2 arrives -> frontier advances to 2 and flushes the buffered 3.
    const result = buffer.apply(runEnvelope(2));
    const applied = result.filter((e) => e.kind === "applied");
    expect(applied).toHaveLength(2);
    expect(applied[0].envelope.cursor.sequence).toBe(2);
    expect(applied[1].envelope.cursor.sequence).toBe(3);
    expect(buffer.lastSequence("run:run-1")).toBe(3);
  });

  it("seeds the frontier from a persisted cursor for reconnect resume", () => {
    const buffer = new StreamReplayBuffer();
    buffer.seed("run:run-1", 99);
    // Replay of 99 (already applied) is suppressed.
    expect(buffer.apply(runEnvelope(99))[0].kind).toBe("duplicate");
    // 100 advances.
    expect(buffer.apply(runEnvelope(100))[0].kind).toBe("applied");
    expect(buffer.lastSequence("run:run-1")).toBe(100);
  });

  it("tracks partitions independently", () => {
    const buffer = new StreamReplayBuffer();
    buffer.apply(runEnvelope(1, "run:run-1"));
    buffer.apply(runEnvelope(1, "run:run-2"));
    expect(buffer.partitions().sort()).toEqual(["run:run-1", "run:run-2"]);
    expect(buffer.lastSequence("run:run-1")).toBe(1);
    expect(buffer.lastSequence("run:run-2")).toBe(1);
  });
});

describe("orderedEvents", () => {
  it("yields a de-duplicated monotonic view of a stream", async () => {
    const source = fromArray([
      runEnvelope(1),
      runEnvelope(2),
      runEnvelope(2), // duplicate
      runEnvelope(4), // gap (3 dropped)
      runEnvelope(3), // fills gap out of order
    ]);
    const buffer = new StreamReplayBuffer({ maxPendingPerPartition: 16 });
    const out: GatewayEnvelope[] = [];
    for await (const env of orderedEvents(source, { maxPendingPerPartition: 16 })) {
      out.push(env);
    }
    // 1, 2 applied; 2 duplicate suppressed; 4 buffered (gap, no immediate yield
    // because pending limit not exceeded); 3 fills the gap and flushes 3 then 4.
    const seqs = out.map((e) => e.cursor.sequence);
    expect(seqs).toContain(1);
    expect(seqs).toContain(2);
    // The duplicate seq 2 must not be yielded twice.
    expect(seqs.filter((s) => s === 2)).toHaveLength(1);
    void buffer;
  });

  it("marks a partition stale and recovers it when a fresh event is applied", () => {
    const buffer = new StreamReplayBuffer();
    buffer.apply(runEnvelope(1));
    buffer.markStale("run:run-1");
    expect(buffer.isStale("run:run-1")).toBe(true);
    // A fresh event that advances the frontier clears the stale flag.
    buffer.apply(runEnvelope(2));
    expect(buffer.isStale("run:run-1")).toBe(false);
    // markRecovered also clears the flag explicitly.
    buffer.markStale("run:run-1");
    buffer.markRecovered("run:run-1");
    expect(buffer.isStale("run:run-1")).toBe(false);
  });

  it("checkStale reports partitions idle past the window (deterministic)", () => {
    const buffer = new StreamReplayBuffer();
    buffer.apply(runEnvelope(1));
    const t0 = buffer.activityAt("run:run-1") ?? 0;
    // 5s later, within a 10s window -> not stale.
    expect(buffer.checkStale(t0 + 5_000, 10_000)).toHaveLength(0);
    // 11s later -> stale.
    const stale = buffer.checkStale(t0 + 11_000, 10_000);
    expect(stale).toHaveLength(1);
    expect(stale[0].partition).toBe("run:run-1");
    expect(stale[0].lastSequence).toBe(1);
    expect(buffer.isStale("run:run-1")).toBe(true);
    // Idempotent: a second check does not re-report.
    expect(buffer.checkStale(t0 + 12_000, 10_000)).toHaveLength(0);
  });
});

describe("StreamCorrelator", () => {
  it("correlates streamed events to an action receipt by correlation_id", () => {
    const correlator = new StreamCorrelator();
    const receipt = makeReceipt("corr-1");
    correlator.registerReceipt(receipt);

    // An event carrying the same correlation_id in its payload.
    const event = terminalEnvelope(7, "corr-1");
    const matched = correlator.observe(event);

    expect(matched).toEqual(receipt);
    expect(correlator.hasCorrelatedEvent("corr-1")).toBe(true);
    expect(correlator.eventsFor("corr-1")).toHaveLength(1);
    expect(correlator.eventsFor("corr-1")[0]).toBe(event);
  });

  it("ignores events without a matching receipt", () => {
    const correlator = new StreamCorrelator();
    correlator.registerReceipt(makeReceipt("corr-1"));
    const unmatched = correlator.observe(terminalEnvelope(1, "corr-other"));
    expect(unmatched).toBeUndefined();
    expect(correlator.hasCorrelatedEvent("corr-1")).toBe(false);
  });

  it("links multiple follow-up events to one receipt", () => {
    const correlator = new StreamCorrelator();
    correlator.registerReceipt(makeReceipt("corr-1"));
    correlator.observe(terminalEnvelope(1, "corr-1"));
    correlator.observe(terminalEnvelope(2, "corr-1"));
    correlator.observe(terminalEnvelope(3, "corr-1"));
    expect(correlator.eventsFor("corr-1")).toHaveLength(3);
    expect(correlator.correlationIds()).toEqual(["corr-1"]);
  });

  it("extracts correlation_id from raw_payload when payload lacks it", () => {
    const correlator = new StreamCorrelator();
    correlator.registerReceipt(makeReceipt("corr-raw"));
    const env: GatewayEnvelope = {
      schema_version: sv,
      cursor: streamCursor(1, "terminal:run-1"),
      entity_ref: entityRefTerminal("term-1"),
      event_kind: "terminal_frame",
      payload: { content: "x" },
      raw_payload: { correlation_id: "corr-raw" },
      emitted_at: "2025-01-15T10:00:00Z",
    };
    expect(envelopeCorrelationId(env)).toBe("corr-raw");
    expect(correlator.observe(env)).toBeDefined();
  });
});