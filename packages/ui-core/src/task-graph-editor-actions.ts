import type {
  ActionDispatch,
  ActionReceipt,
  TaskGraphCommentPayload,
  TaskGraphCreatePayload,
  TaskGraphDependencyPayload,
  TaskGraphNode,
  TaskGraphUpdatePayload,
} from "@opensymphony/gateway-schema";
import { generateId } from "./id.js";

export interface ActionCapableTransport {
  dispatchAction(action: ActionDispatch): Promise<ActionReceipt>;
}

export function isActionCapable(transport: unknown): transport is ActionCapableTransport {
  return (
    typeof transport === "object" &&
    transport !== null &&
    "dispatchAction" in transport &&
    typeof (transport as ActionCapableTransport).dispatchAction === "function"
  );
}

export async function dispatchTaskGraphUpdate(
  transport: ActionCapableTransport,
  payload: TaskGraphUpdatePayload,
  correlationId?: string,
): Promise<ActionReceipt> {
  return transport.dispatchAction({
    schema_version: { major: 1, minor: 0, patch: 0 },
    correlation_id: correlationId ?? `tg-update-${payload.node_id}-${generateId()}`,
    action_kind: "update_node",
    target_entity: { entity_kind: "issue", entity_id: payload.node_id },
    payload,
    idempotency_key: `tg-update-${payload.node_id}`,
  });
}

export async function dispatchTaskGraphCreate(
  transport: ActionCapableTransport,
  payload: TaskGraphCreatePayload,
  correlationId?: string,
): Promise<ActionReceipt> {
  const targetId = payload.parent_id ?? "root";
  return transport.dispatchAction({
    schema_version: { major: 1, minor: 0, patch: 0 },
    correlation_id: correlationId ?? `tg-create-${targetId}-${payload.kind}-${generateId()}`,
    action_kind: "create_followup",
    target_entity: { entity_kind: "issue", entity_id: targetId },
    payload,
    idempotency_key: `tg-create-${targetId}-${payload.kind}-${payload.title}`,
  });
}

export async function dispatchTaskGraphDependencies(
  transport: ActionCapableTransport,
  payload: TaskGraphDependencyPayload,
  correlationId?: string,
): Promise<ActionReceipt> {
  return transport.dispatchAction({
    schema_version: { major: 1, minor: 0, patch: 0 },
    correlation_id: correlationId ?? `tg-deps-${payload.node_id}-${generateId()}`,
    action_kind: "transition_issue",
    target_entity: { entity_kind: "issue", entity_id: payload.node_id },
    payload,
    idempotency_key: `tg-deps-${payload.node_id}`,
  });
}

export async function dispatchTaskGraphComment(
  transport: ActionCapableTransport,
  payload: TaskGraphCommentPayload,
  correlationId?: string,
): Promise<ActionReceipt> {
  return transport.dispatchAction({
    schema_version: { major: 1, minor: 0, patch: 0 },
    correlation_id: correlationId ?? `tg-comment-${payload.node_id}-${generateId()}`,
    action_kind: "comment",
    target_entity: { entity_kind: "issue", entity_id: payload.node_id },
    payload,
    idempotency_key: `tg-comment-${payload.node_id}-${payload.body.slice(0, 40)}`,
  });
}

/** Build a partial task graph node update from the current node and editable fields. */
export function applyNodeUpdate(
  node: TaskGraphNode,
  changes: Partial<TaskGraphUpdatePayload>,
): TaskGraphNode {
  return {
    ...node,
    title: changes.title ?? node.title,
    state: changes.state ?? node.state,
    priority: changes.priority ?? node.priority,
    estimate_minutes: changes.estimate_minutes ?? node.estimate_minutes,
    labels: changes.labels ?? node.labels,
    updated_at: new Date().toISOString(),
  };
}

/** Build a new task graph node from a create payload and a generated id. */
export function buildCreatedNode(
  payload: TaskGraphCreatePayload,
  nodeId: string,
): TaskGraphNode {
  return {
    schema_version: { major: 1, minor: 0, patch: 0 },
    node_id: nodeId,
    kind: payload.kind,
    identifier: payload.identifier ?? nodeId,
    title: payload.title,
    state: payload.state ?? "Todo",
    state_category: stateToCategory(payload.state ?? "Todo"),
    parent_id: payload.parent_id ?? undefined,
    children: [],
    blocked_by: [],
    labels: payload.labels ?? [],
    priority: payload.priority,
    estimate_minutes: payload.estimate_minutes,
    created_at: new Date().toISOString(),
    updated_at: new Date().toISOString(),
  };
}

function stateToCategory(state: string): TaskGraphNode["state_category"] {
  const lower = state.toLowerCase();
  if (lower === "done" || lower === "completed") return "done";
  if (lower === "in progress" || lower === "in_progress" || lower === "started") return "in_progress";
  if (lower === "canceled" || lower === "cancelled") return "canceled";
  if (lower === "todo" || lower === "backlog") return lower === "todo" ? "todo" : "backlog";
  return "todo";
}
