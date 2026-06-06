# OSYM-781: Long-Running Run Observability Fixtures and Client-Facing Diagnostics

## Summary

Make long-running OpenHands work inspectable through fixtures, client state, runtime timelines, diagnostics, and operations documentation.

## Deliverables

### Schema Extensions (Gateway + TS)

The following types were added to both Rust (`opensymphony-gateway-schema`) and TypeScript (`packages/gateway-schema`):

- `RunPhase` — Operational phase observed by the client: `active`, `quiet`, `degraded`, `stalled`, `retry_queued`, `cancelled`, `detached`, `completed`.
- `RunStreamLiveness` — Stream-level liveness classification: `healthy`, `stale`, `dead`.
- `RunProgress` — Compact progress event for replay.
- `RunLivenessEnvelope` — Snapshot of current run liveness surface (phase, stream, latest progress).
- `HarnessSchedulerDisagreement` — Details when scheduler says retry-queued but harness is still running.
- `RunDiagnostics` — Diagnostic hints for subsystem disagreement.
- `SafeActions` — Actions the client may safely invoke given current state.
- `RunDetail` extended with `liveness`, `diagnostics`, and `safe_actions` fields.

### State Management

- `RunLivenessState` interface added to TypeScript state package.
- `LIVENESS_UPDATE`, `LIVENESS_STALL`, `LIVENESS_RECONNECT` actions added.
- `computeSafeActions()` helper computes the safe action set based on phase, stream, and session status.

### Fixture Tests

- `run_phase_roundtrips` — Verifies all 8 phases (including `completed`) serialize/deserialize correctly.
- `run_stream_liveness_roundtrips` — Verifies all 3 stream statuses.
- `run_progress_roundtrips` — Verifies progress event serialization.
- `run_liveness_envelope_roundtrips` — Verifies liveness envelope.
- `safe_actions_roundtrips` — Verifies safe action serialization.
- `safe_actions_defaults_to_all_false` — Verifies Default impl.
- `harness_scheduler_disagreement_roundtrips` — Verifies diagnostic type.
- Updated `run_detail_roundtrips` to include new fields.

All 49 tests pass.

## Operations Guide: Harness/Scheduler Disagreement

### Symptoms

1. Dashboard shows `scheduler_status: "retry_queued"` but `harness_status: "running"` or `"running_turn"`.
2. Run detail shows `phase: "stalled"` with `stream: "stale"`.
3. Safe actions indicate `retry: true`, `cancel: true`, `rehydrate: true`.

### Diagnosis Steps

1. Check `RunDiagnostics.harness_scheduler_disagreement` in the run detail response.
2. Verify `detected_at` timestamp to understand staleness.
3. Check `resolution_path` field for recommended action.
4. Inspect stream health via `RunLivenessEnvelope.stream`:
   - `healthy`: Harness is actively producing events.
   - `stale`: No recent events but harness session may still be alive.
   - `dead`: Harness session is terminated or unreachable.

### Expected Resolution Path

1. **If harness is still running** (phase: `stalled`, stream: `stale`):
   - Use `cancel` safe action to terminate the lingering harness session.
   - After cancel completes, use `retry` to requeue the run.
2. **If harness is dead** (phase: `detached`, stream: `dead`):
   - Use `rehydrate` to attempt session recovery.
   - If rehydration fails, use `retry` to start fresh.
3. **If scheduler says retry_queued but harness shows active** (phase: `active` or `quiet`):
   - Wait for harness to complete current turn.
   - If no progress after stall timeout, cancel and retry.

### Safe Action Matrix

| Phase | Stream | retry | cancel | rehydrate | detach |
|-------|--------|-------|--------|-----------|--------|
| active | healthy | false | true | false | false |
| active | stale | false | true | true | false |
| active | dead | false | false | false | true |
| quiet | stale | false | true | true | false |
| degraded | stale | false | true | true | false |
| stalled | stale | true | true | true | false |
| retry_queued | any | true | false | false | false |
| cancelled | any | true | false | false | false |
| detached | dead | true | false | true | false |
| completed | dead | true | false | true | false |

## Test Plan

- [x] Reducer tests for long-running active, quiet, degraded, stalled, and detached states (liveness reducer tests + computeSafeActions parameterized tests in `packages/state/__tests__/reducer.test.ts`).
- [x] Gateway schema roundtrip tests for all new types (7 new tests, 49 total passing in `crates/opensymphony-gateway-schema/tests/gateway_schema.rs`).
- [x] Operations documentation covers harness/scheduler disagreement diagnosis and recovery.
- [ ] Timeline fixture tests for waiting-on-prior-turn and stall-probe sequences (deferred: requires event-stream replay fixtures).
- [ ] UI fixture tests for retry queue with and without active underlying harness session (deferred: requires UI component test harness).
