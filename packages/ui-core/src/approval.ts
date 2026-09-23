import type { ApprovalRequest, OperatorInteraction } from "@opensymphony/gateway-schema";
import { escapeHtml, escapeAttr } from "./html.js";

/** Decision a user can make on an approval request. */
export type ApprovalDecision = "approved" | "rejected";

/** Render a pending approval list as a lightweight HTML string. */
export function renderApprovalList(
  approvals: ApprovalRequest[],
  opts?: { onDecide?: (id: string, decision: ApprovalDecision, explanation?: string) => void },
): string {
  if (approvals.length === 0) {
    return `<div class="os-approval-list os-empty" data-testid="approval-list">No pending approvals</div>`;
  }
  const items = approvals
    .map((approval) => {
      const riskLevelClass = approval.risk_summary ? escapeAttr(approval.risk_summary.level) : "";
      const risk = approval.risk_summary
        ? `<div class="os-approval-risk os-risk-${riskLevelClass}" data-testid="approval-risk">${escapeHtml(approval.risk_summary.level)}: ${approval.risk_summary.reasons.map(escapeHtml).join("; ")}</div>`
        : "";
      const target = approval.target_context
        ? `<div class="os-approval-target" data-testid="approval-target">${renderTargetContext(approval.target_context)}</div>`
        : "";
      const actor = approval.actor
        ? `<div class="os-approval-actor" data-testid="approval-actor">${escapeHtml(approval.actor.display_name ?? approval.actor.actor_id)} (${escapeHtml(approval.actor.actor_kind)})</div>`
        : "";
      // Only render decision buttons when there is a handler AND the approval is still pending.
      const operator = approval.operator_interaction;
      const operatorChoices = operator && opts?.onDecide && approval.status === "pending"
        ? operator.kind === "permission"
          ? `<div class="os-approval-options">${operator.options.map((option) => {
              const decision = ["allow_once", "allow_always"].includes(option.kind) ? "approved"
                : ["reject_once", "reject_always"].includes(option.kind) ? "rejected" : "";
              return decision ? `<button data-testid="operator-permission-option" data-approval-id="${escapeAttr(approval.approval_id)}" data-option-id="${escapeAttr(option.id)}" data-decision="${decision}">${escapeHtml(option.label)}</button>` : "";
            }).join("")}<button data-testid="operator-permission-cancel" data-approval-id="${escapeAttr(approval.approval_id)}">Cancel</button></div>`
          : `<div class="os-approval-options"><button data-testid="operator-plan-approve" data-approval-id="${escapeAttr(approval.approval_id)}">Approve plan</button><button data-testid="operator-plan-reject" data-approval-id="${escapeAttr(approval.approval_id)}">Reject plan</button><button data-testid="operator-plan-cancel" data-approval-id="${escapeAttr(approval.approval_id)}">Cancel</button></div>`
        : "";
      const explain = !operator && opts?.onDecide && approval.status === "pending"
        ? `<div class="os-approval-explain">
            <input type="text" class="os-approval-explanation" data-testid="approval-explanation" placeholder="Explain your decision (optional)" />
            <button class="os-approve-button" data-testid="approve-button" data-approval-id="${escapeAttr(approval.approval_id)}">Approve</button>
            <button class="os-deny-button" data-testid="deny-button" data-approval-id="${escapeAttr(approval.approval_id)}">Deny</button>
          </div>`
        : "";
      return `<div class="os-approval-item" data-testid="approval-item" data-approval-id="${escapeAttr(approval.approval_id)}" data-approval-kind="${escapeAttr(approval.kind)}">
        <div class="os-approval-title">${escapeHtml(approval.title)}</div>
        <div class="os-approval-description">${escapeHtml(approval.description)}</div>
        ${actor}
        ${target}
        ${risk}
        ${operatorChoices}
        ${explain}
      </div>`;
    })
    .join("");
  return `<div class="os-approval-list" data-testid="approval-list">${items}</div>`;
}

/** Render bounded, offered multiple-choice ACP questions without a free-text field. */
export function renderOperatorInputs(inputs: OperatorInteraction[]): string {
  return inputs.filter((input) => input.kind === "question").map((input) => `
    <form class="os-operator-input" data-testid="operator-input" data-request-id="${escapeAttr(input.request_id)}">
      <div class="os-approval-title">${escapeHtml(input.title)}</div>
      ${input.questions.map((question) => `<fieldset data-question-id="${escapeAttr(question.id)}"><legend>${escapeHtml(question.prompt)}</legend>
        ${question.options.map((option) => `<label><input type="${question.allow_multiple ? "checkbox" : "radio"}" name="${escapeAttr(input.request_id + ":" + question.id)}" value="${escapeAttr(option.id)}" />${escapeHtml(option.label)}</label>`).join("")}
      </fieldset>`).join("")}
      <button type="button" data-testid="operator-input-answer">Answer</button>
      <button type="button" data-testid="operator-input-decline">Decline</button>
      <button type="button" data-testid="operator-input-cancel">Cancel</button>
    </form>`).join("");
}

function renderTargetContext(ctx: ApprovalRequest["target_context"]): string {
  if (!ctx) return "";
  const parts: string[] = [];
  if (ctx.file_path) parts.push(`file: ${escapeHtml(ctx.file_path)}`);
  if (ctx.command) parts.push(`cmd: ${escapeHtml(ctx.command)}`);
  if (ctx.issue_id) parts.push(`issue id: ${escapeHtml(ctx.issue_id)}`);
  if (ctx.issue_identifier) parts.push(`issue: ${escapeHtml(ctx.issue_identifier)}`);
  if (ctx.run_id) parts.push(`run: ${escapeHtml(ctx.run_id)}`);
  return parts.join(" | ");
}
