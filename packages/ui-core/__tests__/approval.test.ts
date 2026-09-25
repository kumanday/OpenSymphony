/**
 * @jest-environment node
 *
 * Unit tests for the approval list renderer.
 */

import type { ApprovalRequest, OperatorInteraction } from "@opensymphony/gateway-schema";
import { renderApprovalList, renderOperatorInputs, type ApprovalDecision } from "../src/approval.js";

const operator: OperatorInteraction = {
  request_id: "req-1", run_id: "run-1", issue_id: "issue-1", issue_identifier: "COE-612",
  session_id: "session-1", generation: 1, rpc_id: "0", kind: "permission", title: "Use tool",
  options: [{ id: "allow-once", label: "Allow once", kind: "allow_once" },
    { id: "deny-once", label: "Deny", kind: "reject_once" }], questions: [],
  requested_at: "2026-09-23T00:00:00Z", expires_at: "2026-09-23T00:05:00Z",
};

function approvalFixture(opts: Partial<ApprovalRequest> & Pick<ApprovalRequest, "approval_id" | "status">): ApprovalRequest {
  const base: ApprovalRequest = {
    schema_version: { major: 1, minor: 0, patch: 0 },
    approval_id: opts.approval_id,
    kind: "command",
    title: "Approve command",
    description: "A command needs approval.",
    status: opts.status,
    actor: {
      actor_id: "actor-1",
      actor_kind: "user",
      display_name: "Operator",
    },
    target_context: {
      command: "rm -rf /",
    },
    risk_summary: {
      level: "high",
      reasons: ["destructive command"],
    },
  };
  return { ...base, ...opts };
}

describe("renderApprovalList", () => {
  it("renders decision buttons only when a handler exists and the approval is pending", () => {
    const handler = jest.fn((_id: string, _decision: ApprovalDecision, _explanation?: string) => {});
    const pending = approvalFixture({ approval_id: "app-1", status: "pending" });
    const html = renderApprovalList([pending], { onDecide: handler });
    expect(html).toContain('data-testid="approve-button"');
    expect(html).toContain('data-testid="deny-button"');
  });

  it("does not render decision buttons when there is no handler", () => {
    const pending = approvalFixture({ approval_id: "app-1", status: "pending" });
    const html = renderApprovalList([pending]);
    expect(html).not.toContain('data-testid="approve-button"');
    expect(html).not.toContain('data-testid="deny-button"');
  });

  it("does not render decision buttons when the approval is already decided", () => {
    const handler = jest.fn((_id: string, _decision: ApprovalDecision, _explanation?: string) => {});
    const approved = approvalFixture({ approval_id: "app-1", status: "approved" });
    const html = renderApprovalList([approved], { onDecide: handler });
    expect(html).not.toContain('data-testid="approve-button"');
    expect(html).not.toContain('data-testid="deny-button"');
  });

  it("escapes dynamic attribute values", () => {
    const handler = jest.fn((_id: string, _decision: ApprovalDecision, _explanation?: string) => {});
    const pending = approvalFixture({
      approval_id: 'app-"-x',
      status: "pending",
      kind: 'cmd"-x',
    });
    const html = renderApprovalList([pending], { onDecide: handler });
    expect(html).toContain('data-approval-id="app-&quot;-x"');
    expect(html).toContain('data-approval-kind="cmd&quot;-x"');
  });

  it("shows only offered ACP permission options and never generic approve controls", () => {
    const html = renderApprovalList([approvalFixture({ approval_id: "req-1", status: "pending", operator_interaction: operator })], { onDecide: jest.fn() });
    expect(html).toContain('data-option-id="allow-once"');
    expect(html).toContain('data-option-id="deny-once"');
    expect(html).not.toContain('data-testid="approve-button"');
    expect(html).toContain('data-testid="operator-permission-cancel"');
  });

  it("renders structured choices with decline and cancel but no free-text field", () => {
    const html = renderOperatorInputs([{ ...operator, kind: "question", options: [], questions: [
      { id: "region", prompt: "Region?", allow_multiple: false,
        options: [{ id: "east", label: "East", kind: "choice" }, { id: "west", label: "West", kind: "choice" }] },
    ] }]);
    expect(html).toContain('type="radio"');
    expect(html).toContain('data-testid="operator-input-answer"');
    expect(html).toContain('data-testid="operator-input-decline"');
    expect(html).toContain('data-testid="operator-input-cancel"');
    expect(html).not.toContain('type="text"');
  });
});
