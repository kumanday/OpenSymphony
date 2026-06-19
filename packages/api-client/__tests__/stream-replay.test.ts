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
    const gaps: number[] = [];
    const duplicates: number[] = [];
    const out: GatewayEnvelope[] = [];
    for await (const env of orderedEvents(source, {
      maxPendingPerPartition: 16,
      onGap: (g) => gaps.push(g.fromSequence),
      onDuplicate: (d) => duplicates.push(d.sequence),
    })) {
      out.push(env);
    }
    // 1, 2 applied; 2 duplicate suppressed; 4 buffered (gap fits the reorder
    // window so no gap is declared yet); 3 fills the gap and flushes 3 then 4.
    const seqs = out.map((e) => e.cursor.sequence);
    // Full monotonic sequence, de-duplicated.
    expect(seqs).toEqual([1, 2, 3, 4]);
    // The duplicate seq 2 must not be yielded twice.
    expect(seqs.filter((s) => s === 2)).toHaveLength(1);
    // The duplicate was reported.
    expect(duplicates).toEqual([2]);
    // No unbridgeable gap was declared (the 3->4 reorder fit the window).
    expect(gaps).toEqual([]);
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

  it("reports accurate staleAt and idleMs on recovery via orderedEvents", async () => {
    // Drive orderedEvents with a deterministic clock so the stale->recovered
    // transition reports the captured staleAt (not Date.now) and a non-zero
    // idleMs = recoveredAt - staleAt.
    //
    // now() is called, in order, for: touchActivity(seq1), checkStale(seq1),
    // touchActivity(seq2) [clears stale], recoveredAt, checkStale(seq2).
    const ticks = [1000, 61_000, 61_000, 65_000, 65_000];
    let tickIndex = 0;
    const now = () => ticks[Math.min(tickIndex++, ticks.length - 1)];

    const staleAts: number[] = [];
    const recovered: { staleAt: number; idleMs: number; seq: number }[] = [];
    const source = fromArray([runEnvelope(1), runEnvelope(2)]);

    for await (const env of orderedEvents(source, {
      staleAfterMs: 30_000,
      now,
      onStale: (info) => staleAts.push(info.staleAt),
      onRecovered: (info) =>
        recovered.push({ staleAt: info.staleAt, idleMs: info.idleMs, seq: info.lastSequence ?? -1 }),
    })) {
      void env;
    }

    // checkStale(seq1) at 61000 sees lastActivityAt=1000 => idle 60s => stale.
    expect(staleAts).toEqual([61_000]);
    // seq2 recovers: staleAt is the captured mark (61000), recoveredAt=65000
    // => idleMs=4000, and lastSequence is the recovering event's sequence.
    expect(recovered).toHaveLength(1);
    expect(recovered[0].staleAt).toBe(61_000);
    expect(recovered[0].idleMs).toBe(4_000);
    expect(recovered[0].seq).toBe(2);
  });

  it("evicts the oldest inactive partition when exceeding maxPartitions", () => {
    const buffer = new StreamReplayBuffer({ maxPartitions: 2 });
    buffer.apply(runEnvelope(1, "run:run-1"));
    buffer.apply(runEnvelope(1, "run:run-2"));
    expect(buffer.partitions()).toHaveLength(2);
    // Adding a third evicts the oldest inactive partition (run-1).
    buffer.apply(runEnvelope(1, "run:run-3"));
    expect(buffer.partitions()).toHaveLength(2);
    expect(buffer.lastSequence("run:run-1")).toBeUndefined();
    expect(buffer.lastSequence("run:run-2")).toBe(1);
    expect(buffer.lastSequence("run:run-3")).toBe(1);
  });

  it("dropPartition retires a partition to bound memory", () => {
    const buffer = new StreamReplayBuffer();
    buffer.apply(runEnvelope(1, "run:run-1"));
    buffer.markStale("run:run-1");
    expect(buffer.partitions()).toContain("run:run-1");
    buffer.dropPartition("run:run-1");
    expect(buffer.lastSequence("run:run-1")).toBeUndefined();
    expect(buffer.isStale("run:run-1")).toBe(false);
    expect(buffer.partitions()).not.toContain("run:run-1");
  });

  it("drops stranded pending frames when a gap advances past them", () => {
    // last=1, buffer out-of-order {3,4} (within the reorder window), then a
    // far-ahead seq 6 declares a gap. The stranded {3,4} must be cleared, not
    // left in pending forever.
    const buffer = new StreamReplayBuffer({ maxPendingPerPartition: 2 });
    buffer.apply(runEnvelope(1)); // frontier -> 1
    buffer.apply(runEnvelope(3)); // buffered (missing=1 <= 2)
    buffer.apply(runEnvelope(4)); // buffered (missing=2 <= 2)
    expect(buffer.lastSequence("run:run-1")).toBe(1);
    // seq 6 jumps past the reorder window (missing=4 > 2): gap, frontier -> 6.
    const result = buffer.apply(runEnvelope(6));
    expect(result[0].kind).toBe("gap");
    expect(buffer.lastSequence("run:run-1")).toBe(6);
    // A subsequent seq 7 must apply cleanly (no stranded 3/4 re-surfacing).
    const r7 = buffer.apply(runEnvelope(7));
    expect(r7.map((e) => e.kind)).toContain("applied");
    expect(buffer.lastSequence("run:run-1")).toBe(7);
    // A late duplicate seq 3 arriving after the gap is suppressed, proving it
    // is no longer stranded/reachable as a fresh frame.
    const r3 = buffer.apply(runEnvelope(3));
    expect(r3.map((e) => e.kind)).toContain("duplicate");
  });

  it("evicts the oldest partition even when all partitions have pending frames", () => {
    // Every partition has buffered out-of-order frames, so the no-pending
    // preference finds nothing; the cap must still hold via the fallback.
    const buffer = new StreamReplayBuffer({ maxPartitions: 2, maxPendingPerPartition: 8 });
    buffer.apply(runEnvelope(1, "run:run-1"));
    buffer.apply(runEnvelope(3, "run:run-1")); // pending
    buffer.apply(runEnvelope(1, "run:run-2"));
    buffer.apply(runEnvelope(3, "run:run-2")); // pending
    expect(buffer.partitions()).toHaveLength(2);
    // Adding a third forces eviction of the oldest (run-1) despite its pending.
    buffer.apply(runEnvelope(1, "run:run-3"));
    expect(buffer.partitions()).toHaveLength(2);
    expect(buffer.lastSequence("run:run-1")).toBeUndefined();
  });

  it("markStale uses the configured clock by default (deterministic)", () => {
    let t = 5_000;
    const buffer = new StreamReplayBuffer({ now: () => t });
    buffer.apply(runEnvelope(1, "run:run-1"));
    t = 50_000;
    buffer.markStale("run:run-1"); // no explicit now -> uses injected clock
    expect(buffer.staleSince("run:run-1")).toBe(50_000);
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

  it("caps observed events per correlation to bound memory", () => {
    const correlator = new StreamCorrelator({ maxEventsPerCorrelation: 3 });
    correlator.registerReceipt(makeReceipt("corr-cap"));
    for (let i = 1; i <= 5; i++) {
      correlator.observe(terminalEnvelope(i, "corr-cap"));
    }
    // Only the most recent 3 are retained (oldest dropped).
    expect(correlator.eventsFor("corr-cap")).toHaveLength(3);
    const seqs = correlator.eventsFor("corr-cap").map((e) => e.cursor.sequence);
    expect(seqs).toEqual([3, 4, 5]);
    expect(correlator.size()).toBe(1);
  });

  it("forgets a completed correlation to retire state", () => {
    const correlator = new StreamCorrelator();
    correlator.registerReceipt(makeReceipt("corr-gone"));
    correlator.observe(terminalEnvelope(1, "corr-gone"));
    expect(correlator.hasCorrelatedEvent("corr-gone")).toBe(true);
    expect(correlator.size()).toBe(1);
    correlator.forget("corr-gone");
    expect(correlator.size()).toBe(0);
    expect(correlator.receiptFor("corr-gone")).toBeUndefined();
    expect(correlator.eventsFor("corr-gone")).toEqual([]);
  });
});