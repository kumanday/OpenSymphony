/**
 * Schema fixture tests.
 *
 * Each fixture JSON file is loaded and validated through the runtime
 * validators to ensure the TypeScript types remain compatible with
 * the gateway's actual JSON payloads.
 */

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import {
  GATEWAY_SCHEMA_VERSION,
  assertCompatibleSchemaVersion,
  assertValidEnvelopeBatch,
  assertValidGatewayEnvelope,
  getGatewaySchemaVersion,
  isValidGatewayEnvelope,
  isValidSchemaVersion,
  parseGatewayEnvelope,
  schemaVersionFromString,
  validateEnvelopeBatch,
} from "@opensymphony/gateway-schema";

const fixturesDir = resolve(__dirname, "fixtures");

// Helper to read and parse a fixture file.
function loadFixture(name: string): unknown {
  const content = readFileSync(resolve(fixturesDir, name), "utf-8");
  return JSON.parse(content);
}

// -- Version tests --

describe("schema version", () => {
  test("GATEWAY_SCHEMA_VERSION matches v1", () => {
    expect(GATEWAY_SCHEMA_VERSION).toBe("1.0.0");
    expect(getGatewaySchemaVersion()).toBe("1.0.0");
  });

  test("isValidSchemaVersion accepts valid version", () => {
    expect(isValidSchemaVersion({ major: 1, minor: 0, patch: 0 })).toBe(true);
  });

  test("isValidSchemaVersion rejects invalid shapes", () => {
    expect(isValidSchemaVersion("1.0.0")).toBe(false);
    expect(isValidSchemaVersion({ major: 1 })).toBe(false);
    expect(isValidSchemaVersion(null)).toBe(false);
    expect(isValidSchemaVersion(undefined)).toBe(false);
  });

  test("assertCompatibleSchemaVersion throws on wrong major", () => {
    expect(() => assertCompatibleSchemaVersion({ major: 2, minor: 0, patch: 0 })).toThrow(
      /unsupported schema major=2/,
    );
  });

  test("isValidSchemaVersion rejects NaN values", () => {
    expect(isValidSchemaVersion({ major: NaN, minor: 0, patch: 0 })).toBe(false);
    expect(isValidSchemaVersion({ major: 1, minor: Infinity, patch: 0 })).toBe(false);
  });

  test("schemaVersionFromString parses valid string", () => {
    expect(schemaVersionFromString("1.2.3")).toEqual({ major: 1, minor: 2, patch: 3 });
  });

  test("schemaVersionFromString throws on incomplete string", () => {
    expect(() => schemaVersionFromString("1.2")).toThrow(/Invalid schema version/);
  });

  test("schemaVersionFromString throws on non-numeric string", () => {
    expect(() => schemaVersionFromString("a.b.c")).toThrow(/Invalid schema version/);
  });

  test("schemaVersionFromString throws on extra components", () => {
    expect(() => schemaVersionFromString("1.2.3.4")).toThrow(/Invalid schema version/);
  });
});

// -- Batch validation tests --

describe("batch validation", () => {
  const validEnvelope = loadFixture("envelope_terminal_frame.json");

  test("validateEnvelopeBatch returns empty array for all valid", () => {
    const failed = validateEnvelopeBatch([validEnvelope, validEnvelope]);
    expect(failed).toEqual([]);
  });

  test("validateEnvelopeBatch returns indices of invalid envelopes", () => {
    const batch = [validEnvelope, { bad: true }, validEnvelope, "not an object"];
    const failed = validateEnvelopeBatch(batch);
    expect(failed).toEqual([1, 3]);
  });

  test("validateEnvelopeBatch does not throw", () => {
    const batch = [validEnvelope, { bad: true }];
    expect(() => validateEnvelopeBatch(batch)).not.toThrow();
  });

  test("assertValidEnvelopeBatch throws on failures", () => {
    const batch = [validEnvelope, { bad: true }];
    expect(() => assertValidEnvelopeBatch(batch)).toThrow(/failed validation/);
  });

  test("assertValidEnvelopeBatch passes for all valid", () => {
    const batch = [validEnvelope, validEnvelope];
    expect(() => assertValidEnvelopeBatch(batch)).not.toThrow();
  });
});

// -- Envelope fixture tests --

describe("envelope fixtures", () => {
  test("terminal_frame envelope validates", () => {
    const data = loadFixture("envelope_terminal_frame.json");
    assertValidGatewayEnvelope(data, "envelope_terminal_frame.json");
    expect(isValidGatewayEnvelope(data)).toBe(true);
  });

  test("parseGatewayEnvelope parses terminal frame", () => {
    const raw = readFileSync(resolve(fixturesDir, "envelope_terminal_frame.json"), "utf-8");
    const envelope = parseGatewayEnvelope(raw);
    expect(envelope.event_kind).toBe("terminal_frame");
    expect(envelope.entity_ref.kind).toBe("terminal_session");
  });

  test("unknown event envelope validates (forward compatibility)", () => {
    const data = loadFixture("envelope_unknown_event.json");
    assertValidGatewayEnvelope(data, "envelope_unknown_event.json");
  });

  test("unknown event preserves raw_payload", () => {
    const data = loadFixture("envelope_unknown_event.json");
    assertValidGatewayEnvelope(data);
    const e = data as Record<string, unknown>;
    expect(e.raw_payload).toEqual({ unknown_field: 42 });
  });
});

// -- Dashboard snapshot fixture --

describe("dashboard snapshot fixture", () => {
  test("dashboard_snapshot validates schema version", () => {
    const data = loadFixture("dashboard_snapshot.json");
    const sv = (data as Record<string, unknown>).schema_version;
    expect(isValidSchemaVersion(sv)).toBe(true);
    expect(() => assertCompatibleSchemaVersion(sv)).not.toThrow();
  });

  test("dashboard_snapshot has expected structure", () => {
    const data = loadFixture("dashboard_snapshot.json") as Record<string, unknown>;
    expect(data.health).toBe("healthy");
    expect(data.sequence).toBe(1);
    expect(Array.isArray(data.projects)).toBe(true);
    expect((data.projects as unknown[]).length).toBe(1);
    expect(Array.isArray(data.recent_events)).toBe(true);
  });
});

// -- Task graph node fixture --

describe("task graph node fixture", () => {
  test("task_graph_node validates schema version", () => {
    const data = loadFixture("task_graph_node.json");
    const sv = (data as Record<string, unknown>).schema_version;
    expect(isValidSchemaVersion(sv)).toBe(true);
  });

  test("task_graph_node has expected fields", () => {
    const data = loadFixture("task_graph_node.json") as Record<string, unknown>;
    expect(data.kind).toBe("issue");
    expect(data.identifier).toBe("COE-390");
    expect(data.state_category).toBe("in_progress");
    expect(Array.isArray(data.children)).toBe(true);
    expect(Array.isArray(data.blocked_by)).toBe(true);
    expect(Array.isArray(data.labels)).toBe(true);
  });
});

// -- Run detail fixture --

describe("run detail fixture", () => {
  test("run_detail validates schema version", () => {
    const data = loadFixture("run_detail.json");
    const sv = (data as Record<string, unknown>).schema_version;
    expect(isValidSchemaVersion(sv)).toBe(true);
  });

  test("run_detail has expected fields", () => {
    const data = loadFixture("run_detail.json") as Record<string, unknown>;
    expect(data.status).toBe("running");
    expect(data.issue_identifier).toBe("COE-390");
    expect(data.turn_count).toBe(3);
    expect(data.max_turns).toBe(8);
  });
});

// -- Run event page fixture --

describe("run event page fixture", () => {
  test("run_event_page validates schema version", () => {
    const data = loadFixture("run_event_page.json");
    const sv = (data as Record<string, unknown>).schema_version;
    expect(isValidSchemaVersion(sv)).toBe(true);
  });

  test("run_event_page has events array", () => {
    const data = loadFixture("run_event_page.json") as Record<string, unknown>;
    const events = data.events as Array<Record<string, unknown>>;
    expect(events.length).toBe(1);
    expect(events[0].kind).toBe("ConversationStateUpdateEvent");
    expect(events[0].sequence).toBe(1);
  });
});

// -- Terminal frame fixture --

describe("terminal frame fixture", () => {
  test("terminal_frame validates schema version", () => {
    const data = loadFixture("terminal_frame.json");
    const sv = (data as Record<string, unknown>).schema_version;
    expect(isValidSchemaVersion(sv)).toBe(true);
  });

  test("terminal_frame has expected fields", () => {
    const data = loadFixture("terminal_frame.json") as Record<string, unknown>;
    expect(data.frame_kind).toBe("stdout");
    expect(data.encoding).toBe("utf8");
    expect(data.content).toBe("hello world\n");
  });
});

// -- Approval fixture --

describe("approval request fixture", () => {
  test("approval_request validates schema version", () => {
    const data = loadFixture("approval_request.json");
    const sv = (data as Record<string, unknown>).schema_version;
    expect(isValidSchemaVersion(sv)).toBe(true);
  });

  test("approval_request has expected fields", () => {
    const data = loadFixture("approval_request.json") as Record<string, unknown>;
    expect(data.kind).toBe("tool_use");
    expect(data.status).toBe("pending");
    expect(data.correlation_id).toBe("corr-1");
  });
});

// -- Gateway capabilities fixture --

describe("gateway capabilities fixture", () => {
  test("gateway_capabilities validates schema version", () => {
    const data = loadFixture("gateway_capabilities.json");
    const sv = (data as Record<string, unknown>).schema_version;
    expect(isValidSchemaVersion(sv)).toBe(true);
  });

  test("gateway_capabilities has expected structure", () => {
    const data = loadFixture("gateway_capabilities.json") as Record<string, unknown>;
    expect(data.gateway_version).toBe("1.6.0");
    expect(data.max_event_page_size).toBe(1000);
    expect(Array.isArray(data.auth_modes)).toBe(true);
  });
});

// -- Planning artifact fixture --

describe("planning artifact fixture", () => {
  test("planning_artifact validates schema version", () => {
    const data = loadFixture("planning_artifact.json");
    const sv = (data as Record<string, unknown>).schema_version;
    expect(isValidSchemaVersion(sv)).toBe(true);
  });

  test("planning_artifact has expected fields", () => {
    const data = loadFixture("planning_artifact.json") as Record<string, unknown>;
    expect(data.kind).toBe("milestone_draft");
    expect(data.approved).toBe(false);
    expect(data.published_to_tracker).toBe(false);
  });
});

// -- Planning session summary fixture --

describe("planning session summary fixture", () => {
  test("planning_session_summary validates schema version", () => {
    const data = loadFixture("planning_session_summary.json");
    const sv = (data as Record<string, unknown>).schema_version;
    expect(isValidSchemaVersion(sv)).toBe(true);
  });

  test("planning_session_summary has expected fields", () => {
    const data = loadFixture("planning_session_summary.json") as Record<string, unknown>;
    expect(data.status).toBe("draft");
    expect(data.artifact_count).toBe(3);
  });
});

// -- Action dispatch fixture --

describe("action dispatch fixture", () => {
  test("action_dispatch validates schema version", () => {
    const data = loadFixture("action_dispatch.json");
    const sv = (data as Record<string, unknown>).schema_version;
    expect(isValidSchemaVersion(sv)).toBe(true);
  });

  test("action_dispatch has expected fields", () => {
    const data = loadFixture("action_dispatch.json") as Record<string, unknown>;
    expect(data.action_kind).toBe("retry");
    expect(data.idempotency_key).toBe("idem-1");
    const target = data.target_entity as Record<string, unknown>;
    expect(target.entity_kind).toBe("run");
    expect(target.entity_id).toBe("run-1");
  });
});

// -- Forward compatibility --

describe("forward compatibility", () => {
  test("envelope with extra unknown fields still validates", () => {
    const raw = readFileSync(resolve(fixturesDir, "envelope_terminal_frame.json"), "utf-8");
    const data = JSON.parse(raw) as Record<string, unknown>;
    // Add extra unknown fields.
    data.future_field = "something";
    data.new_payload_v2 = { nested: true };
    expect(isValidGatewayEnvelope(data)).toBe(true);
    expect(() => assertValidGatewayEnvelope(data)).not.toThrow();
  });

  test("schema version minor/patch changes are accepted", () => {
    expect(() =>
      assertCompatibleSchemaVersion({ major: 1, minor: 5, patch: 2 }),
    ).not.toThrow();
  });

  test("schema version major 2 is rejected", () => {
    expect(() =>
      assertCompatibleSchemaVersion({ major: 2, minor: 0, patch: 0 }),
    ).toThrow(/unsupported schema major=2/);
  });
});