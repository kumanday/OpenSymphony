import type { PlanningArtifactKind } from "@opensymphony/gateway-schema";
import { schemaVersionV1 } from "@opensymphony/gateway-schema";
import { generateId } from "./id.js";

export interface ConversationMessage {
  message_id: string;
  role: "user" | "assistant" | "system";
  body: string;
  happened_at: string;
}

export interface PlanningArtifactRevision {
  revision_id: string;
  created_at: string;
  content: string;
  generated_by?: string;
}

export interface PlanningArtifactWithRevisions {
  schema_version: { major: number; minor: number; patch: number };
  artifact_id: string;
  session_id: string;
  kind: PlanningArtifactKind;
  title: string;
  created_at: string;
  updated_at: string;
  approved: boolean;
  published_to_tracker: boolean;
  revisions: PlanningArtifactRevision[];
}

export interface PlanningNode {
  schema_version: { major: number; minor: number; patch: number };
  node_id: string;
  kind: "milestone" | "issue" | "sub_issue";
  identifier: string;
  title: string;
  state: string;
  state_category: string;
  parent_id?: string;
  children: string[];
  blocked_by: string[];
  comment_count?: number;
}

export interface PlanningValidationMessage {
  message_id: string;
  level: "error" | "warning" | "info";
  message: string;
  field_ref?: {
    kind: "artifact" | "node" | "criteria" | "verification" | "dependency";
    id: string;
    sub_id?: string;
  };
}

export type PlanningWorkspaceTab =
  | "conversation"
  | "artifact"
  | "hierarchy"
  | "dependencies"
  | "criteria"
  | "validation"
  | "diff";

export interface PlanningWorkspaceState {
  session_id: string;
  project_id: string;
  title: string;
  activeTab: PlanningWorkspaceTab;
  messages: ConversationMessage[];
  composerDraft: string;
  artifacts: PlanningArtifactWithRevisions[];
  selectedArtifactId: string | null;
  selectedRevisionId: string | null;
  diffLeftRevisionId: string | null;
  diffRightRevisionId: string | null;
  nodes: PlanningNode[];
  selectedNodeId: string | null;
  expandedNodeIds: Set<string>;
  criteria: { id: string; text: string; checked: boolean }[];
  verification: { id: string; text: string; checked: boolean }[];
  pendingMutations: Set<string>;
}

export function emptyPlanningWorkspaceState(): PlanningWorkspaceState {
  return {
    session_id: "",
    project_id: "",
    title: "",
    activeTab: "artifact",
    messages: [],
    composerDraft: "",
    artifacts: [],
    selectedArtifactId: null,
    selectedRevisionId: null,
    diffLeftRevisionId: null,
    diffRightRevisionId: null,
    nodes: [],
    selectedNodeId: null,
    expandedNodeIds: new Set(),
    criteria: [],
    verification: [],
    pendingMutations: new Set(),
  };
}

export const artifactKindLabels: Record<PlanningArtifactKind, string> = {
  intake: "Intake",
  requirements: "Requirements",
  research_summary: "Research",
  codebase_analysis: "Codebase Analysis",
  milestone_draft: "Milestone Draft",
  issue_draft: "Issue Draft",
  sub_issue_draft: "Sub-issue Draft",
  dependency_map: "Dependency Map",
  acceptance_criteria: "Acceptance Criteria",
  verification_plan: "Verification Plan",
};

export const artifactKindOrder: PlanningArtifactKind[] = [
  "intake",
  "requirements",
  "research_summary",
  "codebase_analysis",
  "milestone_draft",
  "issue_draft",
  "sub_issue_draft",
  "dependency_map",
  "acceptance_criteria",
  "verification_plan",
];

export function buildFixturePlanningWorkspaceState(
  projectId = "opensymphony-local",
  sessionId = "fixture-planning-session",
): PlanningWorkspaceState {
  const now = new Date().toISOString();
  const sv = schemaVersionV1();
  return {
    session_id: sessionId,
    project_id: projectId,
    title: "OpenSymphony Planning",
    activeTab: "artifact",
    messages: [
      {
        message_id: "msg-1",
        role: "assistant",
        body: "I've started a planning session for this project. Review the artifacts and hierarchy, then edit anything that looks off.",
        happened_at: now,
      },
    ],
    composerDraft: "",
    artifacts: [
      {
        schema_version: sv,
        artifact_id: "artifact-intake",
        session_id: sessionId,
        kind: "intake",
        title: "Project Intake",
        created_at: now,
        updated_at: now,
        approved: false,
        published_to_tracker: false,
        revisions: [
          {
            revision_id: "rev-intake-1",
            created_at: now,
            content: "Goal: build a collaborative planning workspace.\nTarget: OpenSymphony rich client.",
          },
        ],
      },
      {
        schema_version: sv,
        artifact_id: "artifact-research",
        session_id: sessionId,
        kind: "research_summary",
        title: "Research Summary",
        created_at: now,
        updated_at: now,
        approved: false,
        published_to_tracker: false,
        revisions: [
          {
            revision_id: "rev-research-1",
            created_at: now,
            content: "Existing OpenHands orchestrator provides runtime event streams. Linear GraphQL is used for tracker writes.",
          },
        ],
      },
      {
        schema_version: sv,
        artifact_id: "artifact-codebase",
        session_id: sessionId,
        kind: "codebase_analysis",
        title: "Codebase Analysis",
        created_at: now,
        updated_at: now,
        approved: false,
        published_to_tracker: false,
        revisions: [
          {
            revision_id: "rev-codebase-1",
            created_at: now,
            content: "Frontend is a shared TypeScript DOM UI in packages/ui-core. Gateway schema is the shared contract.",
          },
        ],
      },
      {
        schema_version: sv,
        artifact_id: "artifact-requirements",
        session_id: sessionId,
        kind: "requirements",
        title: "Requirements",
        created_at: now,
        updated_at: now,
        approved: false,
        published_to_tracker: false,
        revisions: [
          {
            revision_id: "rev-requirements-1",
            created_at: now,
            content: "Conversation pane.\nArtifact pane.\nHierarchy editor.\nDependency graph.\nValidation panel.\nDiff view.",
          },
          {
            revision_id: "rev-requirements-2",
            created_at: now,
            content: "Conversation pane.\nArtifact pane with revisions.\nHierarchy editor.\nDependency graph.\nValidation panel.\nDiff view.\nAcceptance criteria editor.",
          },
        ],
      },
      {
        schema_version: sv,
        artifact_id: "artifact-milestone",
        session_id: sessionId,
        kind: "milestone_draft",
        title: "Milestone Draft",
        created_at: now,
        updated_at: now,
        approved: false,
        published_to_tracker: false,
        revisions: [
          {
            revision_id: "rev-milestone-1",
            created_at: now,
            content: "M1: Collaborative Planning Alpha",
          },
        ],
      },
      {
        schema_version: sv,
        artifact_id: "artifact-empty",
        session_id: sessionId,
        kind: "intake",
        title: "Empty Intake",
        created_at: now,
        updated_at: now,
        approved: false,
        published_to_tracker: false,
        revisions: [
          {
            revision_id: "rev-empty-1",
            created_at: now,
            content: "",
          },
        ],
      },
    ],
    selectedArtifactId: "artifact-requirements",
    selectedRevisionId: "rev-requirements-2",
    diffLeftRevisionId: "rev-requirements-1",
    diffRightRevisionId: "rev-requirements-2",
    nodes: [
      {
        schema_version: sv,
        node_id: "plan-milestone",
        kind: "milestone",
        identifier: "M1",
        title: "Collaborative Planning Alpha",
        state: "In Progress",
        state_category: "in_progress",
        children: ["plan-issue-1"],
        blocked_by: [],
      },
      {
        schema_version: sv,
        node_id: "plan-issue-1",
        kind: "issue",
        identifier: "COE-417",
        title: "Planning Workspace UI",
        state: "In Progress",
        state_category: "in_progress",
        parent_id: "plan-milestone",
        children: ["plan-sub-1"],
        blocked_by: [],
      },
      {
        schema_version: sv,
        node_id: "plan-sub-1",
        kind: "sub_issue",
        identifier: "COE-417-1",
        title: "Implement conversation pane",
        state: "Todo",
        state_category: "todo",
        parent_id: "plan-issue-1",
        children: [],
        blocked_by: [],
      },
    ],
    selectedNodeId: "plan-issue-1",
    expandedNodeIds: new Set(["plan-milestone", "plan-issue-1"]),
    criteria: [
      { id: "crit-1", text: "Users can review and edit all planning artifacts.", checked: false },
      { id: "crit-2", text: "Users can view diffs between artifact revisions.", checked: false },
      { id: "crit-3", text: "Validation messages link to the fields that need review.", checked: false },
    ],
    verification: [
      { id: "ver-1", text: "Run planning UI component tests with fixture sessions.", checked: false },
      { id: "ver-2", text: "Run keyboard navigation and focus checks.", checked: false },
    ],
    pendingMutations: new Set(),
  };
}

export function addMessage(
  state: PlanningWorkspaceState,
  role: ConversationMessage["role"],
  body: string,
): PlanningWorkspaceState {
  return {
    ...state,
    composerDraft: role === "user" ? "" : state.composerDraft,
    messages: [
      ...state.messages,
      {
        message_id: `msg-${generateId()}`,
        role,
        body: body.trim(),
        happened_at: new Date().toISOString(),
      },
    ],
  };
}

export function updateArtifactContent(
  state: PlanningWorkspaceState,
  artifactId: string,
  content: string,
): PlanningWorkspaceState {
  const trimmed = content.trim();
  let selectedRevisionId: string | null = state.selectedRevisionId;
  const artifacts = state.artifacts.map((artifact) => {
    if (artifact.artifact_id !== artifactId) return artifact;
    const latest = artifact.revisions[artifact.revisions.length - 1];
    if (latest && latest.content === trimmed) {
      selectedRevisionId = latest.revision_id;
      return artifact;
    }
    const now = new Date().toISOString();
    const newRevision: PlanningArtifactRevision = {
      revision_id: `rev-${artifact.kind}-${artifact.revisions.length + 1}-${generateId()}`,
      created_at: now,
      content: trimmed,
    };
    selectedRevisionId = newRevision.revision_id;
    return {
      ...artifact,
      updated_at: now,
      revisions: [...artifact.revisions, newRevision],
    };
  });
  return {
    ...state,
    artifacts,
    selectedRevisionId,
  };
}

export function selectArtifact(
  state: PlanningWorkspaceState,
  artifactId: string | null,
): PlanningWorkspaceState {
  const artifact = state.artifacts.find((a) => a.artifact_id === artifactId) ?? null;
  const revisionId = artifact?.revisions[artifact.revisions.length - 1]?.revision_id ?? null;
  return {
    ...state,
    selectedArtifactId: artifactId,
    selectedRevisionId: revisionId,
  };
}

export function selectRevision(
  state: PlanningWorkspaceState,
  revisionId: string | null,
): PlanningWorkspaceState {
  return { ...state, selectedRevisionId: revisionId };
}

export function addPlanningNode(
  state: PlanningWorkspaceState,
  kind: PlanningNode["kind"],
  parentId: string | null,
  title: string,
  stateName = "Todo",
): PlanningWorkspaceState {
  const nodeId = `plan-${kind}-${generateId()}`;
  const identifier = nodeId;
  const newNode: PlanningNode = {
    schema_version: schemaVersionV1(),
    node_id: nodeId,
    kind,
    identifier,
    title: title.trim(),
    state: stateName,
    state_category: stateToCategory(stateName),
    parent_id: parentId ?? undefined,
    children: [],
    blocked_by: [],
  };
  const nodes = [...state.nodes, newNode];
  if (parentId) {
    const parentIdx = nodes.findIndex((n) => n.node_id === parentId);
    if (parentIdx >= 0) {
      const parent = nodes[parentIdx];
      nodes[parentIdx] = { ...parent, children: [...parent.children, nodeId] };
    }
  }
  return { ...state, nodes: rebuildChildren(nodes), selectedNodeId: nodeId, expandedNodeIds: new Set(state.expandedNodeIds).add(nodeId) };
}

export function updatePlanningNode(
  state: PlanningWorkspaceState,
  nodeId: string,
  changes: Partial<Pick<PlanningNode, "title" | "state">>,
): PlanningWorkspaceState {
  const nodes = state.nodes.map((node) => {
    if (node.node_id !== nodeId) return node;
    const updated: PlanningNode = { ...node };
    if (changes.title !== undefined) updated.title = changes.title.trim();
    if (changes.state !== undefined) {
      updated.state = changes.state;
      updated.state_category = stateToCategory(changes.state);
    }
    return updated;
  });
  return { ...state, nodes };
}

export function updateNodeDependencies(
  state: PlanningWorkspaceState,
  nodeId: string,
  blockedBy: string[],
): PlanningWorkspaceState {
  const nodes = state.nodes.map((node) =>
    node.node_id === nodeId ? { ...node, blocked_by: blockedBy } : node,
  );
  return { ...state, nodes };
}

export function removePlanningNode(
  state: PlanningWorkspaceState,
  nodeId: string,
): PlanningWorkspaceState {
  const node = state.nodes.find((n) => n.node_id === nodeId);
  if (!node) return state;
  const descendants = collectDescendants(state.nodes, nodeId);
  const idsToRemove = new Set([nodeId, ...descendants]);
  const nodes = state.nodes
    .filter((n) => !idsToRemove.has(n.node_id))
    .map((n) => ({
      ...n,
      children: n.children.filter((id) => !idsToRemove.has(id)),
      blocked_by: n.blocked_by.filter((id) => !idsToRemove.has(id)),
    }));
  return {
    ...state,
    nodes: rebuildChildren(nodes),
    selectedNodeId: state.selectedNodeId === nodeId ? null : state.selectedNodeId,
  };
}

export function toggleNodeExpanded(
  state: PlanningWorkspaceState,
  nodeId: string,
): PlanningWorkspaceState {
  const expanded = new Set(state.expandedNodeIds);
  if (expanded.has(nodeId)) expanded.delete(nodeId);
  else expanded.add(nodeId);
  return { ...state, expandedNodeIds: expanded };
}

export function addCriterion(
  state: PlanningWorkspaceState,
  text: string,
): PlanningWorkspaceState {
  return {
    ...state,
    criteria: [
      ...state.criteria,
      { id: `crit-${generateId()}`, text: text.trim(), checked: false },
    ],
  };
}

export function updateCriterion(
  state: PlanningWorkspaceState,
  id: string,
  text: string,
): PlanningWorkspaceState {
  return {
    ...state,
    criteria: state.criteria.map((c) => (c.id === id ? { ...c, text: text.trim() } : c)),
  };
}

export function toggleCriterion(
  state: PlanningWorkspaceState,
  id: string,
): PlanningWorkspaceState {
  return {
    ...state,
    criteria: state.criteria.map((c) => (c.id === id ? { ...c, checked: !c.checked } : c)),
  };
}

export function removeCriterion(
  state: PlanningWorkspaceState,
  id: string,
): PlanningWorkspaceState {
  return { ...state, criteria: state.criteria.filter((c) => c.id !== id) };
}

export function addVerification(
  state: PlanningWorkspaceState,
  text: string,
): PlanningWorkspaceState {
  return {
    ...state,
    verification: [
      ...state.verification,
      { id: `ver-${generateId()}`, text: text.trim(), checked: false },
    ],
  };
}

export function updateVerification(
  state: PlanningWorkspaceState,
  id: string,
  text: string,
): PlanningWorkspaceState {
  return {
    ...state,
    verification: state.verification.map((v) => (v.id === id ? { ...v, text: text.trim() } : v)),
  };
}

export function toggleVerification(
  state: PlanningWorkspaceState,
  id: string,
): PlanningWorkspaceState {
  return {
    ...state,
    verification: state.verification.map((v) => (v.id === id ? { ...v, checked: !v.checked } : v)),
  };
}

export function removeVerification(
  state: PlanningWorkspaceState,
  id: string,
): PlanningWorkspaceState {
  return { ...state, verification: state.verification.filter((v) => v.id !== id) };
}

export function validatePlanningWorkspace(
  state: PlanningWorkspaceState,
): PlanningValidationMessage[] {
  const messages: PlanningValidationMessage[] = [];

  for (const artifact of state.artifacts) {
    const latest = artifact.revisions[artifact.revisions.length - 1];
    if (!latest || latest.content.trim().length === 0) {
      messages.push({
        message_id: `val-${artifact.artifact_id}-empty`,
        level: "warning",
        message: `${artifactKindLabels[artifact.kind]} is empty.`,
        field_ref: { kind: "artifact", id: artifact.artifact_id },
      });
    }
  }

  const nodeMap = new Map(state.nodes.map((n) => [n.node_id, n]));
  for (const node of state.nodes) {
    for (const depId of node.blocked_by) {
      if (!nodeMap.has(depId)) {
        messages.push({
          message_id: `val-${node.node_id}-dangling-${depId}`,
          level: "error",
          message: `Dependency ${node.identifier} references unknown node ${depId}.`,
          field_ref: { kind: "dependency", id: node.node_id },
        });
      }
    }
  }

  const reportedCycleNodes = new Set<string>();
  for (const node of state.nodes) {
    if (reportedCycleNodes.has(node.node_id)) continue;
    const cycle = findDependencyCycle(nodeMap, node.node_id);
    if (cycle) {
      for (const id of cycle) reportedCycleNodes.add(id);
      const identifiers = cycle.map((id) => nodeMap.get(id)?.identifier ?? id);
      messages.push({
        message_id: `val-${cycle[0]}-cycle`,
        level: "error",
        message: "Dependency cycle: " + identifiers.join(", "),
        field_ref: { kind: "dependency", id: cycle[0] },
      });
    }
  }

  if (state.criteria.length === 0) {
    messages.push({
      message_id: `val-global-criteria`,
      level: "warning",
      message: "No plan-level acceptance criteria defined.",
      field_ref: { kind: "criteria", id: "plan-criteria" },
    });
  }
  if (state.verification.length === 0) {
    messages.push({
      message_id: `val-global-verification`,
      level: "warning",
      message: "No plan-level verification expectations defined.",
      field_ref: { kind: "verification", id: "plan-verification" },
    });
  }

  return messages;
}

export interface DiffLine {
  kind: "added" | "removed" | "unchanged";
  line: string;
  leftLineNumber?: number;
  rightLineNumber?: number;
}

export function computeArtifactDiff(
  left: string,
  right: string,
): DiffLine[] {
  const leftLines = left.split("\n");
  const rightLines = right.split("\n");
  const lcs = longestCommonSubsequence(leftLines, rightLines);
  let i = 0;
  let j = 0;
  let lcsIndex = 0;
  const result: DiffLine[] = [];
  let leftLineNumber = 0;
  let rightLineNumber = 0;

  while (i < leftLines.length || j < rightLines.length) {
    if (lcsIndex < lcs.length && leftLines[i] === lcs[lcsIndex] && rightLines[j] === lcs[lcsIndex]) {
      result.push({ kind: "unchanged", line: leftLines[i], leftLineNumber: ++leftLineNumber, rightLineNumber: ++rightLineNumber });
      i++;
      j++;
      lcsIndex++;
    } else if (j < rightLines.length && (lcsIndex >= lcs.length || rightLines[j] !== lcs[lcsIndex])) {
      result.push({ kind: "added", line: rightLines[j], rightLineNumber: ++rightLineNumber });
      j++;
    } else if (i < leftLines.length) {
      result.push({ kind: "removed", line: leftLines[i], leftLineNumber: ++leftLineNumber });
      i++;
    } else {
      // Defensive: one of the branches above should have matched.
      break;
    }
  }
  return result;
}

export function findArtifactById(
  state: PlanningWorkspaceState,
  artifactId: string | null,
): PlanningArtifactWithRevisions | undefined {
  return state.artifacts.find((a) => a.artifact_id === artifactId);
}

export function findRevisionById(
  artifact: PlanningArtifactWithRevisions | undefined,
  revisionId: string | null,
): PlanningArtifactRevision | undefined {
  if (!artifact || !revisionId) return undefined;
  return artifact.revisions.find((r) => r.revision_id === revisionId);
}

export function getArtifactRevisionOptions(
  artifact: PlanningArtifactWithRevisions | undefined,
): Array<{ revision_id: string; label: string; index: number }> {
  if (!artifact) return [];
  return artifact.revisions.map((r, index) => ({
    revision_id: r.revision_id,
    label: `Revision ${index + 1} — ${new Date(r.created_at).toLocaleString()}`,
    index: index + 1,
  }));
}

export function getArtifactDepths(nodes: PlanningNode[]): Map<string, number> {
  const depths = new Map<string, number>();
  const rootIds = nodes.filter((n) => !n.parent_id).map((n) => n.node_id);
  function walk(nodeId: string, depth: number): void {
    const existing = depths.get(nodeId);
    if (existing !== undefined && existing <= depth) return;
    depths.set(nodeId, depth);
    const node = nodes.find((n) => n.node_id === nodeId);
    if (node) {
      for (const child of node.children) {
        walk(child, depth + 1);
      }
    }
  }
  for (const rootId of rootIds) walk(rootId, 0);
  for (const node of nodes) {
    if (!depths.has(node.node_id)) depths.set(node.node_id, 0);
  }
  return depths;
}

function findDependencyCycle(
  nodeMap: Map<string, PlanningNode>,
  startId: string,
): string[] | null {
  const path: string[] = [];
  const pathIndex = new Map<string, number>();
  const visited = new Set<string>();
  function dfs(nodeId: string): string[] | null {
    if (pathIndex.has(nodeId)) {
      const idx = pathIndex.get(nodeId)!;
      return path.slice(idx);
    }
    if (visited.has(nodeId)) return null;
    visited.add(nodeId);
    pathIndex.set(nodeId, path.length);
    path.push(nodeId);
    const node = nodeMap.get(nodeId);
    if (node) {
      for (const dep of node.blocked_by) {
        const cycle = dfs(dep);
        if (cycle) return cycle;
      }
    }
    path.pop();
    pathIndex.delete(nodeId);
    return null;
  }
  return dfs(startId);
}

export function hasDependencyCycle(
  nodeMap: Map<string, PlanningNode>,
  startId: string,
): boolean {
  return findDependencyCycle(nodeMap, startId) !== null;
}

function rebuildChildren(nodes: PlanningNode[]): PlanningNode[] {
  const childrenByParent = new Map<string, string[]>();
  for (const node of nodes) {
    if (node.parent_id) {
      const list = childrenByParent.get(node.parent_id) ?? [];
      list.push(node.node_id);
      childrenByParent.set(node.parent_id, list);
    }
  }
  return nodes.map((node) => {
    const children = childrenByParent.get(node.node_id);
    if (children === undefined) return node;
    return { ...node, children };
  });
}

function collectDescendants(nodes: PlanningNode[], nodeId: string): string[] {
  const result: string[] = [];
  const node = nodes.find((n) => n.node_id === nodeId);
  if (!node) return result;
  for (const child of node.children) {
    result.push(child);
    result.push(...collectDescendants(nodes, child));
  }
  return result;
}

function stateToCategory(state: string): PlanningNode["state_category"] {
  const lower = state.toLowerCase();
  if (lower === "done" || lower === "completed") return "done";
  if (lower === "in progress" || lower === "in_progress" || lower === "started") return "in_progress";
  if (lower === "canceled" || lower === "cancelled") return "canceled";
  if (lower === "backlog") return "backlog";
  return "todo";
}

function longestCommonSubsequence(a: string[], b: string[]): string[] {
  const dp: number[][] = Array.from({ length: a.length + 1 }, () => Array(b.length + 1).fill(0));
  for (let i = 1; i <= a.length; i++) {
    for (let j = 1; j <= b.length; j++) {
      if (a[i - 1] === b[j - 1]) {
        dp[i][j] = dp[i - 1][j - 1] + 1;
      } else {
        dp[i][j] = Math.max(dp[i - 1][j], dp[i][j - 1]);
      }
    }
  }
  const result: string[] = [];
  let i = a.length;
  let j = b.length;
  while (i > 0 && j > 0) {
    if (a[i - 1] === b[j - 1]) {
      result.unshift(a[i - 1]);
      i--;
      j--;
    } else if (dp[i - 1][j] > dp[i][j - 1]) {
      i--;
    } else {
      j--;
    }
  }
  return result;
}
