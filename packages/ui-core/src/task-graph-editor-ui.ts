import type { TaskGraphNode, TaskGraphNodeKind, TaskGraphRuntimeOverlay } from "@opensymphony/gateway-schema";
import { escapeHtml, escapeAttr, renderBadge } from "./task-graph-editor.js";
import type { TaskGraphFilter } from "./task-graph-editor.js";

export interface EditorDialogState {
  open: boolean;
  kind: TaskGraphNodeKind | null;
  parentId: string | null;
  draftTitle: string;
  draftState: string;
}

export const emptyEditorDialog: EditorDialogState = {
  open: false,
  kind: null,
  parentId: null,
  draftTitle: "",
  draftState: "Todo",
};

export interface InlineEditState {
  nodeId: string | null;
  title: string;
  state: string;
}

export const emptyInlineEdit: InlineEditState = {
  nodeId: null,
  title: "",
  state: "",
};

export interface DependencyEditState {
  nodeId: string | null;
  blockedBy: string[];
}

export const emptyDependencyEdit: DependencyEditState = {
  nodeId: null,
  blockedBy: [],
};

export interface CommentEditState {
  nodeId: string | null;
  kind: "comment" | "evidence";
  body: string;
}

export const emptyCommentEdit: CommentEditState = {
  nodeId: null,
  kind: "comment",
  body: "",
};

/** Render a single task graph node row with optional inline editing. */
export function renderTaskGraphNode(
  node: TaskGraphNode,
  selectedNodeId: string | null,
  inlineEdit: InlineEditState,
  overlay?: TaskGraphRuntimeOverlay,
): string {
  const isSelected = node.node_id === selectedNodeId;
  const isEditing = inlineEdit.nodeId === node.node_id;
  const overlayBadges = overlay?.badges.length ? overlay.badges.map(renderBadge).join("") : "";
  const runMeta = overlay?.run_id
    ? `<span class="os-run-meta">run ${escapeHtml(overlay.run_id)}</span>`
    : "";

  const titleContent = isEditing
    ? `<input class="os-inline-input" data-tg-inline-title="${escapeAttr(node.node_id)}" value="${escapeAttr(inlineEdit.title)}" />`
    : `<strong>${escapeHtml(node.identifier)}</strong>`;

  const titleDisplay = isEditing ? "" : `<span>${escapeHtml(node.title)}</span>`;

  const stateContent = isEditing
    ? `<input class="os-inline-input os-inline-state" data-tg-inline-state="${escapeAttr(node.node_id)}" value="${escapeAttr(inlineEdit.state)}" />`
    : `<em>${escapeHtml(node.state)}</em>`;

  const editButtons = isEditing
    ? `<button type="button" data-tg-inline-save="${escapeAttr(node.node_id)}">Save</button><button type="button" data-tg-inline-cancel="${escapeAttr(node.node_id)}">Cancel</button>`
    : `<button type="button" data-tg-edit="${escapeAttr(node.node_id)}">Edit</button>`;
  const commentLabel = node.comment_count ? `Comment (${node.comment_count})` : "Comment";

  return `
    <div class="os-node ${isSelected ? "is-selected" : ""}" data-node-id="${escapeAttr(node.node_id)}">
      <span class="os-node-kind">${escapeHtml(node.kind.replace(/_/g, " "))}</span>
      ${titleContent}
      ${titleDisplay}
      ${stateContent}
      <div class="os-node-badges">${overlayBadges}</div>
      ${runMeta}
      <div class="os-node-actions">${editButtons}
        <button type="button" data-tg-deps="${escapeAttr(node.node_id)}">Deps</button>
        <button type="button" data-tg-comment="${escapeAttr(node.node_id)}">${escapeHtml(commentLabel)}</button>
        <button type="button" data-tg-create-child="${escapeAttr(node.node_id)}">+</button>
      </div>
    </div>
  `;
}

/** Render the selected node detail strip and actions. */
export function renderSelectedNodeDetail(node: TaskGraphNode | undefined): string {
  if (!node) return "";
  return `
    <div class="os-detail-strip">
      <strong>${escapeHtml(node.identifier)}</strong>
      <span>${escapeHtml(node.title)}</span>
      <button type="button" data-open-run="${escapeAttr(node.node_id)}">Open Run</button>
    </div>
  `;
}

/** Render the create dialog for milestones/issues/sub-issues. */
export function renderCreateDialog(dialog: EditorDialogState): string {
  if (!dialog.open || !dialog.kind) return "";
  const title = `Create ${dialog.kind.replace(/_/g, " ")}`;
  return `
    <div class="os-dialog-backdrop" data-tg-create-dialog="open">
      <div class="os-dialog">
        <div class="os-section-head"><h2>${escapeHtml(title)}</h2></div>
        <label class="os-field">
          <span>Title</span>
          <input data-tg-create-title value="${escapeAttr(dialog.draftTitle)}" />
        </label>
        <label class="os-field">
          <span>State</span>
          <input data-tg-create-state value="${escapeAttr(dialog.draftState)}" />
        </label>
        <div class="os-dialog-actions">
          <button type="button" data-tg-create-save>Save</button>
          <button type="button" data-tg-create-cancel>Cancel</button>
        </div>
      </div>
    </div>
  `;
}

/** Render the dependency editor for a selected node. */
export function renderDependencyEditor(
  node: TaskGraphNode,
  allNodes: Map<string, TaskGraphNode>,
  editState: DependencyEditState,
): string {
  const options = Array.from(allNodes.values())
    .filter((candidate) => candidate.node_id !== node.node_id)
    .map((candidate) => {
      const selected = editState.blockedBy.includes(candidate.node_id) ? "selected" : "";
      return `<option value="${escapeAttr(candidate.node_id)}" ${selected}>${escapeHtml(candidate.identifier)} — ${escapeHtml(candidate.title)}</option>`;
    })
    .join("");

  return `
    <div class="os-dialog-backdrop" data-tg-deps-dialog="open">
      <div class="os-dialog">
        <div class="os-section-head"><h2>Dependencies for ${escapeHtml(node.identifier)}</h2></div>
        <label class="os-field">
          <span>Blocked by (multi-select)</span>
          <select data-tg-deps-select multiple size="6">${options}</select>
        </label>
        <div class="os-dialog-actions">
          <button type="button" data-tg-deps-save>Save</button>
          <button type="button" data-tg-deps-cancel>Cancel</button>
        </div>
      </div>
    </div>
  `;
}

/** Render the comment / evidence editor for a selected node. */
export function renderCommentEditor(node: TaskGraphNode, editState: CommentEditState): string {
  return `
    <div class="os-dialog-backdrop" data-tg-comment-dialog="open">
      <div class="os-dialog">
        <div class="os-section-head"><h2>Comment on ${escapeHtml(node.identifier)}</h2></div>
        <label class="os-field">
          <span>Kind</span>
          <select data-tg-comment-kind>
            <option value="comment" ${editState.kind === "comment" ? "selected" : ""}>Comment</option>
            <option value="evidence" ${editState.kind === "evidence" ? "selected" : ""}>Evidence</option>
          </select>
        </label>
        <label class="os-field">
          <span>Body</span>
          <textarea data-tg-comment-body rows="4">${escapeHtml(editState.body)}</textarea>
        </label>
        <div class="os-dialog-actions">
          <button type="button" data-tg-comment-save>Save</button>
          <button type="button" data-tg-comment-cancel>Cancel</button>
        </div>
      </div>
    </div>
  `;
}

/** Render the toolbar for creating top-level task graph entities. */
export function renderTaskGraphToolbar(): string {
  return `
    <div class="os-tg-toolbar">
      <button type="button" data-tg-create="milestone">+ Milestone</button>
      <button type="button" data-tg-create="issue">+ Issue</button>
    </div>
  `;
}
