# @opensymphony/gateway-schema

TypeScript types and runtime validators for the OpenSymphony Gateway v1 API.

This package mirrors the Rust `opensymphony-gateway-schema` crate so that
browser and desktop clients can decode payloads with full type safety.

## Schema modules

| Module | Types |
|--------|-------|
| `version` | `SchemaVersion`, `GATEWAY_SCHEMA_VERSION` |
| `cursor` | `StreamCursor`, `PageCursor` |
| `envelope` | `GatewayEnvelope`, `EntityRef`, `EntityKind` |
| `snapshot` | `DashboardSnapshot`, `GatewayHealth`, `GatewayMetrics`, `ProjectSummary`, `SnapshotEventSummary` |
| `task_graph` | `TaskGraphNode`, `TaskGraphSnapshot`, `TaskGraphNodeKind`, `TaskGraphStateCategory` |
| `run` | `RunDetail`, `RunEventPage`, `RunEvent`, `RunStatus`, `ReleaseReason` |
| `terminal` | `TerminalFrame`, `TerminalSnapshot`, `TerminalFrameKind`, `TerminalEncoding` |
| `approval` | `ApprovalRequest`, `ActionReceipt`, `ApprovalKind`, `ApprovalStatus`, `ActionReceiptStatus` |
| `planning` | `PlanningArtifact`, `PlanningSessionSummary`, `PlanningArtifactKind`, `PlanningSessionStatus` |
| `capability` | `GatewayCapabilities`, `TransportCapability`, `FeatureCapability`, `AuthMode` |
| `action` | `ActionDispatch`, `ActionKind`, `ActionTarget` |
| `transport` | `TransportRecommendation`, `TransportProfile` |
| `validation` | Runtime validators for envelopes and schema versions |

## Usage

```typescript
import { parseGatewayEnvelope, GATEWAY_SCHEMA_VERSION } from "@opensymphony/gateway-schema";

const envelope = parseGatewayEnvelope(rawJsonString);
console.log(envelope.event_kind);
console.log(GATEWAY_SCHEMA_VERSION); // "1.0.0"
```

## Update path

These types are hand-maintained to match the Rust gateway-schema crate.
When the Rust crate changes:

1.  Open `crates/opensymphony-gateway-schema/src/` and identify changed types.
2.  Update the matching TypeScript module in `packages/gateway-schema/src/`.
3.  Update or add JSON fixtures in `packages/gateway-schema/__tests__/fixtures/`.
4.  Run `npm test` to verify the fixtures still validate.
5.  Bump the package version and `GATEWAY_SCHEMA_VERSION` if the schema major
    or minor version changes.

### Generating types from Rust

When possible, use `schemars` or `ts-rs` on the Rust types to generate the
TypeScript interfaces automatically. Until then, the manual sync process above
is the documented path.

## Tests

```bash
npm test --workspace=@opensymphony/gateway-schema
```

## License

Same license as the OpenSymphony repository.