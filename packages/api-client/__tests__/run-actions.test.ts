/**
 * Gateway action integration tests using fake run data.
 *
 * Verifies that MockGatewayTransport and HttpGatewayTransport action methods
 * produce ActionReceipts with correlated IDs, expected events, and stable
 * idempotency keys.
 */

import { MockGatewayTransport, HttpGatewayTransport } from "../src/index.js";
import { schemaVersionV1 } from "@opensymphony/gateway-schema";
import type { ActionReceipt } from "@opensymphony/gateway-schema";

const schemaVersion = schemaVersionV1();

const runDetail = {
  schema_version: schemaVersion,
  run_id: "run-actions-1",
  issue_id: "issue-1",
  issue_identifier: "COE-414",
  worker_id: "worker-1",
  status: "running" as const,
  claimed_at: "2025-09-01T00:00:00Z",
  turn_count: 1,
  max_turns: 8,
  input_tokens: 100,
  output_tokens: 50,
  cache_read_tokens: 0,
  runtime_seconds: 30,
};

const receipt: ActionReceipt = {
  schema_version: schemaVersion,
  action_id: "action-override",
  correlation_id: "corr-override",
  status: "accepted",
  expected_followup: ["action_completion", "run_lifecycle"],
  issued_at: "2025-09-01T00:00:00Z",
  reason: "override",
};

function assertReceiptShape(receipt: ActionReceipt): void {
  expect(receipt.schema_version).toBeDefined();
  expect(receipt.action_id).toBeDefined();
  expect(receipt.correlation_id).toBeDefined();
  expect(receipt.status).toBeDefined();
  expect(receipt.issued_at).toBeDefined();
}

describe("MockGatewayTransport action methods", () => {
  let transport: MockGatewayTransport;

  beforeEach(() => {
    transport = new MockGatewayTransport({
      baseUri: "http://mock.local",
      runDetails: [runDetail],
    });
  });

  it("cancelRun returns a receipt correlated to the run", async () => {
    const result = await transport.cancelRun(runDetail.run_id);
    assertReceiptShape(result);
    expect(result.correlation_id).toContain(`cancel-${runDetail.run_id}-`);
    expect(result.status).toBe("accepted");
  });

  it("retryRun returns a receipt with expected events and deterministic idempotency key", async () => {
    const result = await transport.retryRun(runDetail.run_id);
    assertReceiptShape(result);
    expect(result.correlation_id).toContain(`retry-${runDetail.run_id}-`);
  });

  it("rehydrateRun returns a receipt for the run", async () => {
    const result = await transport.rehydrateRun(runDetail.run_id);
    assertReceiptShape(result);
    expect(result.correlation_id).toContain(`rehydrate-${runDetail.run_id}-`);
  });

  it("commentRun includes the run and comment text in the correlation and idempotency key", async () => {
    const text = "retry after config fix";
    const result = await transport.commentRun(runDetail.run_id, text);
    assertReceiptShape(result);
    expect(result.correlation_id).toContain(`comment-${runDetail.run_id}-`);
    expect(result.correlation_id).not.toContain(text);
  });

  it("createFollowup returns a receipt for the run", async () => {
    const result = await transport.createFollowup(runDetail.run_id, { title: "Follow-up issue" });
    assertReceiptShape(result);
    expect(result.correlation_id).toContain(`followup-${runDetail.run_id}-`);
  });

  it("openWorkspace returns a receipt with open_workspace intent", async () => {
    const result = await transport.openWorkspace(runDetail.run_id);
    assertReceiptShape(result);
    expect(result.correlation_id).toContain(`workspace-${runDetail.run_id}-`);
  });

  it("approvalDecision returns a receipt correlated to the approval", async () => {
    const result = await transport.approvalDecision("approval-1", "approved", "approved for test");
    assertReceiptShape(result);
    expect(result.correlation_id).toContain("approval-approval-1-");
    expect(result.status).toBe("accepted");
  });

  it("setActionReceipt overrides generated receipts for a correlation id", async () => {
    transport.setActionReceipt("corr-override", receipt);
    const result = await transport.dispatchAction({
      schema_version: schemaVersion,
      correlation_id: "corr-override",
      action_kind: "cancel",
      target_entity: { entity_kind: "run", entity_id: runDetail.run_id },
    });
    expect(result).toEqual(receipt);
  });
});

describe("HttpGatewayTransport action integration", () => {
  const baseUri = "http://gateway.local";

  afterEach(() => {
    jest.restoreAllMocks();
  });

  function mockFetch(response: unknown): jest.SpyInstance {
    return jest.spyOn(globalThis, "fetch").mockResolvedValue({
      ok: true,
      status: 200,
      statusText: "OK",
      json: async () => response,
      text: async () => JSON.stringify(response),
    } as Response);
  }

  it("cancelRun POSTs a cancel action to the dispatch endpoint", async () => {
    const fetchSpy = mockFetch(receipt);
    const transport = new HttpGatewayTransport({ baseUri });
    const result = await transport.cancelRun("run-1");

    expect(fetchSpy).toHaveBeenCalledTimes(1);
    const requestUrl = fetchSpy.mock.calls[0][0] as string;
    expect(requestUrl).toBe(`${baseUri}/api/v1/actions/dispatch`);
    const requestInit = fetchSpy.mock.calls[0][1] as RequestInit;
    expect(requestInit.method).toBe("POST");
    const body = JSON.parse(requestInit.body as string);
    expect(body.action_kind).toBe("cancel");
    expect(body.target_entity).toEqual({ entity_kind: "run", entity_id: "run-1" });
    expect(body.idempotency_key).toBe("cancel-run-1");
    expect(result.correlation_id).toBe(receipt.correlation_id);
  });

  it("retryRun includes retry action kind and idempotency key", async () => {
    const fetchSpy = mockFetch(receipt);
    const transport = new HttpGatewayTransport({ baseUri });
    await transport.retryRun("run-2");

    const requestInit = fetchSpy.mock.calls[0][1] as RequestInit;
    const body = JSON.parse(requestInit.body as string);
    expect(body.action_kind).toBe("retry");
    expect(body.idempotency_key).toBe("retry-run-2");
  });

  it("approvalDecision POSTs the decision and explanation", async () => {
    const fetchSpy = mockFetch(receipt);
    const transport = new HttpGatewayTransport({ baseUri });
    await transport.approvalDecision("approval-1", "rejected", "unsafe");

    const requestInit = fetchSpy.mock.calls[0][1] as RequestInit;
    const body = JSON.parse(requestInit.body as string);
    expect(body.action_kind).toBe("approval_decision");
    expect(body.target_entity).toEqual({ entity_kind: "approval", entity_id: "approval-1" });
    expect(body.payload).toEqual({ decision: "rejected", explanation: "unsafe" });
    expect(body.idempotency_key).toBe("approval-approval-1-rejected");
  });

  it("openWorkspace POSTs the open_workspace action kind", async () => {
    const fetchSpy = mockFetch(receipt);
    const transport = new HttpGatewayTransport({ baseUri });
    await transport.openWorkspace("run-1");

    const requestInit = fetchSpy.mock.calls[0][1] as RequestInit;
    const body = JSON.parse(requestInit.body as string);
    expect(body.action_kind).toBe("open_workspace");
    expect(body.target_entity).toEqual({ entity_kind: "run", entity_id: "run-1" });
    expect(body.idempotency_key).toBe("workspace-run-1");
    expect(body.payload).toBeUndefined();
  });

  it("debugRun POSTs the debug action kind", async () => {
    const fetchSpy = mockFetch(receipt);
    const transport = new HttpGatewayTransport({ baseUri });
    await transport.debugRun("run-1");

    const requestInit = fetchSpy.mock.calls[0][1] as RequestInit;
    const body = JSON.parse(requestInit.body as string);
    expect(body.action_kind).toBe("debug");
    expect(body.target_entity).toEqual({ entity_kind: "run", entity_id: "run-1" });
    expect(body.idempotency_key).toBe("debug-run-1");
  });

  it("createFollowup uses a payload-aware idempotency key", async () => {
    const fetchSpy = mockFetch(receipt);
    const transport = new HttpGatewayTransport({ baseUri });
    await transport.createFollowup("run-1", { title: "Follow-up A" });
    await transport.createFollowup("run-1", { title: "Follow-up B" });

    const first = JSON.parse((fetchSpy.mock.calls[0][1] as RequestInit).body as string);
    const second = JSON.parse((fetchSpy.mock.calls[1][1] as RequestInit).body as string);
    expect(first.action_kind).toBe("create_followup");
    expect(first.idempotency_key).not.toBe(second.idempotency_key);
    expect(first.idempotency_key).toContain("followup-run-1-");
    expect(second.idempotency_key).toContain("followup-run-1-");
  });
});
