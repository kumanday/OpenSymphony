/**
 * Client-side stream replay and ordering primitives.
 *
 * The gateway guarantees monotonic event sequences per stream partition, but
 * any selected remote transport (SSE or WebSocket) can still deliver
 * duplicated events after reconnect, drop frames in flight, or briefly present
 * stale stream state to a reconnected client. These primitives make that
 * contract enforceable on the client without coupling to a specific transport.
 *
 * The replay engine is transport-agnostic: it wraps any `AsyncIterable` of
 * `GatewayEnvelope` and yields a de-duplicated, monotonic, gap-aware view that
 * a reducer can apply safely. Cursor positions exposed here are the values a
 * transport should resume from after a reconnect.
 */

import type {
  GatewayEnvelope,
  StreamCursor,
  ActionReceipt,
} from "@opensymphony/gateway-schema";

/** A gap (dropped frame) detected while ordering a partition. */
export interface StreamGap {
  partition: string;
  /** Sequence of the last applied envelope before the gap. */
  fromSequence: number;
  /** Sequence of the first envelope observed after the gap. */
  toSequence: number;
  /** Number of missing sequence numbers. */
  missing: number;
  detectedAt: number;
}

/** A duplicate envelope that was suppressed during ordering. */
export interface StreamDuplicate {
  partition: string;
  sequence: number;
  suppressedAt: number;
}

/** Replay/ordering outcome for a single applied envelope. */
export type ReplayEvent =
  | { kind: "applied"; envelope: GatewayEnvelope }
  | { kind: "duplicate"; duplicate: StreamDuplicate }
  | { kind: "gap"; gap: StreamGap; envelope: GatewayEnvelope };

export interface StreamReplayBufferOptions {
  /**
   * Maximum number of pending out-of-order envelopes to buffer per partition
   * before declaring a gap and applying the next available sequence.
   */
  maxPendingPerPartition?: number;
  /**
   * Maximum number of partitions to track before the oldest inactive partition
   * is evicted. Prevents unbounded memory growth in long-running clients that
   * observe many runs/terminal sessions. Defaults to 1024. Use
   * `dropPartition()` to retire a partition explicitly when a stream ends.
   */
  maxPartitions?: number;
  /**
   * Clock function used for activity tracking, stale detection, and duplicate/
   * gap timestamps. Defaults to `Date.now`. Supply a deterministic clock in
   * tests so `apply`/`checkStale`/`orderedEvents` behavior is reproducible.
   */
  now?: () => number;
  /** Called whenever a gap is detected. */
  onGap?: (gap: StreamGap) => void;
  /** Called whenever a duplicate is suppressed. */
  onDuplicate?: (duplicate: StreamDuplicate) => void;
}

/**
 * Per-partition monotonic sequence tracker.
 *
 * Tracks the highest applied sequence per partition, suppresses duplicates,
 * and detects gaps. Envelopes are only released when they advance the
 * monotonic frontier (last + 1) or fill a buffered gap.
 */
export class StreamReplayBuffer {
  private readonly lastApplied = new Map<string, number>();
  private readonly pending = new Map<string, Map<number, GatewayEnvelope>>();
  private readonly lastActivityAt = new Map<string, number>();
  /** partition -> timestamp it was marked stale, for accurate idle reporting. */
  private readonly stalePartitions = new Map<string, number>();
  private readonly maxPendingPerPartition: number;
  private readonly maxPartitions: number;
  private readonly now: () => number;
  private readonly onGap?: (gap: StreamGap) => void;
  private readonly onDuplicate?: (duplicate: StreamDuplicate) => void;

  constructor(options: StreamReplayBufferOptions = {}) {
    this.maxPendingPerPartition = options.maxPendingPerPartition ?? 64;
    this.maxPartitions = options.maxPartitions ?? 1024;
    this.now = options.now ?? Date.now;
    this.onGap = options.onGap;
    this.onDuplicate = options.onDuplicate;
  }

  /** Seed the frontier from a previously persisted cursor (reconnect resume). */
  seed(partition: string, sequence: number): void {
    const current = this.lastApplied.get(partition);
    if (current === undefined || sequence > current) {
      this.maybeEvict(partition);
      this.lastApplied.set(partition, sequence);
    }
  }

  /** Last applied sequence for a partition (the resume point). */
  lastSequence(partition: string): number | undefined {
    return this.lastApplied.get(partition);
  }

  /** Cursor to resume a partition from after a disconnect. */
  nextCursor(partition: string): StreamCursor | undefined {
    const seq = this.lastApplied.get(partition);
    if (seq === undefined) return undefined;
    return { sequence: seq, partition };
  }

  /** Whether an envelope would be a duplicate (already applied or stale). */
  isDuplicate(envelope: GatewayEnvelope): boolean {
    const partition = envelope.cursor.partition;
    const seq = envelope.cursor.sequence;
    const last = this.lastApplied.get(partition);
    return last !== undefined && seq <= last;
  }

  /**
   * Apply an envelope, returning the events that should be emitted to the
   * consumer. Duplicate and out-of-order envelopes are suppressed; gaps are
   * reported and the frontier advances to close them.
   *
   * Reordering window: an envelope whose sequence is ahead of the frontier
   * by more than one is buffered when the gap fits within
   * `maxPendingPerPartition` (transient reorder). A gap larger than the
   * window is treated as dropped frames: a gap is reported and the frontier
   * advances so the stream does not stall.
   */
  apply(envelope: GatewayEnvelope): ReplayEvent[] {
    const partition = envelope.cursor.partition;
    const seq = envelope.cursor.sequence;
    const last = this.lastApplied.get(partition);

    if (last !== undefined && seq <= last) {
      const duplicate: StreamDuplicate = {
        partition,
        sequence: seq,
        suppressedAt: this.now(),
      };
      this.onDuplicate?.(duplicate);
      return [{ kind: "duplicate", duplicate }];
    }

    if (last === undefined || seq === last + 1) {
      if (last === undefined) this.maybeEvict(partition);
      this.lastApplied.set(partition, seq);
      this.touchActivity(partition);
      return [{ kind: "applied", envelope }, ...this.flushPending(partition)];
    }

    // Out of order and ahead of the frontier.
    const missing = seq - last - 1;
    if (missing > this.maxPendingPerPartition) {
      // Unbridgeable gap: dropped frames. Declare the gap and advance.
      return this.declareGapAndApply(envelope);
    }

    // Within the reorder window: buffer for later.
    this.pendingFor(partition).set(seq, envelope);
    return [];
  }

  /** Advance the frontier across a declared gap and apply the envelope. */
  private declareGapAndApply(envelope: GatewayEnvelope): ReplayEvent[] {
    const partition = envelope.cursor.partition;
    const last = this.lastApplied.get(partition) ?? 0;
    const gap: StreamGap = {
      partition,
      fromSequence: last,
      toSequence: envelope.cursor.sequence,
      missing: envelope.cursor.sequence - last - 1,
      detectedAt: this.now(),
    };
    this.onGap?.(gap);
    // Advance the frontier to the gap-triggering envelope and apply it.
    this.lastApplied.set(partition, envelope.cursor.sequence);
    this.touchActivity(partition);
    // Drop any buffered pending frames that now fall at or below the new
    // frontier; the gap has skipped past them and they would otherwise be
    // stranded in the pending map forever (never reachable by flushPending).
    this.dropPendingBelowFrontier(partition);
    return [
      { kind: "gap", gap, envelope },
      ...this.flushPending(partition),
    ];
  }

  /** Remove buffered pending frames at or below the current frontier. */
  private dropPendingBelowFrontier(partition: string): void {
    const pending = this.pending.get(partition);
    if (!pending) return;
    const frontier = this.lastApplied.get(partition) ?? 0;
    for (const seq of pending.keys()) {
      if (seq <= frontier) pending.delete(seq);
    }
    if (pending.size === 0) this.pending.delete(partition);
  }

  /** Apply buffered envelopes that now connect to the frontier. */
  private flushPending(partition: string): ReplayEvent[] {
    const pending = this.pending.get(partition);
    if (!pending || pending.size === 0) return [];
    const events: ReplayEvent[] = [];
    let next = (this.lastApplied.get(partition) ?? 0) + 1;
    while (pending.has(next)) {
      const env = pending.get(next)!;
      pending.delete(next);
      this.lastApplied.set(partition, next);
      this.touchActivity(partition);
      events.push({ kind: "applied", envelope: env });
      next++;
    }
    if (pending.size === 0) {
      this.pending.delete(partition);
    }
    return events;
  }

  private pendingFor(partition: string): Map<number, GatewayEnvelope> {
    let pending = this.pending.get(partition);
    if (!pending) {
      pending = new Map();
      this.pending.set(partition, pending);
    }
    return pending;
  }

  private touchActivity(partition: string): void {
    this.lastActivityAt.set(partition, this.now());
    if (this.stalePartitions.has(partition)) {
      this.stalePartitions.delete(partition);
    }
  }

  /**
   * Evict the oldest partition when a new one would exceed `maxPartitions`.
   * Prefer partitions with no buffered pending frames; if every tracked
   * partition still has pending frames, fall back to evicting the oldest
   * regardless of pending state so `maxPartitions` is always enforced as a
   * hard cap (accepting that some out-of-order frames for that partition may
   * be dropped). The new partition itself is never evicted.
   */
  private maybeEvict(newPartition: string): void {
    if (this.lastApplied.size < this.maxPartitions) return;
    // First pass: prefer the oldest partition with no pending frames.
    let oldest: string | undefined;
    let oldestAt = Infinity;
    for (const partition of this.lastApplied.keys()) {
      if (partition === newPartition) continue;
      if (this.pending.has(partition)) continue;
      const at = this.lastActivityAt.get(partition) ?? 0;
      if (at < oldestAt) {
        oldestAt = at;
        oldest = partition;
      }
    }
    // Fallback: every partition has pending frames; evict the oldest anyway so
    // the cap holds.
    if (oldest === undefined) {
      for (const partition of this.lastApplied.keys()) {
        if (partition === newPartition) continue;
        const at = this.lastActivityAt.get(partition) ?? 0;
        if (at < oldestAt) {
          oldestAt = at;
          oldest = partition;
        }
      }
    }
    if (oldest !== undefined) {
      this.dropPartition(oldest);
    }
  }

  /**
   * Drop all state for a partition. Call this when a stream ends (terminal
   * session closed, run completed) to retire the partition and bound memory.
   */
  dropPartition(partition: string): void {
    this.lastApplied.delete(partition);
    this.pending.delete(partition);
    this.lastActivityAt.delete(partition);
    this.stalePartitions.delete(partition);
  }

  /** Last activity timestamp (ms) for a partition, or undefined if never active. */
  activityAt(partition: string): number | undefined {
    return this.lastActivityAt.get(partition);
  }

  /** Whether a partition is currently marked stale. */
  isStale(partition: string): boolean {
    return this.stalePartitions.has(partition);
  }

  /** Timestamp a partition was marked stale, or undefined if not stale. */
  staleSince(partition: string): number | undefined {
    return this.stalePartitions.get(partition);
  }

  /**
   * Mark a partition stale (for example after a disconnect before replay).
   * Defaults to the buffer's configured clock (`options.now`, or `Date.now`)
   * so behavior stays deterministic when a clock is supplied.
   */
  markStale(partition: string, now: number = this.now()): void {
    this.stalePartitions.set(partition, now);
  }

  /** Mark a partition recovered after a fresh event or successful replay. */
  markRecovered(partition: string): void {
    this.stalePartitions.delete(partition);
  }

  /**
   * Check which tracked partitions have been idle longer than `staleAfterMs`.
   * Returns stale info for each newly-stale partition (idempotent: a partition
   * already marked stale is not reported again). Deterministic over the
   * supplied `now` so it can be tested without real timers.
   */
  checkStale(now: number, staleAfterMs: number): StreamStaleInfo[] {
    const newlyStale: StreamStaleInfo[] = [];
    for (const partition of this.lastActivityAt.keys()) {
      if (this.stalePartitions.has(partition)) continue;
      const lastAt = this.lastActivityAt.get(partition) ?? now;
      const idleMs = now - lastAt;
      if (idleMs >= staleAfterMs) {
        this.stalePartitions.set(partition, now);
        newlyStale.push({
          partition,
          lastSequence: this.lastApplied.get(partition) ?? null,
          staleAt: now,
          idleMs,
        });
      }
    }
    return newlyStale;
  }

  /** All partitions currently tracked. */
  partitions(): string[] {
    return [...new Set([...this.lastApplied.keys(), ...this.pending.keys()])];
  }
}

export interface OrderedEventsOptions extends StreamReplayBufferOptions {
  /**
   * Idle window in ms after which the stream is considered stale. A `stale`
   * signal is emitted once per stale episode when no envelope is applied.
   */
  staleAfterMs?: number;
  /**
   * Clock function used for stale detection. Defaults to `Date.now`. Supply a
   * deterministic clock in tests so stale/recovered reports are reproducible.
   */
  now?: () => number;
  /** Called when the stream goes stale. */
  onStale?: (info: StreamStaleInfo) => void;
  /** Called when the stream recovers from stale after receiving an event. */
  onRecovered?: (info: StreamStaleInfo) => void;
}

export interface StreamStaleInfo {
  partition: string | null;
  lastSequence: number | null;
  staleAt: number;
  idleMs: number;
}

/**
 * Wrap a source async iterable with replay/ordering semantics.
 *
 * Yields envelopes in monotonic, de-duplicated order per partition. Gaps and
 * duplicates are reported via callbacks but do not break iteration; the
 * consumer only receives envelopes that advanced the frontier. Stale
 * partitions (idle longer than `staleAfterMs`) are reported via `onStale` and
 * `onRecovered`.
 *
 * Staleness is driven by `options.now` (default `Date.now`). For fully
 * deterministic stale/recovered behavior in tests, supply a `now` clock; the
 * buffer's `checkStale(now, ...)` API is the deterministic low-level surface.
 */
export async function* orderedEvents(
  source: AsyncIterable<GatewayEnvelope>,
  options: OrderedEventsOptions = {},
): AsyncGenerator<GatewayEnvelope> {
  const buffer = new StreamReplayBuffer(options);
  const staleAfterMs = options.staleAfterMs ?? 30_000;
  const now = options.now ?? Date.now;

  for await (const envelope of source) {
    const partition = envelope.cursor.partition;
    const wasStale = staleAfterMs > 0 && buffer.isStale(partition);
    const staleAt = wasStale ? buffer.staleSince(partition) ?? now() : 0;

    for (const event of buffer.apply(envelope)) {
      if (event.kind === "applied") {
        if (wasStale) {
          const recoveredAt = now();
          options.onRecovered?.({
            partition,
            lastSequence: event.envelope.cursor.sequence,
            staleAt,
            idleMs: recoveredAt - staleAt,
          });
        }
        yield event.envelope;
      } else if (event.kind === "gap") {
        // The envelope that triggered the gap is applied as part of the gap;
        // emit it so the consumer sees the post-gap frontier advance.
        yield event.envelope;
      }
      // duplicates are intentionally not yielded
    }

    // Stale detection: report partitions idle longer than the window. Driven
    // by `options.now` (default wall-clock); supply a deterministic clock for
    // reproducible stale reports, or use buffer.checkStale(now, ...) directly.
    if (staleAfterMs > 0) {
      for (const info of buffer.checkStale(now(), staleAfterMs)) {
        options.onStale?.(info);
      }
    }
  }
}

/**
 * Extract a correlation id from an envelope payload.
 *
 * Action receipts and streamed events share a `correlation_id`. Envelopes
 * carry it inside `payload` or `raw_payload` (for example a `TerminalFrame`),
 * so the correlator inspects both.
 */
export function envelopeCorrelationId(envelope: GatewayEnvelope): string | undefined {
  const candidates = [envelope.payload, envelope.raw_payload];
  for (const candidate of candidates) {
    const id = extractCorrelationId(candidate);
    if (id) return id;
  }
  return undefined;
}

function extractCorrelationId(value: unknown): string | undefined {
  if (typeof value !== "object" || value === null) return undefined;
  const record = value as Record<string, unknown>;
  if (typeof record.correlation_id === "string") return record.correlation_id;
  // Some payloads nest correlation under a `frame`/`event` wrapper.
  if (typeof record.frame === "object" && record.frame !== null) {
    const nested = (record.frame as Record<string, unknown>).correlation_id;
    if (typeof nested === "string") return nested;
  }
  return undefined;
}

/**
 * Correlates dispatched action receipts with streamed events.
 *
 * Callers register a receipt (from `dispatchAction`) and then observe the
 * event stream; the correlator links events whose payload carries the same
 * `correlation_id` back to the originating receipt and its expected follow-up
 * events.
 *
 * Memory is bounded: each correlation tracks at most `maxEventsPerCorrelation`
 * observed events (oldest dropped), and `forget()` retires a completed
 * correlation entirely. Callers should `forget(correlationId)` once the
 * receipt's expected follow-up events have all arrived (or the run ends).
 */
export class StreamCorrelator {
  private readonly receiptsByCorrelation = new Map<
    string,
    { receipt: ActionReceipt; events: GatewayEnvelope[] }
  >();
  private readonly maxEventsPerCorrelation: number;

  constructor(options: { maxEventsPerCorrelation?: number } = {}) {
    this.maxEventsPerCorrelation = options.maxEventsPerCorrelation ?? 256;
  }

  /** Register a receipt returned from an action dispatch. */
  registerReceipt(receipt: ActionReceipt): void {
    if (!this.receiptsByCorrelation.has(receipt.correlation_id)) {
      this.receiptsByCorrelation.set(receipt.correlation_id, {
        receipt,
        events: [],
      });
    } else {
      this.receiptsByCorrelation.get(receipt.correlation_id)!.receipt = receipt;
    }
  }

  /** Observe an envelope and link it to any registered receipt. */
  observe(envelope: GatewayEnvelope): ActionReceipt | undefined {
    const correlationId = envelopeCorrelationId(envelope);
    if (!correlationId) return undefined;
    const entry = this.receiptsByCorrelation.get(correlationId);
    if (!entry) return undefined;
    entry.events.push(envelope);
    if (entry.events.length > this.maxEventsPerCorrelation) {
      entry.events.shift();
    }
    return entry.receipt;
  }

  /** All envelopes observed for a correlation id (the follow-up stream). */
  eventsFor(correlationId: string): GatewayEnvelope[] {
    return this.receiptsByCorrelation.get(correlationId)?.events ?? [];
  }

  /** The receipt registered for a correlation id, if any. */
  receiptFor(correlationId: string): ActionReceipt | undefined {
    return this.receiptsByCorrelation.get(correlationId)?.receipt;
  }

  /** Whether a receipt has at least one correlated streamed event. */
  hasCorrelatedEvent(correlationId: string): boolean {
    return (this.receiptsByCorrelation.get(correlationId)?.events.length ?? 0) > 0;
  }

  /** All correlation ids currently tracked. */
  correlationIds(): string[] {
    return [...this.receiptsByCorrelation.keys()];
  }

  /**
   * Retire a completed correlation, dropping its receipt and observed events.
   * Call this once the receipt's expected follow-up events have arrived or the
   * run has ended, to bound memory in long-running sessions.
   */
  forget(correlationId: string): void {
    this.receiptsByCorrelation.delete(correlationId);
  }

  /** Number of correlations currently tracked. */
  size(): number {
    return this.receiptsByCorrelation.size;
  }
}