/**
 * @jest-environment jsdom
 *
 * Planning workspace UI tests for COE-417.
 */

import { renderOpenSymphonyApp } from "../src/app-shell.js";
import {
  emptyPlanningWorkspaceState,
  hasDependencyCycle,
  updateArtifactContent,
  validatePlanningWorkspace,
} from "../src/planning-workspace.js";
import { MockGatewayTransport } from "@opensymphony/api-client";
import { schemaVersionV1 } from "@opensymphony/gateway-schema";
import type {
  DashboardSnapshot,
  GatewayCapabilities,
  TaskGraphSnapshot,
} from "@opensymphony/gateway-schema";

const capabilities: GatewayCapabilities = {
  schema_version: schemaVersionV1(),
  gateway_version: "planning-test",
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
    running_issue_count: 0,
    retry_queue_depth: 0,
    total_input_tokens: 0,
    total_output_tokens: 0,
    total_cache_read_tokens: 0,
    total_cost_micros: 0,
  },
  projects: [
    {
      project_id: "proj-planning",
      name: "Planning Test Project",
      milestone_count: 1,
      issue_count: 2,
      running_count: 0,
      completed_count: 0,
      failed_count: 0,
    },
  ],
  recent_events: [],
};

const taskGraph: TaskGraphSnapshot = {
  schema_version: schemaVersionV1(),
  project_id: "proj-planning",
  generated_at: "2025-09-01T00:00:00Z",
  root_ids: ["m1"],
  nodes: [
    {
      schema_version: schemaVersionV1(),
      node_id: "m1",
      kind: "milestone",
      identifier: "M1",
      title: "Planning milestone",
      state: "In Progress",
      state_category: "in_progress",
      children: ["editor-1"],
      blocked_by: [],
      labels: ["planning"],
    },
    {
      schema_version: schemaVersionV1(),
      node_id: "editor-1",
      kind: "issue",
      identifier: "EDITOR-1",
      title: "Planning issue",
      state: "In Progress",
      state_category: "in_progress",
      parent_id: "m1",
      children: [],
      blocked_by: [],
      labels: ["planning"],
    },
  ],
};

function buildTransport(): MockGatewayTransport {
  return new MockGatewayTransport({
    baseUri: "http://127.0.0.1:8000",
    health: capabilities,
    snapshot: dashboard,
    taskGraph,
    runDetails: [],
  });
}

async function waitForPlanningReady(root: HTMLElement): Promise<void> {
  await flushUntil(() => root.querySelector(".os-status-connected") !== null);
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

describe("PlanningWorkspace", () => {
  it("renders the planning workspace with conversation and artifact panes", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport: buildTransport(),
    });

    await flushUntil(
      () => root.querySelector("[data-plan-view='planning']") !== null,
    );
    (root.querySelector("[data-plan-view='planning']") as HTMLButtonElement).click();
    await flushUntil(() => root.querySelector(".os-planning-panel") !== null);
    await waitForPlanningReady(root);

    expect(root.querySelector("[data-plan-tab='artifact']")).not.toBeNull();
    expect(root.querySelector("[data-plan-tab='hierarchy']")).not.toBeNull();
    expect(root.querySelector("[data-plan-tab='dependencies']")).not.toBeNull();
    expect(root.querySelector("[data-plan-tab='validation']")).not.toBeNull();
    expect(root.querySelector("[data-plan-tab='diff']")).not.toBeNull();
    expect(root.querySelector("[data-plan-composer]")).not.toBeNull();
    expect(root.querySelector("[data-plan-artifact-select]")).not.toBeNull();

    await handle.destroy();
  });

  it("sends a message from the conversation pane", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport: buildTransport(),
    });

    await flushUntil(() => root.querySelector("[data-plan-view='planning']") !== null);
    (root.querySelector("[data-plan-view='planning']") as HTMLButtonElement).click();
    await flushUntil(() => root.querySelector("[data-plan-composer]") !== null);
    await waitForPlanningReady(root);

    const composer = root.querySelector("[data-plan-composer]") as HTMLTextAreaElement;
    composer.value = "What is the next step?";
    (root.querySelector("[data-plan-send-message]") as HTMLButtonElement).click();
    await flushAsync();

    expect(root.textContent).toContain("What is the next step?");
    expect(root.textContent).toContain("Acknowledged.");

    await handle.destroy();
  });

  it("sends a message when pressing Enter in the composer", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport: buildTransport(),
    });

    await flushUntil(() => root.querySelector("[data-plan-view='planning']") !== null);
    (root.querySelector("[data-plan-view='planning']") as HTMLButtonElement).click();
    await flushUntil(() => root.querySelector("[data-plan-composer]") !== null);
    await waitForPlanningReady(root);

    const composer = root.querySelector("[data-plan-composer]") as HTMLTextAreaElement;
    composer.focus();
    composer.value = "Enter key test";
    composer.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    await flushAsync();

    expect(root.textContent).toContain("Enter key test");

    await handle.destroy();
  });

  it("saves an edited artifact as a new revision", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport: buildTransport(),
    });

    await flushUntil(() => root.querySelector("[data-plan-view='planning']") !== null);
    (root.querySelector("[data-plan-view='planning']") as HTMLButtonElement).click();
    await flushUntil(() => root.querySelector("[data-plan-artifact-select]") !== null);
    await waitForPlanningReady(root);

    const select = root.querySelector("[data-plan-artifact-select]") as HTMLSelectElement;
    select.value = "artifact-intake";
    select.dispatchEvent(new Event("change"));
    await flushAsync();

    const textarea = root.querySelector("[data-plan-artifact-content]") as HTMLTextAreaElement;
    const originalRevisionOptions = root.querySelectorAll("[data-plan-revision-select] option").length;
    textarea.value = "Updated intake content";
    (root.querySelector("[data-plan-save-artifact]") as HTMLButtonElement).click();
    await flushAsync();

    const options = root.querySelectorAll("[data-plan-revision-select] option");
    expect(options.length).toBe(originalRevisionOptions + 1);
    expect(root.textContent).toContain("Updated intake content");

    await handle.destroy();
  });

  it("renders a diff between artifact revisions", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport: buildTransport(),
    });

    await flushUntil(() => root.querySelector("[data-plan-view='planning']") !== null);
    (root.querySelector("[data-plan-view='planning']") as HTMLButtonElement).click();
    await flushUntil(() => root.querySelector("[data-plan-tab='diff']") !== null);
    await waitForPlanningReady(root);
    (root.querySelector("[data-plan-tab='diff']") as HTMLButtonElement).click();
    await flushUntil(() => root.querySelector(".os-plan-diff") !== null);

    const addedLine = root.querySelector(".os-plan-diff-add");
    expect(addedLine).not.toBeNull();
    expect(root.textContent).toContain("Acceptance criteria editor");

    await handle.destroy();
  });

  it("adds and edits a hierarchy node", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport: buildTransport(),
    });

    await flushUntil(() => root.querySelector("[data-plan-view='planning']") !== null);
    (root.querySelector("[data-plan-view='planning']") as HTMLButtonElement).click();
    await flushUntil(() => root.querySelector("[data-plan-tab='hierarchy']") !== null);
    await waitForPlanningReady(root);
    (root.querySelector("[data-plan-tab='hierarchy']") as HTMLButtonElement).click();
    await flushUntil(() => root.querySelector("[data-plan-add-node='milestone']") !== null);

    (root.querySelector("[data-plan-add-node='milestone']") as HTMLButtonElement).click();
    await flushAsync();

    const titleInput = root.querySelector("[data-plan-node-title]") as HTMLInputElement;
    expect(titleInput).not.toBeNull();
    titleInput.value = "Custom milestone";
    (root.querySelector("[data-plan-node-save]") as HTMLButtonElement).click();
    await flushAsync();

    expect(root.textContent).toContain("Custom milestone");

    await handle.destroy();
  });

  it("selects a hierarchy node and toggles expansion", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport: buildTransport(),
    });

    await flushUntil(() => root.querySelector("[data-plan-view='planning']") !== null);
    (root.querySelector("[data-plan-view='planning']") as HTMLButtonElement).click();
    await flushUntil(() => root.querySelector("[data-plan-tab='hierarchy']") !== null);
    await waitForPlanningReady(root);
    (root.querySelector("[data-plan-tab='hierarchy']") as HTMLButtonElement).click();
    await flushUntil(() => root.querySelector("[data-plan-node-toggle='plan-milestone']") !== null);

    (root.querySelector("[data-plan-node-toggle='plan-milestone']") as HTMLButtonElement).click();
    await flushAsync();

    expect(root.querySelector("[data-plan-node-select='plan-sub-1']")).toBeNull();

    (root.querySelector("[data-plan-node-toggle='plan-milestone']") as HTMLButtonElement).click();
    await flushAsync();

    expect(root.querySelector("[data-plan-node-select='plan-sub-1']")).not.toBeNull();

    await handle.destroy();
  });

  it("edits dependencies for a node and shows the graph view", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport: buildTransport(),
    });

    await flushUntil(() => root.querySelector("[data-plan-view='planning']") !== null);
    (root.querySelector("[data-plan-view='planning']") as HTMLButtonElement).click();
    await flushUntil(() => root.querySelector("[data-plan-tab='dependencies']") !== null);
    await waitForPlanningReady(root);
    (root.querySelector("[data-plan-tab='dependencies']") as HTMLButtonElement).click();
    await flushUntil(() => root.querySelector("[data-plan-deps-select]") !== null);

    const select = root.querySelector("[data-plan-deps-select]") as HTMLSelectElement;
    const targetOption = Array.from(select.options).find((option) => option.value === "plan-milestone");
    expect(targetOption).not.toBeNull();
    targetOption!.selected = true;
    (root.querySelector("[data-plan-deps-save]") as HTMLButtonElement).click();
    await flushAsync();

    const updatedSelect = root.querySelector("[data-plan-deps-select]") as HTMLSelectElement;
    const selectedValues = Array.from(updatedSelect.selectedOptions).map((option) => option.value);
    expect(selectedValues).toContain("plan-milestone");
    expect(root.querySelector(".os-plan-graph svg")).not.toBeNull();

    await handle.destroy();
  });

  it("adds, toggles, and removes acceptance criteria", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport: buildTransport(),
    });

    await flushUntil(() => root.querySelector("[data-plan-view='planning']") !== null);
    (root.querySelector("[data-plan-view='planning']") as HTMLButtonElement).click();
    await flushUntil(() => root.querySelector("[data-plan-tab='criteria']") !== null);
    await waitForPlanningReady(root);
    (root.querySelector("[data-plan-tab='criteria']") as HTMLButtonElement).click();
    await flushUntil(() => root.querySelector("[data-plan-criteria-new]") !== null);

    const initialCount = root.querySelectorAll("[data-plan-criteria-text]").length;
    const input = root.querySelector("[data-plan-criteria-new]") as HTMLInputElement;
    input.value = "New criterion";
    (root.querySelector("[data-plan-criteria-add]") as HTMLButtonElement).click();
    await flushAsync();

    expect(root.querySelectorAll("[data-plan-criteria-text]").length).toBe(initialCount + 1);

    const newInput = Array.from(root.querySelectorAll("[data-plan-criteria-text]")).find(
      (el) => (el as HTMLInputElement).value === "New criterion",
    ) as HTMLInputElement;
    expect(newInput).not.toBeUndefined();
    const newId = newInput.dataset.planCriteriaText;

    const checkbox = root.querySelector(`[data-plan-criteria-toggle="${newId}"]`) as HTMLInputElement;
    expect(checkbox).not.toBeNull();
    checkbox.checked = true;
    checkbox.dispatchEvent(new Event("change"));
    await flushAsync();

    expect((root.querySelector(`[data-plan-criteria-toggle="${newId}"]`) as HTMLInputElement).checked).toBe(true);

    (root.querySelector(`[data-plan-criteria-remove="${newId}"]`) as HTMLButtonElement).click();
    await flushAsync();

    expect(root.querySelector(`[data-plan-criteria-text="${newId}"]`)).toBeNull();
    expect(root.querySelectorAll("[data-plan-criteria-text]").length).toBe(initialCount);

    await handle.destroy();
  });

  it("renders validation messages that link to artifact fields", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport: buildTransport(),
    });

    await flushUntil(() => root.querySelector("[data-plan-view='planning']") !== null);
    (root.querySelector("[data-plan-view='planning']") as HTMLButtonElement).click();
    await flushUntil(() => root.querySelector("[data-plan-tab='validation']") !== null);
    await waitForPlanningReady(root);
    (root.querySelector("[data-plan-tab='validation']") as HTMLButtonElement).click();
    await flushUntil(() => root.querySelector("[data-plan-validation-link]") !== null);

    const link = root.querySelector("[data-plan-validation-link]") as HTMLElement;
    expect(link).not.toBeNull();
    expect(link.dataset.planFieldKind).toMatch(/^(artifact|node|criteria|verification|dependency)$/);
    expect(link.dataset.planFieldId).toBeTruthy();

    // Click an artifact link and verify it navigates to the artifact editor.
    const artifactLink = Array.from(root.querySelectorAll("[data-plan-validation-link]")).find(
      (el) => el.dataset.planFieldKind === "artifact",
    ) as HTMLElement | undefined;
    expect(artifactLink).not.toBeUndefined();
    artifactLink!.click();
    await flushAsync();

    expect(root.querySelector("[data-plan-tab='artifact'].os-plan-tab-active")).not.toBeNull();
    expect(root.querySelector("[data-plan-artifact-select]")).not.toBeNull();

    await handle.destroy();
  });

  it("preserves focus while typing in a checklist input", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport: buildTransport(),
    });

    await flushUntil(() => root.querySelector("[data-plan-view='planning']") !== null);
    (root.querySelector("[data-plan-view='planning']") as HTMLButtonElement).click();
    await flushUntil(() => root.querySelector("[data-plan-tab='criteria']") !== null);
    await waitForPlanningReady(root);
    (root.querySelector("[data-plan-tab='criteria']") as HTMLButtonElement).click();
    await flushUntil(() => root.querySelector("[data-plan-criteria-text]") !== null);

    const input = root.querySelector("[data-plan-criteria-text]") as HTMLInputElement;
    input.focus();
    input.value = "Editing criterion";
    input.setSelectionRange(3, 3);
    input.dispatchEvent(new Event("input", { bubbles: true }));

    const active = root.ownerDocument!.activeElement as HTMLElement;
    expect(active).not.toBeNull();
    expect(active.dataset.planCriteriaText).toBe(input.dataset.planCriteriaText);
    expect((active as HTMLInputElement).value).toBe("Editing criterion");
    expect(active.selectionStart).toBe(3);
    expect(active.selectionEnd).toBe(3);

    await handle.destroy();
  });

  it("detects dependency cycles that do not include the start node", () => {
    const a = {
      schema_version: schemaVersionV1(),
      node_id: "a",
      kind: "issue" as const,
      identifier: "A",
      title: "A",
      state: "Todo",
      state_category: "todo" as const,
      parent_id: undefined,
      children: [],
      blocked_by: ["b"],
    };
    const b = {
      schema_version: schemaVersionV1(),
      node_id: "b",
      kind: "issue" as const,
      identifier: "B",
      title: "B",
      state: "Todo",
      state_category: "todo" as const,
      parent_id: undefined,
      children: [],
      blocked_by: ["c"],
    };
    const c = {
      schema_version: schemaVersionV1(),
      node_id: "c",
      kind: "issue" as const,
      identifier: "C",
      title: "C",
      state: "Todo",
      state_category: "todo" as const,
      parent_id: undefined,
      children: [],
      blocked_by: ["b"],
    };
    const nodeMap = new Map([a, b, c].map((n) => [n.node_id, n]));

    expect(hasDependencyCycle(nodeMap, "a")).toBe(true);
  });

  it("reports each dependency cycle only once", () => {
    const base = emptyPlanningWorkspaceState();
    const a = {
      schema_version: schemaVersionV1(),
      node_id: "a",
      kind: "issue" as const,
      identifier: "A",
      title: "A",
      state: "Todo",
      state_category: "todo" as const,
      parent_id: undefined,
      children: [],
      blocked_by: ["b"],
    };
    const b = {
      schema_version: schemaVersionV1(),
      node_id: "b",
      kind: "issue" as const,
      identifier: "B",
      title: "B",
      state: "Todo",
      state_category: "todo" as const,
      parent_id: undefined,
      children: [],
      blocked_by: ["c"],
    };
    const c = {
      schema_version: schemaVersionV1(),
      node_id: "c",
      kind: "issue" as const,
      identifier: "C",
      title: "C",
      state: "Todo",
      state_category: "todo" as const,
      parent_id: undefined,
      children: [],
      blocked_by: ["a"],
    };
    const state = {
      ...base,
      nodes: [a, b, c],
      activeTab: "validation" as const,
    };
    const messages = validatePlanningWorkspace(state).filter(
      (m) => m.message_id.includes("-cycle"),
    );
    expect(messages.length).toBe(1);
    expect(messages[0].message).toContain("A");
    expect(messages[0].message).toContain("B");
    expect(messages[0].message).toContain("C");
  });

  it("reports dangling blocked_by references", () => {
    const base = emptyPlanningWorkspaceState();
    const a = {
      schema_version: schemaVersionV1(),
      node_id: "a",
      kind: "issue" as const,
      identifier: "A",
      title: "A",
      state: "Todo",
      state_category: "todo" as const,
      parent_id: undefined,
      children: [],
      blocked_by: ["missing"],
    };
    const state = { ...base, nodes: [a] };
    const messages = validatePlanningWorkspace(state).filter(
      (m) => m.message_id.includes("-dangling-"),
    );
    expect(messages.length).toBe(1);
    expect(messages[0].level).toBe("error");
    expect(messages[0].message).toContain("missing");
    expect(messages[0].field_ref).toEqual({ kind: "dependency", id: "a" });
  });

  it("does not create a new artifact revision when content is unchanged", () => {
    const base = emptyPlanningWorkspaceState();
    const artifact = {
      schema_version: schemaVersionV1(),
      artifact_id: "art-1",
      session_id: base.session_id,
      kind: "intake" as const,
      title: "Intake",
      created_at: new Date().toISOString(),
      updated_at: new Date().toISOString(),
      approved: false,
      published_to_tracker: false,
      revisions: [
        { revision_id: "rev-1", created_at: new Date().toISOString(), content: "same content" },
      ],
    };
    const state = { ...base, artifacts: [artifact], selectedArtifactId: "art-1" };
    const next = updateArtifactContent(state, "art-1", "same content");
    expect(next.artifacts[0].revisions.length).toBe(1);
    expect(next.artifacts[0].revisions[0].revision_id).toBe("rev-1");
    expect(next.selectedRevisionId).toBe("rev-1");
  });
});
