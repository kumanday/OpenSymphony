/**
 * @jest-environment jsdom
 *
 * Task graph editor and runtime overlay tests for COE-411.
 */

import { renderOpenSymphonyApp } from "../src/app-shell.js";
import { MockGatewayTransport } from "@opensymphony/api-client";
import { schemaVersionV1 } from "@opensymphony/gateway-schema";
import { buildRuntimeOverlay } from "../src/task-graph-editor.js";
import type {
  DashboardSnapshot,
  GatewayCapabilities,
  RunDetail,
  TaskGraphNode,
  TaskGraphSnapshot,
} from "@opensymphony/gateway-schema";

const capabilities: GatewayCapabilities = {
  schema_version: schemaVersionV1(),
  gateway_version: "editor-test",
  supported_api_versions: ["1.0.0"],
  transports: [
    {
      transport: "loopback_http",
      modes: ["json"],
      supported_encodings: ["utf-8"],
      bidirectional: false,
    },
  ],
  features: [
    { feature: "task_graph", available: true, requires_auth: false },
    { feature: "actions", available: true, requires_auth: false },
  ],
  auth_modes: ["none"],
  max_event_page_size: 1000,
  max_terminal_frame_batch: 500,
};

const dashboard: DashboardSnapshot = {
  schema_version: schemaVersionV1(),
  generated_at: "2025-09-01T00:00:00Z",
  sequence: 1,
  health: "healthy",
  metrics: {
    running_issue_count: 1,
    retry_queue_depth: 0,
    total_input_tokens: 1000,
    total_output_tokens: 500,
    total_cache_read_tokens: 100,
    total_cost_micros: 0,
  },
  projects: [
    {
      project_id: "proj-editor",
      name: "Editor Test Project",
      milestone_count: 1,
      issue_count: 2,
      running_count: 1,
      completed_count: 1,
      failed_count: 0,
    },
  ],
  recent_events: [],
};

const runningRun: RunDetail = {
  schema_version: schemaVersionV1(),
  run_id: "run-1",
  issue_id: "issue-1",
  issue_identifier: "EDITOR-1",
  worker_id: "openhands-local",
  status: "running",
  claimed_at: "2025-09-01T00:00:00Z",
  started_at: "2025-09-01T00:00:10Z",
  turn_count: 2,
  max_turns: 10,
  input_tokens: 800,
  output_tokens: 400,
  cache_read_tokens: 100,
  runtime_seconds: 45,
  workspace_path: "/tmp/opensymphony/editor-1",
  liveness: {
    phase: "active",
    stream: "healthy",
  },
  safe_actions: {
    retry: false,
    cancel: true,
    rehydrate: false,
    detach: false,
  },
};

const releasedRun: RunDetail = {
  ...runningRun,
  run_id: "run-2",
  issue_id: "issue-2",
  issue_identifier: "EDITOR-2",
  status: "released",
  release_reason: "completed",
  finished_at: "2025-09-01T00:01:00Z",
  liveness: {
    phase: "completed",
    stream: "stale",
  },
};

function buildTaskGraph(): TaskGraphSnapshot {
  return {
    schema_version: schemaVersionV1(),
    project_id: "proj-editor",
    generated_at: "2025-09-01T00:00:00Z",
    root_ids: ["m1"],
    nodes: [
      {
        schema_version: schemaVersionV1(),
        node_id: "m1",
        kind: "milestone",
        identifier: "M1",
        title: "Editor milestone",
        state: "In Progress",
        state_category: "in_progress",
        children: ["editor-1", "editor-2"],
        blocked_by: [],
        labels: ["editor"],
      },
      {
        schema_version: schemaVersionV1(),
        node_id: "editor-1",
        kind: "issue",
        identifier: "EDITOR-1",
        title: "Running issue",
        state: "In Progress",
        state_category: "in_progress",
        parent_id: "m1",
        children: [],
        blocked_by: ["editor-2"],
        labels: ["runtime"],
        run_id: "run-1",
      },
      {
        schema_version: schemaVersionV1(),
        node_id: "editor-2",
        kind: "issue",
        identifier: "EDITOR-2",
        title: "Completed issue",
        state: "Done",
        state_category: "done",
        parent_id: "m1",
        children: [],
        blocked_by: [],
        labels: ["runtime"],
        run_id: "run-2",
      },
    ],
  };
}

function buildRunDetails(): RunDetail[] {
  return [runningRun, releasedRun];
}

function flushAsync(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, 0));
}

async function flushUntil(
  predicate: () => boolean,
  maxIterations = 40,
): Promise<void> {
  for (let i = 0; i < maxIterations; i++) {
    if (predicate()) return;
    await flushAsync();
  }
  throw new Error(
    `flushUntil timed out after ${maxIterations} iterations waiting for predicate`,
  );
}

function buildTransport(): MockGatewayTransport {
  return new MockGatewayTransport({
    baseUri: "http://127.0.0.1:2468",
    health: capabilities,
    snapshot: dashboard,
    taskGraph: buildTaskGraph(),
    runDetails: buildRunDetails(),
  });
}

describe("TaskGraphEditor", () => {
  it("renders runtime overlay badges on task nodes with linked runs", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const handle = renderOpenSymphonyApp({
      root,
      mode: "web",
      transport: buildTransport(),
    });

    await flushUntil(() => root.querySelector("[data-node-id='editor-1']") !== null);

    const runningNode = root.querySelector("[data-node-id='editor-1']");
    expect(runningNode).not.toBeNull();
    expect(runningNode?.querySelector(".os-badge-running")).not.toBeNull();
    expect(runningNode?.querySelector(".os-badge-workspace")).not.toBeNull();
    expect(runningNode?.querySelector(".os-badge-harness")).not.toBeNull();
    expect(runningNode?.querySelector(".os-badge-blocker")).not.toBeNull();

    const completedNode = root.querySelector("[data-node-id='editor-2']");
    expect(completedNode).not.toBeNull();
    expect(completedNode?.querySelector(".os-badge-complete")).not.toBeNull();
    expect(completedNode?.querySelector(".os-badge-stale")).not.toBeNull();

    await handle.destroy();
  });

  it("filters task nodes by runtime badge", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const handle = renderOpenSymphonyApp({
      root,
      mode: "web",
      transport: buildTransport(),
    });

    await flushUntil(() => root.querySelector("[data-tg-filter='runtime']") !== null);

    let runtimeSelect = root.querySelector("[data-tg-filter='runtime']") as HTMLSelectElement;
    runtimeSelect.value = "running";
    runtimeSelect.dispatchEvent(new Event("change"));
    await flushAsync();

    expect(root.querySelector("[data-node-id='editor-1']")).not.toBeNull();
    expect(root.querySelector("[data-node-id='editor-2']")).toBeNull();

    runtimeSelect = root.querySelector("[data-tg-filter='runtime']") as HTMLSelectElement;
    runtimeSelect.value = "complete";
    runtimeSelect.dispatchEvent(new Event("change"));
    await flushAsync();

    expect(root.querySelector("[data-node-id='editor-1']")).toBeNull();
    expect(root.querySelector("[data-node-id='editor-2']")).not.toBeNull();

    await handle.destroy();
  });

  it("filters the runtime badge option to match the emitted badge", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const handle = renderOpenSymphonyApp({
      root,
      mode: "web",
      transport: buildTransport(),
    });

    await flushUntil(() => root.querySelector("[data-tg-filter='runtime']") !== null);
    expect(root.querySelector("[data-node-id='editor-1']")?.querySelector(".os-badge-blocker")).not.toBeNull();

    const runtimeSelect = root.querySelector("[data-tg-filter='runtime']") as HTMLSelectElement;
    runtimeSelect.value = "blocker";
    runtimeSelect.dispatchEvent(new Event("change"));
    await flushAsync();

    expect(root.querySelector("[data-node-id='editor-1']")).not.toBeNull();
    expect(root.querySelector("[data-node-id='editor-2']")).toBeNull();

    await handle.destroy();
  });

  it("preserves search input focus and cursor position while typing", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const handle = renderOpenSymphonyApp({
      root,
      mode: "web",
      transport: buildTransport(),
    });

    await flushUntil(() => root.querySelector("[data-tg-filter='search']") !== null);
    const searchInput = root.querySelector("[data-tg-filter='search']") as HTMLInputElement;
    searchInput.focus();
    searchInput.setSelectionRange(3, 3);
    searchInput.value = "Com";
    searchInput.dispatchEvent(new Event("input"));
    await flushAsync();

    const active = document.activeElement as HTMLInputElement | null;
    expect(active?.getAttribute("data-tg-filter")).toBe("search");
    expect(active?.value).toBe("Com");
    expect(active?.selectionStart).toBe(3);
    expect(active?.selectionEnd).toBe(3);

    await handle.destroy();
  });

  it("filters task nodes by kind and state", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const handle = renderOpenSymphonyApp({
      root,
      mode: "web",
      transport: buildTransport(),
    });

    await flushUntil(() => root.querySelector("[data-tg-filter='kind']") !== null);

    let kindSelect = root.querySelector("[data-tg-filter='kind']") as HTMLSelectElement;
    kindSelect.value = "milestone";
    kindSelect.dispatchEvent(new Event("change"));
    await flushAsync();

    expect(root.querySelector("[data-node-id='m1']")).not.toBeNull();
    expect(root.querySelector("[data-node-id='editor-1']")).toBeNull();

    kindSelect = root.querySelector("[data-tg-filter='kind']") as HTMLSelectElement;
    kindSelect.value = "all";
    kindSelect.dispatchEvent(new Event("change"));
    await flushAsync();

    const stateSelect = root.querySelector("[data-tg-filter='state']") as HTMLSelectElement;
    stateSelect.value = "done";
    stateSelect.dispatchEvent(new Event("change"));
    await flushAsync();

    expect(root.querySelector("[data-node-id='editor-2']")).not.toBeNull();
    expect(root.querySelector("[data-node-id='editor-1']")).toBeNull();

    await handle.destroy();
  });

  it("filters task nodes by search text", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const handle = renderOpenSymphonyApp({
      root,
      mode: "web",
      transport: buildTransport(),
    });

    await flushUntil(() => root.querySelector("[data-tg-filter='search']") !== null);

    let searchInput = root.querySelector("[data-tg-filter='search']") as HTMLInputElement;
    searchInput.value = "Completed";
    searchInput.dispatchEvent(new Event("input"));
    await flushAsync();

    expect(root.querySelector("[data-node-id='editor-2']")).not.toBeNull();
    expect(root.querySelector("[data-node-id='editor-1']")).toBeNull();

    await handle.destroy();
  });

  it("resets filters when reset button is clicked", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const handle = renderOpenSymphonyApp({
      root,
      mode: "web",
      transport: buildTransport(),
    });

    await flushUntil(() => root.querySelector("[data-tg-filter='kind']") !== null);

    let kindSelect = root.querySelector("[data-tg-filter='kind']") as HTMLSelectElement;
    kindSelect.value = "milestone";
    kindSelect.dispatchEvent(new Event("change"));
    await flushAsync();
    expect(root.querySelector("[data-node-id='editor-1']")).toBeNull();

    root.querySelector("[data-tg-filter-reset]")?.dispatchEvent(new Event("click"));
    await flushAsync();

    expect(root.querySelector("[data-node-id='editor-1']")).not.toBeNull();
    expect(root.querySelector("[data-node-id='editor-2']")).not.toBeNull();

    await handle.destroy();
  });

  it("inline edits a node title and state", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const transport = buildTransport();
    const handle = renderOpenSymphonyApp({
      root,
      mode: "web",
      transport,
    });

    await flushUntil(() => root.querySelector("[data-tg-edit='editor-1']") !== null);

    (root.querySelector("[data-tg-edit='editor-1']") as HTMLButtonElement).click();
    await flushAsync();

    const titleInput = root.querySelector("[data-tg-inline-title='editor-1']") as HTMLInputElement;
    const stateInput = root.querySelector("[data-tg-inline-state='editor-1']") as HTMLInputElement;
    expect(titleInput).not.toBeNull();
    expect(stateInput).not.toBeNull();

    titleInput.value = "Updated running issue";
    stateInput.value = "Done";

    (root.querySelector("[data-tg-inline-save='editor-1']") as HTMLButtonElement).click();
    await flushAsync();

    expect(root.querySelector("[data-tg-inline-title='editor-1']")).toBeNull();
    expect(root.textContent).toContain("Updated running issue");
    expect(root.textContent).toContain("Done");

    await handle.destroy();
  });

  it("creates a sub-issue under a selected node and links it", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const transport = buildTransport();
    const handle = renderOpenSymphonyApp({
      root,
      mode: "web",
      transport,
    });

    await flushUntil(() => root.querySelector("[data-tg-create-child='editor-1']") !== null);

    (root.querySelector("[data-tg-create-child='editor-1']") as HTMLButtonElement).click();
    await flushAsync();

    expect(root.querySelector("[data-tg-create-dialog='open']")).not.toBeNull();

    const titleInput = root.querySelector("[data-tg-create-title]") as HTMLInputElement;
    titleInput.value = "New sub-issue";
    (root.querySelector("[data-tg-create-save]") as HTMLButtonElement).click();
    await flushAsync();

    expect(root.querySelector("[data-tg-create-dialog='open']")).toBeNull();
    expect(root.textContent).toContain("New sub-issue");

    await handle.destroy();
  });

  it("edits dependencies for a node", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const transport = buildTransport();
    const handle = renderOpenSymphonyApp({
      root,
      mode: "web",
      transport,
    });

    await flushUntil(() => root.querySelector("[data-tg-deps='editor-1']") !== null);

    (root.querySelector("[data-tg-deps='editor-1']") as HTMLButtonElement).click();
    await flushAsync();

    expect(root.querySelector("[data-tg-deps-dialog='open']")).not.toBeNull();

    const select = root.querySelector("[data-tg-deps-select]") as HTMLSelectElement;
    // Remove existing blocked_by selection.
    Array.from(select.options).forEach((option) => {
      option.selected = false;
    });
    (root.querySelector("[data-tg-deps-save]") as HTMLButtonElement).click();
    await flushAsync();

    expect(root.querySelector("[data-tg-deps-dialog='open']")).toBeNull();
    const updatedNode = root.querySelector("[data-node-id='editor-1']");
    expect(updatedNode?.querySelector(".os-badge-blocker")).toBeNull();

    await handle.destroy();
  });

  it("adds a comment to a node", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const transport = buildTransport();
    const handle = renderOpenSymphonyApp({
      root,
      mode: "web",
      transport,
    });

    await flushUntil(() => root.querySelector("[data-tg-comment='editor-1']") !== null);

    (root.querySelector("[data-tg-comment='editor-1']") as HTMLButtonElement).click();
    await flushAsync();

    expect(root.querySelector("[data-tg-comment-dialog='open']")).not.toBeNull();

    const bodyInput = root.querySelector("[data-tg-comment-body]") as HTMLTextAreaElement;
    bodyInput.value = "This is evidence";
    (root.querySelector("[data-tg-comment-save]") as HTMLButtonElement).click();
    await flushAsync();

    expect(root.querySelector("[data-tg-comment-dialog='open']")).toBeNull();

    await handle.destroy();
  });

  it("shows a pending acknowledgement banner during a mutation and reconciles after receipt", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const transport = buildTransport();
    const handle = renderOpenSymphonyApp({
      root,
      mode: "web",
      transport,
    });

    await flushUntil(() => root.querySelector("[data-tg-edit='editor-1']") !== null);

    (root.querySelector("[data-tg-edit='editor-1']") as HTMLButtonElement).click();
    await flushAsync();

    const titleInput = root.querySelector("[data-tg-inline-title='editor-1']") as HTMLInputElement;
    titleInput.value = "Acknowledged title";
    (root.querySelector("[data-tg-inline-save='editor-1']") as HTMLButtonElement).click();

    await flushUntil(() => root.textContent?.includes("pending server acknowledgement") ?? false);
    expect(root.querySelector(".os-pending-banner")).not.toBeNull();

    await flushUntil(() => root.querySelector(".os-pending-banner") === null);
    expect(root.textContent).toContain("Acknowledged title");

    await handle.destroy();
  });

  it("treats a rejected mutation receipt as a failure even when a result is present", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const transport = buildTransport();
    const rejectedAt = new Date().toISOString();
    transport.dispatchAction = async (action) => ({
      schema_version: schemaVersionV1(),
      action_id: "mock-rejected",
      correlation_id: action.correlation_id,
      status: "rejected",
      reason: "Title is not allowed",
      expected_events: [],
      issued_at: rejectedAt,
      result: { node_id: "editor-1", updated_at: rejectedAt, applied: true },
    });

    const handle = renderOpenSymphonyApp({
      root,
      mode: "web",
      transport,
    });

    await flushUntil(() => root.querySelector("[data-tg-edit='editor-1']") !== null);

    (root.querySelector("[data-tg-edit='editor-1']") as HTMLButtonElement).click();
    await flushAsync();

    const titleInput = root.querySelector("[data-tg-inline-title='editor-1']") as HTMLInputElement;
    titleInput.value = "Rejected title";
    (root.querySelector("[data-tg-inline-save='editor-1']") as HTMLButtonElement).click();

    await flushUntil(() => root.querySelector(".os-pending-banner") === null);
    expect(root.textContent).toContain("Mutation rejected: Title is not allowed");
    expect(root.textContent).toContain("Running issue");
    expect(root.textContent).not.toContain("Rejected title");

    await handle.destroy();
  });

  describe("optimistic mutation rollback on failure", () => {
    function rejectDispatchAction(reason: string) {
      return async (action: any) => {
        const rejectedAt = new Date().toISOString();
        return {
          schema_version: schemaVersionV1(),
          action_id: "mock-rejected",
          correlation_id: action.correlation_id,
          status: "rejected",
          reason,
          expected_events: [],
          issued_at: rejectedAt,
          result: { node_id: "editor-1", updated_at: rejectedAt, applied: true },
        };
      };
    }

    it("restores the original title after a rejected inline edit", async () => {
      const root = document.createElement("div");
      document.body.appendChild(root);
      const transport = buildTransport();
      transport.dispatchAction = rejectDispatchAction("Title is not allowed");
      const handle = renderOpenSymphonyApp({
        root,
        mode: "web",
        transport,
      });

      await flushUntil(() => root.querySelector("[data-tg-edit='editor-1']") !== null);

      (root.querySelector("[data-tg-edit='editor-1']") as HTMLButtonElement).click();
      await flushAsync();

      const titleInput = root.querySelector("[data-tg-inline-title='editor-1']") as HTMLInputElement;
      titleInput.value = "Rejected title";
      (root.querySelector("[data-tg-inline-save='editor-1']") as HTMLButtonElement).click();

      await flushUntil(() => root.querySelector(".os-pending-banner") === null);

      expect(root.textContent).toContain("Mutation rejected: Title is not allowed");
      expect(root.textContent).not.toContain("Rejected title");
      expect(root.textContent).toContain("Running issue");

      await handle.destroy();
    });

    it("removes the optimistic node after a rejected create", async () => {
      const root = document.createElement("div");
      document.body.appendChild(root);
      const transport = buildTransport();
      transport.dispatchAction = rejectDispatchAction("Create is not allowed");
      const handle = renderOpenSymphonyApp({
        root,
        mode: "web",
        transport,
      });

      await flushUntil(() => root.querySelector("[data-tg-create='milestone']") !== null);

      (root.querySelector("[data-tg-create='milestone']") as HTMLButtonElement).click();
      await flushAsync();

      const titleInput = root.querySelector("[data-tg-create-title]") as HTMLInputElement;
      titleInput.value = "Rejected milestone";
      (root.querySelector("[data-tg-create-save]") as HTMLButtonElement).click();

      await flushUntil(() => root.querySelector(".os-pending-banner") === null);

      expect(root.textContent).not.toContain("Rejected milestone");

      await handle.destroy();
    });

    it("restores the blocked_by list after a rejected dependency edit", async () => {
      const root = document.createElement("div");
      document.body.appendChild(root);
      const transport = buildTransport();
      transport.dispatchAction = rejectDispatchAction("Dependencies are not allowed");
      const handle = renderOpenSymphonyApp({
        root,
        mode: "web",
        transport,
      });

      await flushUntil(() => root.querySelector("[data-tg-deps='editor-1']") !== null);

      (root.querySelector("[data-tg-deps='editor-1']") as HTMLButtonElement).click();
      await flushAsync();

      const select = root.querySelector("[data-tg-deps-select]") as HTMLSelectElement;
      Array.from(select.options).forEach((option) => {
        option.selected = false;
      });
      (root.querySelector("[data-tg-deps-save]") as HTMLButtonElement).click();

      await flushUntil(() => root.querySelector(".os-pending-banner") === null);

      const updatedNode = root.querySelector("[data-node-id='editor-1']");
      expect(updatedNode?.querySelector(".os-badge-blocker")).not.toBeNull();

      await handle.destroy();
    });

    it("restores the comment count after a rejected comment", async () => {
      const root = document.createElement("div");
      document.body.appendChild(root);
      const transport = buildTransport();
      transport.dispatchAction = rejectDispatchAction("Comment is not allowed");
      const handle = renderOpenSymphonyApp({
        root,
        mode: "web",
        transport,
      });

      await flushUntil(() => root.querySelector("[data-tg-comment='editor-1']") !== null);

      const button = root.querySelector("[data-tg-comment='editor-1']") as HTMLButtonElement;
      expect(button.textContent).toBe("Comment");
      button.click();
      await flushAsync();

      const bodyInput = root.querySelector("[data-tg-comment-body]") as HTMLTextAreaElement;
      bodyInput.value = "Rejected evidence";
      (root.querySelector("[data-tg-comment-save]") as HTMLButtonElement).click();

      await flushUntil(() => root.querySelector(".os-pending-banner") === null);

      const restoredButton = root.querySelector("[data-tg-comment='editor-1']") as HTMLButtonElement;
      expect(restoredButton.textContent).toBe("Comment");

      await handle.destroy();
    });

    it("rolls back an optimistic create when the transport throws", async () => {
      const root = document.createElement("div");
      document.body.appendChild(root);
      const transport = buildTransport();
      transport.dispatchAction = async () => {
        throw new Error("Network unreachable");
      };
      const handle = renderOpenSymphonyApp({
        root,
        mode: "web",
        transport,
      });

      await flushUntil(() => root.querySelector("[data-tg-create='milestone']") !== null);

      (root.querySelector("[data-tg-create='milestone']") as HTMLButtonElement).click();
      await flushAsync();

      const titleInput = root.querySelector("[data-tg-create-title]") as HTMLInputElement;
      titleInput.value = "Lost milestone";
      (root.querySelector("[data-tg-create-save]") as HTMLButtonElement).click();

      await flushUntil(() => root.textContent?.includes("Create failed: Network unreachable") ?? false);

      expect(root.textContent).not.toContain("Lost milestone");
      expect(root.textContent).toContain("Create failed: Network unreachable");

      await handle.destroy();
    });
  });

  it("saves inline edits on a node whose id contains a single quote", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const graph = buildTaskGraph();
    const quotedNode: TaskGraphNode = {
      schema_version: schemaVersionV1(),
      node_id: "node'quote",
      kind: "issue",
      identifier: "QUOTED-1",
      title: "Quoted issue",
      state: "Todo",
      state_category: "todo",
      parent_id: "m1",
      children: [],
      blocked_by: [],
      labels: [],
    };
    graph.nodes.push(quotedNode);
    graph.nodes.find((n) => n.node_id === "m1")?.children.push("node'quote");

    const transport = buildTransport();
    (transport as any).mockTaskGraph = graph;
    const handle = renderOpenSymphonyApp({
      root,
      mode: "web",
      transport,
    });

    await flushUntil(() => root.querySelector("[data-node-id=\"node'quote\"]") !== null);

    (root.querySelector("[data-tg-edit=\"node'quote\"]") as HTMLButtonElement).click();
    await flushAsync();

    const titleInput = root.querySelector("[data-tg-inline-title=\"node'quote\"]") as HTMLInputElement;
    expect(titleInput).not.toBeNull();
    titleInput.value = "Updated quoted issue";
    (root.querySelector("[data-tg-inline-save=\"node'quote\"]") as HTMLButtonElement).click();
    await flushAsync();

    expect(root.textContent).toContain("Updated quoted issue");

    await handle.destroy();
  });

  it("creates a root-level milestone and dispatches to the server", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const transport = buildTransport();
    const handle = renderOpenSymphonyApp({
      root,
      mode: "web",
      transport,
    });

    await flushUntil(() => root.querySelector("[data-tg-create='milestone']") !== null);

    (root.querySelector("[data-tg-create='milestone']") as HTMLButtonElement).click();
    await flushAsync();

    expect(root.querySelector("[data-tg-create-dialog='open']")).not.toBeNull();

    const titleInput = root.querySelector("[data-tg-create-title]") as HTMLInputElement;
    titleInput.value = "Root milestone";
    (root.querySelector("[data-tg-create-save]") as HTMLButtonElement).click();
    await flushAsync();

    expect(root.querySelector("[data-tg-create-dialog='open']")).toBeNull();
    expect(root.textContent).toContain("Root milestone");
    await flushUntil(() => root.querySelector(".os-pending-banner") === null);

    await handle.destroy();
  });

  it("does not double-render the title during inline editing", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const handle = renderOpenSymphonyApp({
      root,
      mode: "web",
      transport: buildTransport(),
    });

    await flushUntil(() => root.querySelector("[data-tg-edit='editor-1']") !== null);
    expect(root.textContent).toContain("Running issue");

    (root.querySelector("[data-tg-edit='editor-1']") as HTMLButtonElement).click();
    await flushAsync();

    // The read-only title span should be removed while the input is active.
    expect(root.querySelector("[data-tg-inline-title='editor-1']")).not.toBeNull();
    expect(root.textContent).not.toContain("Running issue");

    await handle.destroy();
  });

  it("deduplicates retry badges when retry_queued carries a retry attempt", () => {
    const node: TaskGraphNode = {
      schema_version: schemaVersionV1(),
      node_id: "retry-1",
      kind: "issue",
      identifier: "RETRY-1",
      title: "Retry queued issue",
      state: "In Progress",
      state_category: "in_progress",
      children: [],
      blocked_by: [],
      labels: [],
    };
    const run: RunDetail = {
      schema_version: schemaVersionV1(),
      run_id: "run-retry",
      issue_id: "retry-1",
      issue_identifier: "RETRY-1",
      worker_id: "worker-1",
      status: "retry_queued",
      claimed_at: "2025-09-01T00:00:00Z",
      turn_count: 0,
      max_turns: 10,
      input_tokens: 0,
      output_tokens: 0,
      cache_read_tokens: 0,
      runtime_seconds: 0,
      retry_attempt: 3,
    };
    const overlay = buildRuntimeOverlay(node, run);
    const retryCount = overlay.badges.filter((b) => b === "retry").length;
    expect(retryCount).toBe(1);
    expect(overlay.badges).toContain("retry");
    expect(overlay.badges).toContain("queued");
  });
});
