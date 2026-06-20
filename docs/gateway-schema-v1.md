# Gateway Schema v1

## Version

`1.0.0`

## Summary

This document defines the public DTO schemas for the OpenSymphony Gateway v1 API.
All schemas live in the `opensymphony-gateway-schema` crate and are designed to
be forward-compatible, versioned, and transport-agnostic.

## Design principles

1. **Version in every payload**: Every DTO carries a `schema_version` field so
   clients can negotiate compatibility.
2. **Cursor-based replay**: Stream items use monotonic `sequence` numbers within
   logical partitions. Clients resume by sending the last sequence they received.
3. **Entity references**: Every event envelope carries an `entity_ref` with a
   stable `id`, human-readable `identifier`, and typed `kind`.
4. **Raw payload preservation**: Unknown or future event kinds keep the original
   JSON in `raw_payload` so the gateway can forward them without loss.
5. **No orchestrator internals leak**: `DashboardSnapshot`, `RunDetail`, and
   `TaskGraphNode` deliberately avoid exposing `IssueExecution`, scheduler
   internals, or private state machines.

## Schema modules

| Module | Purpose | Key types |
|--------|---------|-----------|
| `version` | Semantic versioning | `SchemaVersion`, `GATEWAY_SCHEMA_VERSION` |
| `cursor` | Replay and pagination | `StreamCursor`, `PageCursor` |
| `envelope` | Base event envelope | `GatewayEnvelope`, `EntityRef`, `EntityKind` |
| `snapshot` | Dashboard snapshot | `DashboardSnapshot`, `GatewayHealth`, `GatewayMetrics` |
| `task_graph` | Read-only task graph | `TaskGraphNode`, `TaskGraphSnapshot` |
| `run` | Run detail and events | `RunDetail`, `RunEventPage`, `RunEvent` |
| `terminal` | Terminal/log frames | `TerminalFrame`, `TerminalSnapshot` |
| `approval` | Human-in-the-loop | `ApprovalRequest`, `ActionReceipt` |
| `planning` | Planning artifacts | `PlanningArtifact`, `PlanningSessionSummary` |
| `capability` | Discovery | `GatewayCapabilities`, `TransportCapability`, `HarnessCapability` |
| `action` | Mutation dispatch | `ActionDispatch`, `ActionKind` |
| `transport` | Benchmark metadata | `TransportRecommendation`, `TransportProfile` |

## Harness Capabilities

`GET /api/v1/capabilities` includes a `harnesses` array. Each
`HarnessCapability` describes a public harness kind, transport shape, supported
run actions, event stream behavior, approval support, model-setting support,
cancellation, pause/resume, and history/replay support.

The harness list deliberately uses stable strings such as
`openhands_agent_server`, `codex_app_server`, and `rust_native` rather than
private adapter class names. Clients should use the advertised booleans and
feature gaps instead of special-casing adapter internals.

## Event envelope example

```json
{
  "schema_version": {"major": 1, "minor": 0, "patch": 0},
  "cursor": {"sequence": 42, "partition": "terminal:run-1", "timestamp_anchor": 1700000000},
  "entity_ref": {"kind": "terminal_session", "id": "term-1"},
  "event_kind": "terminal_frame",
  "payload": {"content": "hello"},
  "raw_payload": {"content": "hello"},
  "emitted_at": "2025-08-17T09:12:00Z"
}
```

## Transport profiles

See `docs/stream-benchmark-report.md` for measured throughput, latency, and
reconnect behavior.
