import type {
  TaskGraphNode,
  TaskGraphNodeKind,
  TaskGraphSnapshot,
  RunDetail,
  TaskGraphRuntimeOverlay,
  RuntimeBadgeKind,
  TaskGraphStateCategory,
} from "@opensymphony/gateway-schema";

export type TaskGraphFilterKind = TaskGraphNodeKind | "all";
export type TaskGraphFilterRuntime = RuntimeBadgeKind | "all";

export interface TaskGraphFilter {
  kind: TaskGraphFilterKind;
  runtime: TaskGraphFilterRuntime;
  stateCategory: TaskGraphStateCategory | "all";
  search: string;
}

export const defaultTaskGraphFilter: TaskGraphFilter = {
  kind: "all",
  runtime: "all",
  stateCategory: "all",
  search: "",
};

/** Runtime overlay state derived from a run detail and node relationships. */
export function buildRuntimeOverlay(
  node: TaskGraphNode,
  runDetail?: RunDetail,
): TaskGraphRuntimeOverlay {
  const badges: RuntimeBadgeKind[] = [];
  const status = runDetail?.status;
  const releaseReason = runDetail?.release_reason;
  const isStale = runDetail?.liveness?.stream === "stale";
  const isBlocked = node.blocked_by.length > 0;

  if (isBlocked) badges.push("blocker");
  if (isStale) badges.push("stale");

  if (status === "running" || status === "claimed") {
    badges.push("running");
  } else if (status === "retry_queued") {
    badges.push("retry");
    badges.push("queued");
  } else if (status === "released") {
    if (releaseReason === "completed" || releaseReason === "tracker_terminal") {
      badges.push("complete");
    } else {
      badges.push("failed");
    }
  }

  if (runDetail?.workspace_path) badges.push("workspace");
  if (runDetail?.worker_id) badges.push("harness");
  if (runDetail?.retry_attempt && runDetail.retry_attempt > 0 && !badges.includes("retry")) {
    badges.push("retry");
  }

  // Validation status is inferred from run outcome when not explicit.
  let validationStatus: TaskGraphRuntimeOverlay["validation_status"] = "unknown";
  if (status === "released" && releaseReason === "completed") {
    validationStatus = "passed";
  } else if (status === "released" && releaseReason && releaseReason !== "completed" && releaseReason !== "tracker_terminal") {
    validationStatus = "failed";
  }
  if (validationStatus !== "unknown") badges.push("validation");

  return {
    schema_version: { major: 1, minor: 0, patch: 0 },
    node_id: node.node_id,
    run_id: runDetail?.run_id,
    status,
    release_reason: releaseReason,
    phase: runDetail?.liveness?.phase,
    is_stale: isStale,
    is_blocked: isBlocked,
    workspace_path: runDetail?.workspace_path,
    harness: runDetail?.worker_id,
    diff_summary: undefined,
    validation_status: validationStatus,
    retry_attempt: runDetail?.retry_attempt,
    blocked_by_count: node.blocked_by.length,
    last_event_at: runDetail?.liveness?.latest_progress?.happened_at,
    badges: [...new Set(badges)],
  };
}

/** Apply kind, runtime badge, state category, and text filters to task graph nodes. */
export function filterTaskGraphNodes(
  nodes: TaskGraphNode[],
  filter: TaskGraphFilter,
  getOverlay: (node: TaskGraphNode) => TaskGraphRuntimeOverlay,
): TaskGraphNode[] {
  return nodes.filter((node) => {
    if (filter.kind !== "all" && node.kind !== filter.kind) return false;
    if (filter.stateCategory !== "all" && node.state_category !== filter.stateCategory) return false;
    if (filter.runtime !== "all") {
      const overlay = getOverlay(node);
      if (!overlay.badges.includes(filter.runtime)) return false;
    }
    if (filter.search) {
      const term = filter.search.toLowerCase();
      const haystack = `${node.identifier} ${node.title} ${node.state} ${node.labels.join(" ")}`.toLowerCase();
      if (!haystack.includes(term)) return false;
    }
    return true;
  });
}

/** Render a runtime badge pill. */
export function renderBadge(kind: RuntimeBadgeKind): string {
  return `<span class="os-badge os-badge-${kind}">${kind.replace(/_/g, " ")}</span>`;
}

/** Render the filter bar for the task graph editor. */
export function renderTaskGraphFilters(filter: TaskGraphFilter): string {
  const kindOptions = [
    { value: "all", label: "All kinds" },
    { value: "milestone", label: "Milestone" },
    { value: "issue", label: "Issue" },
    { value: "sub_issue", label: "Sub-issue" },
  ]
    .map((opt) => `<option value="${opt.value}" ${filter.kind === opt.value ? "selected" : ""}>${opt.label}</option>`)
    .join("");

  const runtimeOptions = [
    { value: "all", label: "All runtime" },
    { value: "running", label: "Running" },
    { value: "queued", label: "Queued" },
    { value: "complete", label: "Complete" },
    { value: "failed", label: "Failed" },
    { value: "stale", label: "Stale" },
    { value: "blocker", label: "Blocker" },
    { value: "retry", label: "Retry" },
    { value: "workspace", label: "Workspace" },
    { value: "harness", label: "Harness" },
    { value: "validation", label: "Validation" },
  ]
    .map((opt) => `<option value="${opt.value}" ${filter.runtime === opt.value ? "selected" : ""}>${opt.label}</option>`)
    .join("");

  const stateOptions = [
    { value: "all", label: "All states" },
    { value: "backlog", label: "Backlog" },
    { value: "todo", label: "Todo" },
    { value: "in_progress", label: "In Progress" },
    { value: "done", label: "Done" },
    { value: "canceled", label: "Canceled" },
  ]
    .map((opt) => `<option value="${opt.value}" ${filter.stateCategory === opt.value ? "selected" : ""}>${opt.label}</option>`)
    .join("");

  return `
    <div class="os-filter-bar">
      <label class="os-field">
        <span>Kind</span>
        <select data-tg-filter="kind">${kindOptions}</select>
      </label>
      <label class="os-field">
        <span>Runtime</span>
        <select data-tg-filter="runtime">${runtimeOptions}</select>
      </label>
      <label class="os-field">
        <span>State</span>
        <select data-tg-filter="state">${stateOptions}</select>
      </label>
      <label class="os-field">
        <span>Search</span>
        <input data-tg-filter="search" type="search" value="${escapeAttr(filter.search)}" placeholder="Filter tasks..." />
      </label>
      <button type="button" data-tg-filter-reset>Reset</button>
    </div>
  `;
}

function escapeHtml(value: unknown): string {
  return String(value)
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}

function escapeAttr(value: unknown): string {
  return escapeHtml(value).replace(/"/g, "&quot;");
}

export { escapeHtml, escapeAttr };
