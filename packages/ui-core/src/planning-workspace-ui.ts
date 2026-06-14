import { escapeHtml, escapeAttr } from "./task-graph-editor.js";
import type {
  PlanningArtifactRevision,
  PlanningArtifactWithRevisions,
  PlanningNode,
  PlanningValidationMessage,
  PlanningWorkspaceState,
  PlanningWorkspaceTab,
  DiffLine,
} from "./planning-workspace.js";
import {
  artifactKindLabels,
  artifactKindOrder,
  computeArtifactDiff,
  findArtifactById,
  findRevisionById,
  getArtifactDepths,
  getArtifactRevisionOptions,
  validatePlanningWorkspace,
} from "./planning-workspace.js";

export interface PlanningEditState {
  nodeId: string | null;
  title: string;
  state: string;
}

export const emptyPlanningEditState: PlanningEditState = {
  nodeId: null,
  title: "",
  state: "",
};

const tabLabels: Record<Exclude<PlanningWorkspaceTab, "conversation">, string> = {
  artifact: "Artifact",
  hierarchy: "Hierarchy",
  dependencies: "Dependencies",
  criteria: "Acceptance & Verification",
  validation: "Validation",
  diff: "Diff",
};

export function renderPlanningWorkspace(
  state: PlanningWorkspaceState,
  editState: PlanningEditState,
): string {
  const validationMessages = validatePlanningWorkspace(state);
  const validationCount = validationMessages.filter((m) => m.level === "error").length +
    validationMessages.filter((m) => m.level === "warning").length;
  return `
    <section class="os-panel os-planning-panel">
      <div class="os-planning-head">
        <div>
          <h2>${escapeHtml(state.title || "Planning Workspace")}</h2>
          <span class="os-meta">${escapeHtml(state.session_id)} • ${escapeHtml(state.project_id)}</span>
        </div>
        <div class="os-plan-tabs">
          ${(Object.keys(tabLabels) as (Exclude<PlanningWorkspaceTab, "conversation">)[])
            .map((tab) => renderTab(tab, state.activeTab, validationCount))
            .join("")}
        </div>
      </div>
      <div class="os-planning-layout">
        <div class="os-planning-conversation">
          ${renderConversationPane(state)}
        </div>
        <div class="os-planning-content">
          ${renderActiveTab(state, editState, validationMessages)}
        </div>
      </div>
    </section>
  `;
}

function renderTab(tab: Exclude<PlanningWorkspaceTab, "conversation">, active: PlanningWorkspaceTab, validationCount: number): string {
  const isActive = tab === active ? "os-plan-tab-active" : "";
  const badge = tab === "validation" && validationCount > 0
    ? `<span class="os-badge os-badge-blocked">${validationCount}</span>`
    : "";
  return `<button type="button" class="os-plan-tab ${isActive}" data-plan-tab="${escapeAttr(tab)}">${escapeHtml(tabLabels[tab])}${badge}</button>`;
}

function renderActiveTab(
  state: PlanningWorkspaceState,
  editState: PlanningEditState,
  validationMessages: PlanningValidationMessage[],
): string {
  switch (state.activeTab) {
    case "artifact":
      return renderArtifactEditor(state);
    case "hierarchy":
      return renderHierarchyEditor(state, editState);
    case "dependencies":
      return renderDependenciesEditor(state);
    case "criteria":
      return renderCriteriaEditor(state);
    case "validation":
      return renderValidationPanel(validationMessages);
    case "diff":
      return renderDiffEditor(state);
    default:
      return "";
  }
}

function renderConversationPane(state: PlanningWorkspaceState): string {
  const messages = state.messages.map((msg) => `
    <div class="os-conversation-message os-conversation-${escapeAttr(msg.role)}">
      <span class="os-conversation-role">${escapeHtml(msg.role)}</span>
      <p>${escapeHtml(msg.body)}</p>
    </div>
  `).join("");
  return `
    <div class="os-section-head"><h3>Conversation</h3></div>
    <div class="os-conversation-list">${messages || `<div class="os-empty">No messages yet</div>`}</div>
    <label class="os-field">
      <span>Message</span>
      <textarea data-plan-composer rows="3" placeholder="Ask a question or provide guidance...">${escapeHtml(state.composerDraft)}</textarea>
    </label>
    <div class="os-planning-actions">
      <button type="button" data-plan-send-message>Send</button>
    </div>
  `;
}

function renderArtifactEditor(state: PlanningWorkspaceState): string {
  const selected = findArtifactById(state, state.selectedArtifactId);
  const revision = findRevisionById(selected, state.selectedRevisionId);
  const artifactOptions = state.artifacts
    .map((a) => {
      const selected = a.artifact_id === state.selectedArtifactId ? "selected" : "";
      return `<option value="${escapeAttr(a.artifact_id)}" ${selected}>${escapeHtml(artifactKindLabels[a.kind])}: ${escapeHtml(a.title)}</option>`;
    })
    .join("");
  const revisionOptions = getArtifactRevisionOptions(selected)
    .map((opt) => {
      const selected = opt.revision_id === state.selectedRevisionId ? "selected" : "";
      return `<option value="${escapeAttr(opt.revision_id)}" ${selected}>${escapeHtml(opt.label)}</option>`;
    })
    .join("");
  return `
    <div class="os-section-head"><h3>Artifact Editor</h3></div>
    <div class="os-inline-fields">
      <label class="os-field">
        <span>Artifact</span>
        <select data-plan-artifact-select>${artifactOptions || `<option value="">No artifacts</option>`}</select>
      </label>
      <label class="os-field">
        <span>Revision</span>
        <select data-plan-revision-select>${revisionOptions || `<option value="">No revisions</option>`}</select>
      </label>
    </div>
    <div class="os-planning-actions">
      <button type="button" data-plan-add-artifact>Add Artifact</button>
    </div>
    <label class="os-field">
      <span>Content</span>
      <textarea data-plan-artifact-content rows="14">${escapeHtml(revision?.content ?? "")}</textarea>
    </label>
    <div class="os-planning-actions">
      <button type="button" data-plan-save-artifact>Save</button>
    </div>
  `;
}

function renderHierarchyEditor(state: PlanningWorkspaceState, editState: PlanningEditState): string {
  const depths = getArtifactDepths(state.nodes);
  const roots = state.nodes.filter((n) => !n.parent_id);
  const rows = roots.map((node) => renderHierarchyNode(node, state, depths, editState, 0)).join("");
  return `
    <div class="os-section-head"><h3>Hierarchy Editor</h3></div>
    <div class="os-tg-toolbar">
      <button type="button" data-plan-add-node="milestone">+ Milestone</button>
      <button type="button" data-plan-add-node="issue">+ Issue</button>
      <button type="button" data-plan-add-node="sub_issue">+ Sub-issue</button>
    </div>
    <div class="os-plan-hierarchy">${rows || `<div class="os-empty">No hierarchy nodes. Add a milestone to start.</div>`}</div>
  `;
}

function renderHierarchyNode(
  node: PlanningNode,
  state: PlanningWorkspaceState,
  depths: Map<string, number>,
  editState: PlanningEditState,
  depth: number,
): string {
  const isEditing = editState.nodeId === node.node_id;
  const isSelected = state.selectedNodeId === node.node_id;
  const isExpanded = state.expandedNodeIds.has(node.node_id) || node.children.length === 0;
  const toggle = node.children.length > 0
    ? `<button type="button" class="os-plan-toggle" data-plan-node-toggle="${escapeAttr(node.node_id)}">${isExpanded ? "▼" : "▶"}</button>`
    : `<span class="os-plan-toggle-spacer"></span>`;
  const titleContent = isEditing
    ? `<input class="os-inline-input" data-plan-node-title="${escapeAttr(node.node_id)}" value="${escapeAttr(editState.title)}" />`
    : `<strong>${escapeHtml(node.identifier)}</strong> <span>${escapeHtml(node.title)}</span>`;
  const stateContent = isEditing
    ? `<input class="os-inline-input os-inline-state" data-plan-node-state="${escapeAttr(node.node_id)}" value="${escapeAttr(editState.state)}" />`
    : `<em>${escapeHtml(node.state)}</em>`;
  const editButtons = isEditing
    ? `<button type="button" data-plan-node-save="${escapeAttr(node.node_id)}">Save</button><button type="button" data-plan-node-cancel="${escapeAttr(node.node_id)}">Cancel</button>`
    : `<button type="button" data-plan-node-edit="${escapeAttr(node.node_id)}">Edit</button>`;
  const children = isExpanded
    ? node.children
        .map((childId) => state.nodes.find((n) => n.node_id === childId))
        .filter((n): n is PlanningNode => Boolean(n))
        .map((child) => renderHierarchyNode(child, state, depths, editState, depth + 1))
        .join("")
    : "";
  return `
    <div class="os-plan-hierarchy-row ${isSelected ? "is-selected" : ""}" data-plan-node-select="${escapeAttr(node.node_id)}" style="padding-left: ${depth * 18}px">
      ${toggle}
      <div class="os-plan-node-body">
        <span class="os-node-kind">${escapeHtml(node.kind.replace(/_/g, " "))}</span>
        ${titleContent}
        ${stateContent}
        <div class="os-node-actions">
          ${editButtons}
          <button type="button" data-plan-add-child="${escapeAttr(node.node_id)}">+</button>
          <button type="button" data-plan-remove-node="${escapeAttr(node.node_id)}">−</button>
        </div>
      </div>
    </div>
    ${children}
  `;
}

function renderDependenciesEditor(state: PlanningWorkspaceState): string {
  const selected = state.nodes.find((n) => n.node_id === state.selectedNodeId);
  const options = state.nodes
    .filter((n) => n.node_id !== selected?.node_id)
    .map((n) => {
      const isSelected = selected?.blocked_by.includes(n.node_id) ? "selected" : "";
      return `<option value="${escapeAttr(n.node_id)}" ${isSelected}>${escapeHtml(n.identifier)} — ${escapeHtml(n.title)}</option>`;
    })
    .join("");
  const graphSvg = renderDependencyGraphSvg(state);
  return `
    <div class="os-section-head"><h3>Dependencies</h3></div>
    <div class="os-inline-fields">
      <label class="os-field">
        <span>Selected node</span>
        <select data-plan-deps-node-select>${state.nodes.map((n) => {
          const selected = n.node_id === state.selectedNodeId ? "selected" : "";
          return `<option value="${escapeAttr(n.node_id)}" ${selected}>${escapeHtml(n.identifier)} — ${escapeHtml(n.title)}</option>`;
        }).join("") || `<option value="">No nodes</option>`}</select>
      </label>
    </div>
    <label class="os-field">
      <span>Blocked by (multi-select)</span>
      <select data-plan-deps-select multiple size="6">${options || `<option disabled>No other nodes</option>`}</select>
    </label>
    <div class="os-planning-actions">
      <button type="button" data-plan-deps-save>Save</button>
    </div>
    <div class="os-section-head"><h3>Graph View</h3></div>
    <div class="os-plan-graph">${graphSvg}</div>
  `;
}

function renderDependencyGraphSvg(state: PlanningWorkspaceState): string {
  if (state.nodes.length === 0) {
    return `<div class="os-empty">Add hierarchy nodes to see the dependency graph.</div>`;
  }
  const depths = getArtifactDepths(state.nodes);
  const maxDepth = Math.max(0, ...Array.from(depths.values()));
  const levelWidth = 160;
  const rowHeight = 56;
  const width = (maxDepth + 1) * levelWidth + 40;
  const height = Math.max(200, state.nodes.length * rowHeight + 40);
  const positions = new Map<string, { x: number; y: number }>();
  const levelCounts = new Map<number, number>();
  const levelIndex = new Map<number, number>();
  for (const node of state.nodes) {
    const depth = depths.get(node.node_id) ?? 0;
    levelCounts.set(depth, (levelCounts.get(depth) ?? 0) + 1);
  }
  function place(nodeId: string, depth: number, indexInLevel: number): void {
    const count = levelCounts.get(depth) ?? 1;
    const x = 20 + depth * levelWidth;
    const y = 20 + (indexInLevel + 0.5) * (height / Math.max(count, 1));
    positions.set(nodeId, { x, y });
  }
  for (const [depth] of levelCounts) levelIndex.set(depth, 0);
  for (const node of state.nodes) {
    const depth = depths.get(node.node_id) ?? 0;
    const idx = levelIndex.get(depth) ?? 0;
    place(node.node_id, depth, idx);
    levelIndex.set(depth, idx + 1);
  }
  const edges: string[] = [];
  for (const node of state.nodes) {
    const start = positions.get(node.node_id);
    if (!start) continue;
    for (const childId of node.children) {
      const end = positions.get(childId);
      if (!end) continue;
      edges.push(`<line x1="${start.x}" y1="${start.y}" x2="${end.x}" y2="${end.y}" class="os-plan-graph-edge" />`);
    }
    for (const depId of node.blocked_by) {
      const end = positions.get(depId);
      if (!end) continue;
      edges.push(`<line x1="${end.x}" y1="${end.y}" x2="${start.x}" y2="${start.y}" class="os-plan-graph-edge os-plan-graph-dependency" marker-end="url(#arrow)" />`);
    }
  }
  const nodes = state.nodes.map((node) => {
    const pos = positions.get(node.node_id);
    if (!pos) return "";
    const selected = node.node_id === state.selectedNodeId ? "os-plan-graph-node-selected" : "";
    return `
      <g class="os-plan-graph-node ${selected}" data-plan-graph-node="${escapeAttr(node.node_id)}" transform="translate(${pos.x}, ${pos.y})">
        <rect x="-70" y="-22" width="140" height="44" rx="6" />
        <text y="-4" text-anchor="middle">${escapeHtml(node.identifier)}</text>
        <text y="14" text-anchor="middle" class="os-plan-graph-node-sub">${escapeHtml(node.title)}</text>
      </g>
    `;
  }).join("");
  return `
    <svg viewBox="0 0 ${width} ${height}" xmlns="http://www.w3.org/2000/svg">
      <defs>
        <marker id="arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="6" markerHeight="6" orient="auto-start-reverse">
          <path d="M 0 0 L 10 5 L 0 10 z" fill="#667788" />
        </marker>
      </defs>
      ${edges}
      ${nodes}
    </svg>
  `;
}

function renderCriteriaEditor(state: PlanningWorkspaceState): string {
  const criteriaRows = state.criteria.map((c) => `
    <li class="os-plan-checklist-row">
      <input type="checkbox" data-plan-criteria-toggle="${escapeAttr(c.id)}" ${c.checked ? "checked" : ""} />
      <input type="text" data-plan-criteria-text="${escapeAttr(c.id)}" value="${escapeAttr(c.text)}" />
      <button type="button" data-plan-criteria-remove="${escapeAttr(c.id)}">Remove</button>
    </li>
  `).join("");
  const verificationRows = state.verification.map((v) => `
    <li class="os-plan-checklist-row">
      <input type="checkbox" data-plan-verification-toggle="${escapeAttr(v.id)}" ${v.checked ? "checked" : ""} />
      <input type="text" data-plan-verification-text="${escapeAttr(v.id)}" value="${escapeAttr(v.text)}" />
      <button type="button" data-plan-verification-remove="${escapeAttr(v.id)}">Remove</button>
    </li>
  `).join("");
  return `
    <div class="os-section-head"><h3>Acceptance Criteria</h3></div>
    <ul class="os-plan-checklist">${criteriaRows || `<li class="os-empty">No acceptance criteria</li>`}</ul>
    <div class="os-planning-actions">
      <input type="text" data-plan-criteria-new placeholder="Add criterion..." />
      <button type="button" data-plan-criteria-add>Add</button>
    </div>
    <div class="os-section-head"><h3>Verification Expectations</h3></div>
    <ul class="os-plan-checklist">${verificationRows || `<li class="os-empty">No verification expectations</li>`}</ul>
    <div class="os-planning-actions">
      <input type="text" data-plan-verification-new placeholder="Add verification expectation..." />
      <button type="button" data-plan-verification-add>Add</button>
    </div>
  `;
}

function renderValidationPanel(messages: PlanningValidationMessage[]): string {
  const rows = messages.map((m) => `
    <div class="os-plan-validation-row os-plan-validation-${escapeAttr(m.level)}">
      <button type="button" class="os-plan-validation-link" data-plan-validation-link="${escapeAttr(m.message_id)}" data-plan-field-kind="${escapeAttr(m.field_ref?.kind ?? "")}" data-plan-field-id="${escapeAttr(m.field_ref?.id ?? "")}" data-plan-field-sub="${escapeAttr(m.field_ref?.sub_id ?? "")}">
        ${escapeHtml(m.level)}: ${escapeHtml(m.message)}
      </button>
    </div>
  `).join("");
  return `
    <div class="os-section-head"><h3>Plan Validation</h3></div>
    <div class="os-plan-validation-list">${rows || `<div class="os-empty">No validation messages</div>`}</div>
  `;
}

function renderDiffEditor(state: PlanningWorkspaceState): string {
  const selected = findArtifactById(state, state.selectedArtifactId);
  const options = getArtifactRevisionOptions(selected);
  const leftOptions = options
    .map((opt) => {
      const selected = opt.revision_id === state.diffLeftRevisionId ? "selected" : "";
      return `<option value="${escapeAttr(opt.revision_id)}" ${selected}>${escapeHtml(opt.label)}</option>`;
    })
    .join("");
  const rightOptions = options
    .map((opt) => {
      const selected = opt.revision_id === state.diffRightRevisionId ? "selected" : "";
      return `<option value="${escapeAttr(opt.revision_id)}" ${selected}>${escapeHtml(opt.label)}</option>`;
    })
    .join("");
  const left = findRevisionById(selected, state.diffLeftRevisionId);
  const right = findRevisionById(selected, state.diffRightRevisionId);
  const diffLines = left && right ? computeArtifactDiff(left.content, right.content) : [];
  const kindClass: Record<string, string> = {
    added: "add",
    removed: "remove",
    unchanged: "unchanged",
  };
  const diffRows = diffLines.map((line) => `
    <div class="os-plan-diff-line os-plan-diff-${escapeAttr(kindClass[line.kind] ?? line.kind)}">
      <span class="os-plan-diff-lnum">${line.leftLineNumber ?? " "}</span>
      <span class="os-plan-diff-rnum">${line.rightLineNumber ?? " "}</span>
      <span class="os-plan-diff-text">${escapeHtml(line.line)}</span>
    </div>
  `).join("");
  return `
    <div class="os-section-head"><h3>Artifact Diff</h3></div>
    <div class="os-inline-fields">
      <label class="os-field">
        <span>Left</span>
        <select data-plan-diff-left>${leftOptions}</select>
      </label>
      <label class="os-field">
        <span>Right</span>
        <select data-plan-diff-right>${rightOptions}</select>
      </label>
    </div>
    <div class="os-plan-diff">${diffRows || `<div class="os-empty">Select two revisions to compare</div>`}</div>
  `;
}

export {
  artifactKindLabels,
  artifactKindOrder,
  computeArtifactDiff,
  findArtifactById,
  findRevisionById,
  getArtifactRevisionOptions,
  validatePlanningWorkspace,
};
