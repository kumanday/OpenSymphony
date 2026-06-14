import type {
  RunValidationSummary,
  ValidationCommand,
  ValidationEvidenceItem,
} from "@opensymphony/gateway-schema";
import { escapeHtml, escapeAttr } from "./html.js";

/** Render a validation summary as a lightweight HTML string. */
export function renderValidationSummary(
  summary: RunValidationSummary,
): string {
  const statusClass = `os-validation-status-${escapeAttr(summary.overall_status)}`;
  const header = `<div class="os-validation-header" data-testid="validation-header">
    <span class="os-validation-status ${statusClass}">${escapeHtml(summary.overall_status)}</span>
    <span class="os-validation-timestamp">${escapeHtml(summary.generated_at)}</span>
  </div>`;
  const commands = summary.commands.length
    ? `<div class="os-validation-commands" data-testid="validation-commands">${summary.commands.map(renderCommand).join("")}</div>`
    : `<div class="os-validation-commands os-empty" data-testid="validation-commands">No validation commands</div>`;
  const evidence = summary.evidence.length
    ? `<div class="os-validation-evidence" data-testid="validation-evidence">${summary.evidence.map(renderEvidence).join("")}</div>`
    : `<div class="os-validation-evidence os-empty" data-testid="validation-evidence">No validation evidence</div>`;
  return `<div class="os-validation-summary" data-testid="validation-summary" data-run-id="${escapeAttr(summary.run_id)}">${header}${commands}${evidence}</div>`;
}

function renderCommand(cmd: ValidationCommand): string {
  const statusClass = `os-validation-status-${escapeAttr(cmd.status)}`;
  return `<div class="os-validation-command" data-testid="validation-command" data-command-id="${escapeAttr(cmd.command_id)}">
    <div class="os-command-header">
      <span class="os-command-status ${statusClass}">${escapeHtml(cmd.status)}</span>
      <code class="os-command-text">${escapeHtml(cmd.command)}</code>
    </div>
    ${cmd.exit_code !== undefined ? `<div class="os-command-exit" data-testid="command-exit">exit ${cmd.exit_code}</div>` : ""}
    ${cmd.stderr_summary ? `<div class="os-command-stderr">${escapeHtml(cmd.stderr_summary)}</div>` : ""}
  </div>`;
}

function renderEvidence(item: ValidationEvidenceItem): string {
  const statusClass = `os-validation-status-${escapeAttr(item.status)}`;
  return `<div class="os-validation-evidence-item" data-testid="validation-evidence-item" data-evidence-id="${escapeAttr(item.evidence_id)}">
    <div class="os-evidence-header">
      <span class="os-evidence-status ${statusClass}">${escapeHtml(item.status)}</span>
      <span class="os-evidence-label">${escapeHtml(item.label)}</span>
    </div>
    <div class="os-evidence-summary">${escapeHtml(item.summary)}</div>
    ${item.file_path ? `<div class="os-evidence-location">${escapeHtml(item.file_path)}${item.line_number ? ":" + item.line_number : ""}</div>` : ""}
  </div>`;
}
