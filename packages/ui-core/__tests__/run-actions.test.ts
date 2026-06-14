/**
 * @jest-environment node
 *
 * Unit tests for the run action bar helpers, receipts, and audit trail.
 */

import {
  buildActionBarItems,
  renderActionBar,
  renderActionReceipt,
  renderAuditTrailEntry,
} from "../src/run-actions.js";
import type { ActionReceipt, RunDetail } from "@opensymphony/gateway-schema";

function runFixture(opts: Partial<RunDetail> & Pick<RunDetail, "run_id">): RunDetail {
  const base: RunDetail = {
    schema_version: { major: 1, minor: 0, patch: 0 },
    run_id: opts.run_id,
    issue_id: opts.run_id,
    issue_identifier: opts.run_id,
    worker_id: "worker-1",
    status: "running",
    claimed_at: "2025-09-01T00:00:00Z",
    turn_count: 1,
    max_turns: 8,
    input_tokens: 100,
    output_tokens: 50,
    cache_read_tokens: 0,
    runtime_seconds: 30,
  };
  return { ...base, ...opts };
}

const receipt: ActionReceipt = {
  schema_version: { major: 1, minor: 0, patch: 0 },
  action_id: "action-1",
  correlation_id: "corr-1",
  status: "accepted",
  expected_followup: ["action_completion", "run_lifecycle"],
  issued_at: "2025-09-01T00:00:00Z",
};

describe("buildActionBarItems", () => {
  it("renders all standard actions when allowed and safe", () => {
    const run = runFixture({
      allowed_actions: ["retry", "cancel", "rehydrate", "detach"],
      safe_actions: {
        retry: true,
        cancel: true,
        rehydrate: true,
        detach: true,
      },
    });
    const items = buildActionBarItems(run);
    const actions = items.map((i) => i.action);
    expect(actions).toContain("retry");
    expect(actions).toContain("cancel");
    expect(actions).toContain("rehydrate");
    expect(actions).toContain("detach");
    const standardItems = items.filter((i) =>
      ["retry", "cancel", "rehydrate", "detach"].includes(i.action),
    );
    expect(standardItems.every((i) => i.enabled)).toBe(true);
  });

  it("disables unsafe retry and warns against duplicate-run retries", () => {
    const run = runFixture({
      allowed_actions: ["retry"],
      safe_actions: {
        retry: false,
        cancel: true,
        rehydrate: true,
        detach: false,
      },
      liveness: { phase: "active", stream: "healthy" },
    });
    const retry = buildActionBarItems(run).find((i) => i.action === "retry");
    expect(retry?.enabled).toBe(false);
    expect(retry?.warning).toMatch(/Prevented duplicate-run retry/);
  });

  it("disables actions not advertised by the gateway", () => {
    const run = runFixture({
      allowed_actions: ["cancel"],
      safe_actions: {
        retry: true,
        cancel: true,
        rehydrate: true,
        detach: true,
      },
    });
    const items = buildActionBarItems(run);
    expect(items.find((i) => i.action === "retry")?.enabled).toBe(false);
    expect(items.find((i) => i.action === "cancel")?.enabled).toBe(true);
  });

  it("shows extra actions only when allowed", () => {
    const run = runFixture({
      allowed_actions: ["retry", "comment", "create_followup", "open_workspace", "debug"],
      safe_actions: {
        retry: true,
        cancel: true,
        rehydrate: true,
        detach: false,
      },
    });
    const items = buildActionBarItems(run);
    expect(items.find((i) => i.action === "comment")?.enabled).toBe(true);
    expect(items.find((i) => i.action === "create_followup")?.enabled).toBe(true);
    expect(items.find((i) => i.action === "open_workspace")?.enabled).toBe(true);
    expect(items.find((i) => i.action === "debug")?.enabled).toBe(true);
  });

  it("warns when cancel is unsafe for a stalled run", () => {
    const run = runFixture({
      allowed_actions: ["cancel"],
      safe_actions: {
        retry: false,
        cancel: false,
        rehydrate: true,
        detach: false,
      },
      liveness: { phase: "stalled", stream: "dead" },
    });
    const cancel = buildActionBarItems(run).find((i) => i.action === "cancel");
    expect(cancel?.enabled).toBe(false);
    expect(cancel?.warning).toMatch(/stalled/);
  });
});

describe("renderActionBar", () => {
  it("renders enabled and disabled buttons with warnings", () => {
    const html = renderActionBar([
      { action: "retry", label: "Retry", enabled: true },
      { action: "cancel", label: "Cancel", enabled: false, warning: "Unsafe" },
    ]);
    expect(html).toContain('data-action="retry"');
    expect(html).not.toContain('data-action="retry" disabled');
    expect(html).toContain('data-action="cancel" disabled');
    expect(html).toContain("Unsafe");
  });

  it("renders an empty placeholder when no actions are available", () => {
    const html = renderActionBar([]);
    expect(html).toContain("No actions available");
  });
});

describe("renderActionReceipt", () => {
  it("renders status and expected follow-up events", () => {
    const html = renderActionReceipt(receipt);
    expect(html).toContain('data-testid="action-receipt"');
    expect(html).toContain("accepted");
    expect(html).toContain("action_completion");
    expect(html).toContain("run_lifecycle");
  });

  it("escapes dynamic attribute values", () => {
    const malicious: ActionReceipt = {
      ...receipt,
      action_id: 'action-"-onclick',
      status: 'accepted" data-x',
    };
    const html = renderActionReceipt(malicious);
    expect(html).toContain('data-action-id="action-&quot;-onclick"');
    expect(html).toContain('data-status="accepted&quot; data-x"');
    expect(html).not.toContain('data-action-id="action-"-onclick"');
    expect(html).not.toContain('data-status="accepted" data-x"');
  });
});

describe("renderAuditTrailEntry", () => {
  it("renders actor, action, status, and details", () => {
    const html = renderAuditTrailEntry({
      timestamp: "2025-09-01T00:00:00Z",
      actor: "operator",
      action: "cancel",
      target: "run-1",
      status: "accepted",
      details: "acknowledged by harness",
    });
    expect(html).toContain('data-testid="audit-trail-entry"');
    expect(html).toContain("operator");
    expect(html).toContain("cancel");
    expect(html).toContain("acknowledged by harness");
    expect(html).toContain("accepted");
  });

  it("escapes dynamic action and status values", () => {
    const html = renderAuditTrailEntry({
      timestamp: "2025-09-01T00:00:00Z",
      actor: "operator",
      action: 'cancel"-x',
      target: "run-1",
      status: 'accepted"-x',
      details: "acknowledged by harness",
    });
    expect(html).toContain('data-action="cancel&quot;-x"');
    expect(html).toContain('data-status="accepted&quot;-x"');
  });
});
