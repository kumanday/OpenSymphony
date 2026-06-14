/**
 * @jest-environment node
 *
 * Unit tests for the approval list renderer.
 */

import type { ApprovalRequest } from "@opensymphony/gateway-schema";
import { renderApprovalList, type ApprovalDecision } from "../src/approval.js";

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
});
