/**
 * @jest-environment jsdom
 *
 * App-shell mount smoke tests for COE-449 desktop alpha recovery.
 */

import { renderOpenSymphonyApp } from "../src/app-shell.js";
import { MockGatewayTransport } from "@opensymphony/api-client";
import {
  computeGraphLayout,
  createFixtureCodeGraphAdapter,
  createFixtureGraphAdapter,
  createScaleGraphSnapshot,
  fixtureConceptDetail,
  fixtureGraphSnapshot,
  graphVizFixtureBundleList,
  graphVizFixtureCommunityList,
  graphVizFixtureConceptDetail,
  graphVizFixtureSnapshot,
  initialGraphState,
  pageCompletedTasks,
  type GraphDataAdapter,
  type MemoryCompletedTask,
  type CodeGraphSnapshot,
} from "@opensymphony/graph";
import { schemaVersionV1 } from "@opensymphony/gateway-schema";
import {
  bindKnowledgeGraphListNavigation,
  createKnowledgeGraphViewState,
  disposeKnowledgeGraphRenderer,
  mountKnowledgeGraphRenderer,
  renderKnowledgeGraphInspector,
  renderKnowledgeGraphNodeList,
  renderKnowledgeGraphSurface,
} from "../src/knowledge-graph-renderer.js";
import { hitTestHull, hitTestScene, type GraphScene } from "../src/knowledge-graph-scene.js";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import type {
  EditableProfileInput,
  ModelProfileController,
  ProfileController,
} from "../src/app-shell.js";
import type {
  CodeGraphNode,
  ConnectionProfile,
  ChangedFileEntry,
  DashboardSnapshot,
  FileDiffPage,
  GatewayEnvelope,
  GatewayCapabilities,
  ModelConfigurationProfile,
  RunDetail,
  RunEventPage,
  TaskGraphSnapshot,
} from "@opensymphony/gateway-schema";
import { defaultModelProfiles } from "@opensymphony/gateway-schema";

interface ProjectGroupingFixtureIssue {
  id: string;
  title: string;
  runtime_state: string;
  tracker_state: string;
  project_id: string | null;
  project_slug: string | null;
  project_name: string | null;
  workspace_label: string | null;
  blocked_by: string[];
}

const projectGroupingFixture = JSON.parse(readFileSync(
  join(__dirname, "../../../tests/fixtures/project_grouping_cases.json"),
  "utf8",
)) as ProjectGroupingFixtureIssue[];

const capabilities: GatewayCapabilities = {
  schema_version: schemaVersionV1(),
  gateway_version: "alpha-test",
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
    { feature: "terminal_stream", available: false, requires_auth: false },
  ],
  auth_modes: ["none"],
  max_event_page_size: 1000,
  max_terminal_frame_batch: 500,
};

const dashboard: DashboardSnapshot = {
  schema_version: schemaVersionV1(),
  generated_at: "2025-09-01T00:00:00Z",
  sequence: 7,
  health: "healthy",
  metrics: {
    running_issue_count: 2,
    retry_queue_depth: 1,
    total_input_tokens: 4500,
    total_output_tokens: 1100,
    total_cache_read_tokens: 600,
    total_cost_micros: 250,
  },
  projects: [
    {
      project_id: "proj-alpha",
      name: "Alpha Recovery",
      milestone_count: 2,
      issue_count: 4,
      running_count: 1,
      completed_count: 2,
      failed_count: 1,
    },
  ],
  recent_events: [
    {
      happened_at: "2025-09-01T00:00:05Z",
      kind: "codex.thread/tokenUsage/updated",
      issue_identifier: "COE-449",
      summary: "Codex token usage: 5539770 input, 35185 output, 5271680 cache",
    },
    {
      happened_at: "2025-09-01T00:00:04Z",
      kind: "codex.turn/diff/updated",
      issue_identifier: "COE-449",
      summary: "Codex diff updated",
    },
    {
      happened_at: "2025-09-01T00:00:00Z",
      kind: "client_attached",
      issue_identifier: "COE-449",
      summary: "App shell mounted under test",
    },
    {
      happened_at: "2025-09-01T00:00:01Z",
      kind: "snapshot_published",
      issue_identifier: "COE-450",
      summary: "published dependency-aware snapshot",
    },
    {
      happened_at: "2025-09-01T00:00:02Z",
      kind: "run_event",
      issue_identifier: "COE-451",
      summary: "captured runtime event",
    },
    {
      happened_at: "2025-09-01T00:00:03Z",
      kind: "hidden_event",
      issue_identifier: "COE-452",
      summary: "should not render in compact status",
    },
  ],
};

const taskGraph: TaskGraphSnapshot = {
  schema_version: schemaVersionV1(),
  project_id: "proj-alpha",
  generated_at: "2025-09-01T00:00:00Z",
  root_ids: ["m7-milestone"],
  nodes: [
    {
      schema_version: schemaVersionV1(),
      node_id: "m7-milestone",
      kind: "milestone",
      identifier: "M7",
      title: "Shared Client and Desktop Alpha",
      state: "In Progress",
      state_category: "in_progress",
      children: ["app-shell", "desktop-alpha", "hosted-auth", "follow-up", "skip-target"],
      blocked_by: [],
      labels: ["desktop"],
    },
    {
      schema_version: schemaVersionV1(),
      node_id: "app-shell",
      kind: "issue",
      identifier: "COE-450",
      title: "Desktop follow-on review",
      state: "Todo",
      state_category: "todo",
      parent_id: "m7-milestone",
      children: [],
      blocked_by: ["COE-449"],
      labels: ["desktop", "recovery"],
    },
    {
      schema_version: schemaVersionV1(),
      node_id: "desktop-alpha",
      kind: "issue",
      identifier: "COE-449",
      title: "Replace stubs with functional app",
      state: "In Progress",
      state_category: "in_progress",
      parent_id: "m7-milestone",
      children: [],
      blocked_by: [],
      labels: ["transport"],
    },
    {
      schema_version: schemaVersionV1(),
      node_id: "hosted-auth",
      kind: "issue",
      identifier: "COE-452",
      title: "Hosted auth placeholders",
      state: "Todo",
      state_category: "todo",
      parent_id: "m7-milestone",
      children: [],
      blocked_by: ["COE-449"],
      labels: ["hosted"],
    },
    {
      schema_version: schemaVersionV1(),
      node_id: "skip-target",
      kind: "issue",
      identifier: "COE-453",
      title: "Nested hosted follow-up",
      state: "Todo",
      state_category: "todo",
      parent_id: "m7-milestone",
      children: [],
      blocked_by: ["COE-449", "COE-450"],
      labels: ["hosted"],
    },
    {
      schema_version: schemaVersionV1(),
      node_id: "follow-up",
      kind: "issue",
      identifier: "COE-451",
      title: "Released prerequisite detail",
      state: "Todo",
      state_category: "todo",
      parent_id: "m7-milestone",
      children: [],
      blocked_by: ["completed-prereq"],
      labels: ["transport"],
    },
    {
      schema_version: schemaVersionV1(),
      node_id: "completed-prereq",
      kind: "issue",
      identifier: "COE-448",
      title: "Completed prerequisite",
      state: "Done",
      state_category: "done",
      parent_id: "m7-milestone",
      children: [],
      blocked_by: [],
      labels: ["transport"],
    },
  ],
};

const projectSetTaskGraph: TaskGraphSnapshot = {
  ...taskGraph,
  nodes: taskGraph.nodes.map((node) => {
    const beta = ["hosted-auth", "skip-target"].includes(node.node_id);
    return {
      ...node,
      project_id: beta ? "proj-beta" : "proj-alpha",
      project_slug: beta ? "beta-project" : "alpha-project",
      project_name: beta ? "Beta Project" : "Alpha Project",
    };
  }),
};

const sharedProjectGroupingTaskGraph: TaskGraphSnapshot = {
  schema_version: schemaVersionV1(),
  project_id: "project-set",
  generated_at: "2025-09-01T00:00:00Z",
  root_ids: [],
  nodes: projectGroupingFixture.map((item) => ({
    schema_version: schemaVersionV1(),
    node_id: item.id,
    kind: "issue",
    identifier: item.id,
    title: item.title,
    state: item.tracker_state,
    state_category: item.tracker_state === "Done"
      ? "done"
      : item.tracker_state === "In Progress"
        ? "in_progress"
        : "todo",
    project_id: item.project_id ?? undefined,
    project_slug: item.project_slug ?? undefined,
    project_name: item.project_name ?? undefined,
    children: [],
    blocked_by: item.blocked_by,
    run_id: item.id,
    labels: item.workspace_label ? [item.workspace_label] : [],
  })),
};

const runEvents: RunEventPage = {
  schema_version: schemaVersionV1(),
  run_id: "COE-449",
  events: [
    {
      sequence: 1,
      event_id: "evt-action",
      happened_at: "2025-09-01T00:00:05Z",
      kind: "ActionEvent",
      summary: "action",
      payload: { tool_name: "terminal", command: "npm test -- apps/desktop" },
    },
    {
      sequence: 2,
      event_id: "evt-observation",
      happened_at: "2025-09-01T00:00:06Z",
      kind: "ObservationEvent",
      summary: "A long observation detail should receive the full activity card width.\nSecond line stays hidden until expanded.",
    },
    {
      sequence: 3,
      event_id: "evt-item-started",
      happened_at: "2025-09-01T00:00:07Z",
      kind: "item/started",
      summary: "event: item/started",
    },
    {
      sequence: 4,
      event_id: "evt-item-completed",
      happened_at: "2025-09-01T00:00:08Z",
      kind: "item/completed",
      summary: "event: item/completed",
    },
    {
      sequence: 5,
      event_id: "evt-token-usage",
      happened_at: "2025-09-01T00:00:09Z",
      kind: "codex.thread/tokenUsage/updated",
      summary: "Codex token usage: 5539770 input, 35185 output, 5271680 cache",
    },
    {
      sequence: 6,
      event_id: "evt-diff-updated",
      happened_at: "2025-09-01T00:00:10Z",
      kind: "codex.turn/diff/updated",
      summary: "Codex diff updated",
    },
  ],
};

const runDetail: RunDetail = {
  schema_version: schemaVersionV1(),
  run_id: "COE-449",
  issue_id: "issue-coe-449",
  issue_identifier: "COE-449",
  worker_id: "worker-alpha",
  status: "running",
  claimed_at: "2025-09-01T00:00:00Z",
  started_at: "2025-09-01T00:00:30Z",
  turn_count: 3,
  max_turns: 8,
  input_tokens: 4500,
  output_tokens: 1100,
  cache_read_tokens: 600,
  runtime_seconds: 90,
  workspace_path: "/tmp/opensymphony/projects/COE-449",
  safe_actions: {
    retry: false,
    cancel: true,
    rehydrate: true,
    detach: false,
  },
};

const changedFiles: ChangedFileEntry[] = [
  {
    path: "src/config.ts",
    change_kind: "modified",
    lines_added: 12,
    lines_removed: 3,
  },
];

const fileDiff: FileDiffPage = {
  schema_version: schemaVersionV1(),
  run_id: "COE-449",
  file_path: "src/config.ts",
  hunks: [
    {
      file_path: "src/config.ts",
      header: "@@ -1 +1 @@",
      start_line: 1,
      old_line_count: 1,
      new_line_count: 1,
      lines: [{ type: "addition", line: "export const gateway = true;" }],
    },
  ],
  total_lines_added: 12,
  total_lines_removed: 3,
};

function buildTransport(opts?: {
  failHealth?: boolean;
  failTaskGraphStructured?: boolean;
  taskGraph?: TaskGraphSnapshot;
  runDetails?: RunDetail[];
}): MockGatewayTransport {
  if (opts?.failHealth) {
    class AlwaysFailHealthTransport extends MockGatewayTransport {
      override async health(): Promise<never> {
        throw new Error("simulated health probe failure");
      }
    }
    return new AlwaysFailHealthTransport({
      baseUri: "http://127.0.0.1:2468",
      health: capabilities,
      snapshot: dashboard,
      taskGraph,
      runDetails: [runDetail],
    });
  }
  if (opts?.failTaskGraphStructured) {
    class StructuredTaskGraphFailureTransport extends MockGatewayTransport {
      override async taskGraph(): Promise<never> {
        throw { type: "Gateway", message: "simulated structured task graph failure" };
      }
    }
    return new StructuredTaskGraphFailureTransport({
      baseUri: "http://127.0.0.1:2468",
      health: capabilities,
      snapshot: dashboard,
      taskGraph,
      runDetails: [runDetail],
    });
  }
  return new MockGatewayTransport({
    baseUri: "http://127.0.0.1:2468",
    health: capabilities,
    snapshot: dashboard,
    taskGraph: opts?.taskGraph ?? taskGraph,
    // Map the desktop-alpha task graph node to the COE-449 run detail so
    // the actual mock gateway response drives the run detail panel.
    runDetails: [
      runDetail,
      { ...runDetail, run_id: "desktop-alpha", issue_id: "desktop-alpha" },
      ...(opts?.runDetails ?? []),
    ],
    runFiles: [
      { runId: "COE-449", files: changedFiles },
      { runId: "desktop-alpha", files: changedFiles },
    ],
    runDiffs: [
      { runId: "COE-449", filePath: "src/config.ts", diff: fileDiff },
      { runId: "desktop-alpha", filePath: "src/config.ts", diff: { ...fileDiff, run_id: "desktop-alpha" } },
    ],
    runEvents: [
      runEvents,
      { ...runEvents, run_id: "desktop-alpha" },
    ],
  });
}

class LiveEventTransport extends MockGatewayTransport {
  snapshotReads = 0;
  activeStreams = 0;
  subscriptions: Array<{ sequence: number; partition: string } | undefined> = [];
  private queuedEvents: Array<GatewayEnvelope | null> = [];
  private resolveNext: ((event: GatewayEnvelope | null) => void) | null = null;
  private liveTaskGraph: TaskGraphSnapshot | null = null;
  private liveSnapshot: DashboardSnapshot | null = null;
  private nextSnapshotError: Error | null = null;

  emit(event: GatewayEnvelope): void {
    this.push(event);
  }

  failNextSnapshot(message: string): void {
    this.nextSnapshotError = new Error(message);
  }

  endStream(): void {
    this.push(null);
  }

  setSnapshot(snapshot: DashboardSnapshot): void {
    this.liveSnapshot = snapshot;
  }

  override async snapshot(): Promise<DashboardSnapshot> {
    this.snapshotReads += 1;
    if (this.nextSnapshotError) {
      const error = this.nextSnapshotError;
      this.nextSnapshotError = null;
      throw error;
    }
    return this.liveSnapshot ?? super.snapshot();
  }

  setTaskGraph(snapshot: TaskGraphSnapshot): void {
    this.liveTaskGraph = snapshot;
  }

  override async taskGraph(projectId: string): Promise<TaskGraphSnapshot> {
    return this.liveTaskGraph ?? super.taskGraph(projectId);
  }

  override async *events(
    fromCursor?: { sequence: number; partition: string },
  ): AsyncIterable<GatewayEnvelope> {
    this.subscriptions.push(fromCursor);
    this.activeStreams += 1;
    try {
      while (true) {
        const event = this.queuedEvents.length > 0
          ? this.queuedEvents.shift()!
          : await new Promise<GatewayEnvelope | null>((resolve) => {
              this.resolveNext = resolve;
            });
        if (!event) return;
        if (
          fromCursor
          && event.cursor.partition === fromCursor.partition
          && event.cursor.sequence <= fromCursor.sequence
        ) {
          continue;
        }
        yield event;
      }
    } finally {
      this.activeStreams -= 1;
    }
  }

  override async close(): Promise<void> {
    this.endStream();
    await super.close();
  }

  private push(event: GatewayEnvelope | null): void {
    if (this.resolveNext) {
      const resolve = this.resolveNext;
      this.resolveNext = null;
      resolve(event);
      return;
    }
    this.queuedEvents.push(event);
  }
}

function buildModelProfileController(
  initial = defaultModelProfiles(),
): ModelProfileController & { saved: ModelConfigurationProfile[] } {
  const saved = initial.map((profile) => ({
    ...profile,
    harnesses: [...profile.harnesses],
  }));
  return {
    saved,
    async listProfiles() {
      return saved;
    },
    async storeProfile(profile) {
      const index = saved.findIndex((candidate) => candidate.id === profile.id);
      if (index >= 0) {
        saved[index] = profile;
      } else {
        saved.push(profile);
      }
      return profile;
    },
    async setActiveProfile(profileId) {
      const active = saved.find((profile) => profile.id === profileId);
      if (!active) {
        throw new Error(`Unknown model profile: ${profileId}`);
      }
      saved.forEach((profile) => {
        profile.active = profile.id === profileId;
      });
      return active;
    },
    async removeProfile(profileId) {
      const index = saved.findIndex((profile) => profile.id === profileId);
      if (index < 0) {
        throw new Error(`Unknown model profile: ${profileId}`);
      }
      if (saved.length <= 1) {
        throw new Error("Cannot remove the last model profile");
      }
      saved.splice(index, 1);
      if (!saved.some((profile) => profile.active) && saved[0]) {
        saved[0].active = true;
      }
      return saved;
    },
  };
}

function flushAsync(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, 0));
}

async function flushMicrotasks(iterations = 20): Promise<void> {
  for (let i = 0; i < iterations; i++) {
    await Promise.resolve();
  }
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

async function expandSettingsPanel(
  root: HTMLElement,
  panel: "connection" | "model",
  readySelector: string,
): Promise<void> {
  const toggle = root.querySelector(
    `[data-toggle-settings='${panel}']`,
  ) as HTMLButtonElement;
  expect(toggle).not.toBeNull();
  toggle.click();
  await flushUntil(() => root.querySelector(readySelector) !== null);
}

describe("OpenSymphonyApp mount", () => {
  it("flushUntil rejects with a clear timeout message instead of returning silently", async () => {
    // Regression coverage for the reviewer finding that exhausted
    // flushUntil iterations used to resolve silently, which masked the
    // real failure inside a later null assertion.
    await expect(flushUntil(() => false, 4)).rejects.toThrow(
      /timed out after 4 iterations/,
    );
  });

  it("mounts the shared app shell with the marker attribute", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      title: "OpenSymphony Desktop",
      transport: buildTransport(),
      graphAdapter: createFixtureGraphAdapter(),
      codeGraphAdapter: createFixtureCodeGraphAdapter(),
    });
    await flushUntil(
      () =>
        root.querySelector(".os-app[data-opensymphony-app-shell='mounted']") !==
        null,
    );

    expect(
      root.querySelector(".os-app[data-opensymphony-app-shell='mounted']"),
    ).not.toBeNull();
    expect(root.querySelector(".os-app[data-mode='desktop']")).not.toBeNull();
    expect(root.textContent).toContain("OpenSymphony Desktop");

    await handle.destroy();
    expect(root.children.length).toBe(0);
  });

  it("keeps dark-mode tabs, changed-file rows, and graph selections readable", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      title: "OpenSymphony Desktop",
      transport: buildTransport(),
    });

    await flushUntil(
      () =>
        root.querySelector(".os-app[data-opensymphony-app-shell='mounted']") !==
        null,
    );

    const styleText = root.querySelector("style")?.textContent ?? "";
    expect(styleText).toContain("@media (prefers-color-scheme: dark)");
    expect(styleText).toContain(
      ".os-view-tab, .os-plan-tab, .os-changed-file",
    );
    expect(styleText).toContain(".os-changed-file .os-file-path");
    expect(styleText).toContain(".os-changed-file .os-file-stats");
    expect(styleText).toContain(".os-knowledge-stage { background: #e7ebef;");
    expect(styleText).toContain(".os-kg-label.is-selected");
    expect(styleText).toContain(".os-kg-list li.is-selected");
    expect(styleText).toContain(".os-kg-inspector dt, .os-kg-inspector p");

    await handle.destroy();
  });

  it("lays out status, task graph, run detail, and activity panels", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport: buildTransport(),
      graphAdapter: createFixtureGraphAdapter(),
    });

    await flushUntil(
      () => root.querySelector("[data-node-id='desktop-alpha']") !== null,
    );

    expect(root.querySelector("[data-testid='status-strip']")).not.toBeNull();
    expect(root.querySelector(".os-status-panel")).toBeNull();
    expect(root.querySelector(".os-profile-panel")).toBeNull();
    expect(root.querySelector("[data-testid='graph-hero']")).not.toBeNull();
    expect(root.querySelector("[data-testid='graph-hero'] h2")?.textContent).toBe("Graph Surface");
    expect(root.querySelector(".os-run-detail-panel h2")?.textContent).toBe("Run Detail");
    expect(root.querySelector(".os-run-evidence-panel h2")?.textContent).toBe("Inspector");
    expect(root.querySelectorAll("[data-pane-resizer]")).toHaveLength(1);
    (root.querySelector("[data-pane-resizer='lower-columns']") as HTMLElement).dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true }));
    expect((root.querySelector("[data-testid='workspace-lower-columns']") as HTMLElement).style.getPropertyValue("--os-left-column")).toBe("52%");
    expect(root.querySelector("[data-profile-label]")).toBeNull();
    expect(root.querySelector(".os-strip-metrics")).not.toBeNull();
    expect(root.querySelector("[data-project-id='proj-alpha']")).toBeNull();
    expect(root.querySelectorAll("[data-testid='event-log-mini'] li")).toHaveLength(2);
    const compactEvents = root.querySelector("[data-testid='event-log-mini']");
    expect(compactEvents?.textContent).not.toContain("codex.thread/tokenUsage/updated");
    expect(compactEvents?.textContent).not.toContain("codex.turn/diff/updated");
    expect(compactEvents?.textContent).not.toContain("Codex token usage");
    expect(compactEvents?.textContent).not.toContain("Codex diff updated");
    expect(root.textContent).not.toContain("should not render in compact status");
    (root.querySelector("[data-open-event-log]") as HTMLButtonElement).click();
    await flushUntil(() => root.querySelector("[data-testid='event-log-modal']") !== null);
    expect(root.querySelector("[data-testid='event-log-modal']")?.textContent).toContain("should not render in compact status");
    expect(root.querySelector("[data-testid='event-log-modal']")?.textContent).not.toContain("Codex token usage");
    (root.querySelector("[data-close-event-log]") as HTMLButtonElement).click();
    await flushUntil(() => root.querySelector("[data-testid='event-log-modal']") === null);
    expect(root.textContent).not.toContain("1 running, 2 done, 1 failed");
    expect(root.querySelector("[data-tg-create='milestone']")).toBeNull();
    expect(root.querySelector("[data-tg-create='issue']")).toBeNull();
    expect(root.querySelector("[data-tg-edit]")).toBeNull();
    expect(root.querySelector("[data-tg-deps]")).toBeNull();
    expect(root.querySelector("[data-tg-comment]")).toBeNull();
    expect(root.querySelector("[data-tg-create-child]")).toBeNull();
    expect(root.querySelector("[data-testid='task-graph-visualization']")).not.toBeNull();
    expect(root.querySelector(".os-project-group-header")).toBeNull();
    expect(root.querySelector("[data-testid='task-graph-link']")).not.toBeNull();
    expect(root.querySelector(".os-task-graph-link-skip")).not.toBeNull();
    // Skip arrows route through the left gutter with rounded corners and a
    // per-source hue/lane instead of sharp L-shapes.
    expect(root.querySelector(".os-task-graph-link-skip")?.getAttribute("d")).toMatch(/ H \S+ Q .+ V .+ Q .+ H /);
    expect(root.querySelector(".os-task-graph-link-skip")?.getAttribute("class")).toMatch(/os-tg-hue-\d/);
    // The hue marker is applied via CSS, not an inline `marker-end` that the
    // base `.os-task-graph-link` rule would override to the default arrow.
    expect(root.querySelector(".os-task-graph-link-skip")?.hasAttribute("marker-end")).toBe(false);
    const shellStyleText = Array.from(root.querySelectorAll("style"))
      .map((style) => style.textContent ?? "")
      .join("\n");
    expect(shellStyleText).toMatch(/\.os-tg-hue-0 \{[^}]*marker-end: url\(#os-task-arrow-0\)/);
    expect((root.querySelector("[data-node-id='app-shell']") as HTMLElement).style.getPropertyValue("--os-lane")).toBe("1");
    expect((root.querySelector("[data-node-id='app-shell']") as HTMLElement).style.getPropertyValue("--os-node-indent")).toBe("34px");
    expect((root.querySelector("[data-node-id='app-shell']") as HTMLElement).style.getPropertyValue("--os-node-height")).toBe("44px");
    // Dependencies are read from the connector glyph + arrows now, not a text
    // line on the card: a downstream-only blocker shows ">", a blocked task "<".
    expect(root.querySelector("[data-node-id='desktop-alpha'] .os-node-gutter")?.textContent).toContain(">");
    expect(root.querySelector("[data-node-id='app-shell'] .os-node-gutter")?.textContent).toContain("<");
    expect(root.querySelector("[data-node-id='desktop-alpha'] [data-testid='dependency-suffix']")).toBeNull();
    expect(root.querySelector("[data-node-id='app-shell'] .os-badge-blocker")).toBeNull();
    expect(root.querySelector("[data-node-id='desktop-alpha'] .os-badge-blocker")).not.toBeNull();
    expect(root.textContent).not.toContain("blocked by COE-448");
    await flushUntil(() => root.querySelector(".os-run-head strong")?.textContent === "COE-449");

    taskGraph.root_ids.forEach((rootId) => {
      expect(root.querySelector(`[data-node-id='${rootId}']`)).not.toBeNull();
    });

    await flushUntil(() => root.querySelector(".os-run-grid") !== null);
    await flushUntil(() => root.querySelector("[data-testid='changed-file-item']") !== null);

    const runSection = root.querySelector(".os-run-grid");
    expect(runSection).not.toBeNull();
    // The issue identifier is rendered in the .os-run-head strip, not
    // inside the .os-run-grid metrics block. Verify the run detail
    // panel reflects the navigation event with the mock gateway response.
    expect(root.querySelector(".os-run-head strong")?.textContent).toBe("COE-449");
    expect(root.querySelector(".os-pill")?.textContent).toBe("running");
    expect(root.querySelector("[data-testid='dependency-detail']")?.textContent).toContain("blocks COE-450, COE-452");
    expect(root.querySelector(".os-run-detail-panel [data-testid='changed-file-list']")).not.toBeNull();
    expect(root.querySelector("[data-testid='graph-view-toggle']")).not.toBeNull();
    expect(root.querySelector(".os-run-evidence-panel [data-testid='evidence-toggle']")).not.toBeNull();
    expect(root.querySelector(".os-run-evidence-panel [data-evidence-view='knowledge']")).toBeNull();
    expect(root.querySelector(".os-run-evidence-panel [data-testid='file-diff']")).not.toBeNull();

    (root.querySelector("[data-evidence-view='activity']") as HTMLButtonElement).click();
    await flushUntil(() => root.querySelector(".os-run-evidence-panel [data-testid='run-activity']") !== null);
    expect(root.querySelector(".os-run-evidence-panel [data-testid='run-activity']")).not.toBeNull();
    expect(root.querySelector("[data-testid='activity-lifecycle']")?.textContent).toContain("1 started, 1 completed");
    const activityEntries = Array.from(root.querySelectorAll("[data-testid='run-activity-entry']"));
    expect(activityEntries.map((entry) => entry.getAttribute("data-event-id"))).toEqual([
      "evt-observation",
      "evt-action",
    ]);
    const activity = root.querySelector("[data-testid='run-activity']");
    expect(activity?.textContent).not.toContain("codex.thread/tokenUsage/updated");
    expect(activity?.textContent).not.toContain("codex.turn/diff/updated");
    expect(activity?.textContent).not.toContain("Codex token usage");
    expect(activity?.textContent).not.toContain("Codex diff updated");
    expect(root.querySelector(".os-activity-entry-action-event .os-activity-preview")?.textContent).toBe("terminal: npm test -- apps/desktop");
    expect(root.querySelector(".os-activity-entry-action-event .os-activity-detail")).toBeNull();
    expect(root.querySelector(".os-activity-entry-observation-event .os-activity-meta strong")?.textContent).toBe("ObservationEvent");
    expect(root.querySelector(".os-activity-entry-observation-event .os-activity-preview")?.textContent).toContain("Second line stays hidden");
    expect(root.querySelector(".os-activity-entry-observation-event .os-activity-detail")?.textContent).toContain("Second line stays hidden until expanded.");

    (root.querySelector(".os-activity-entry-observation-event [data-activity-toggle]") as HTMLButtonElement).click();
    await flushUntil(
      () => root.querySelector(".os-activity-entry-observation-event .os-activity-detail") === null,
    );
    expect(root.querySelector(".os-activity-entry-observation-event [data-activity-toggle]")?.getAttribute("aria-expanded")).toBe("false");

    (root.querySelector("[data-testid='changed-file-item']") as HTMLButtonElement).click();
    await flushUntil(() => root.querySelector(".os-run-evidence-panel [data-testid='file-diff']") !== null);
    expect(root.querySelector("[data-evidence-view='diff']")?.classList.contains("is-selected")).toBe(true);

    const originalGetContext = HTMLCanvasElement.prototype.getContext;
    const originalConsoleError = console.error;
    const fillStyles: string[] = [];
    const consoleError = jest.spyOn(console, "error").mockImplementation((...args: unknown[]) => {
      const first = args[0];
      const message = first instanceof Error ? first.message : String(first);
      if (message.includes("HTMLCanvasElement.prototype.getContext")) return;
      originalConsoleError(...args);
    });
    const getContext = jest.spyOn(HTMLCanvasElement.prototype, "getContext").mockImplementation(function (
      this: HTMLCanvasElement,
      contextId: string,
      contextOptions?: unknown,
    ) {
      if (contextId.startsWith("webgl")) {
        return originalGetContext.call(this, contextId as "webgl", contextOptions as WebGLContextAttributes);
      }
      if (contextId !== "2d") return null;
      return {
        setTransform: jest.fn(),
        fillRect: jest.fn(),
        beginPath: jest.fn(),
        closePath: jest.fn(),
        moveTo: jest.fn(),
        lineTo: jest.fn(),
        stroke: jest.fn(),
        arc: jest.fn(),
        fill: jest.fn(),
        set fillStyle(value: string) {
          fillStyles.push(value);
        },
        set strokeStyle(_value: string) {},
        set lineWidth(_value: number) {},
        set globalAlpha(_value: number) {},
      } as unknown as CanvasRenderingContext2D;
    });
    try {
      (root.querySelector("[data-graph-view='knowledge']") as HTMLButtonElement).click();
      await flushUntil(() => root.querySelector(".os-graph-hero-panel [data-testid='knowledge-graph-renderer']")?.getAttribute("data-layout-status") === "ready");
      expect(root.querySelector(".os-graph-hero-panel h2")?.textContent).toBe("Graph Surface");
      expect(root.querySelector("[data-graph-view='knowledge']")?.classList.contains("is-selected")).toBe(true);
      expect(root.querySelector(".os-graph-hero-panel [data-testid='knowledge-graph-canvas']")).not.toBeNull();
      expect(root.querySelector(".os-graph-hero-panel [data-testid='knowledge-graph-canvas']")?.getAttribute("data-nonblank")).toBe("true");
      expect(fillStyles).toContain("#eef1f4");
      expect(getContext.mock.calls.some(([contextId]) => String(contextId).startsWith("webgl"))).toBe(true);
      expect(root.querySelector("[data-testid='knowledge-graph-metrics']")?.textContent).toContain(`${fixtureGraphSnapshot.nodes.length} nodes`);
      const fallbackButtons = Array.from(root.querySelectorAll<HTMLButtonElement>(".os-kg-list [data-kg-node-id]"));
      expect(fallbackButtons.map((button) => button.dataset.kgNodeId).sort()).toEqual(
        fixtureGraphSnapshot.nodes.map((node) => node.id).sort(),
      );
      const nextFocus = jest.spyOn(fallbackButtons[1]!, "focus").mockImplementation(() => {});
      fallbackButtons[0]?.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true }));
      expect(nextFocus).toHaveBeenCalled();
      nextFocus.mockRestore();
      // Entity list and inspector live in the lower workspace columns, not
      // inside the graph hero, so the stage keeps the full hero height.
      expect(root.querySelector(".os-graph-hero-panel .os-kg-list")).toBeNull();
      expect(root.querySelector(".os-graph-hero-panel [data-testid='knowledge-graph-inspector']")).toBeNull();
      expect(root.querySelector(".os-knowledge-lower-panel [data-kg-node-id='concept:coe-465']")).not.toBeNull();
      expect(root.querySelector(".os-knowledge-lower-panel [data-testid='knowledge-graph-inspector']")).not.toBeNull();
      expect(root.textContent).not.toContain("unknown_frontmatter");
      expect(root.textContent).not.toContain("frontmatter_summary");
      // TODO(COE-471): migrate the COE-468 search/filter/inspector/raw-frontmatter controls
      // into the live canvas renderer; COE-469 covers the current live canvas, fallback list,
      // keyboard navigation, and privacy surface.
      expect(root.querySelector("[data-kg-search]")).toBeNull();
      expect(root.querySelector("[data-kg-raw-toggle]")).toBeNull();
      expect(root.querySelector(".os-run-evidence-panel [data-testid='knowledge-graph-renderer']")).toBeNull();
      (root.querySelector("[data-graph-view='code']") as HTMLButtonElement).click();
      await flushUntil(() => root.querySelector("[data-testid='code-graph-renderer']") !== null);
      expect(root.querySelector("[data-testid='workspace-pane-shell']")?.getAttribute("data-graph-surface")).toBe("code");
      expect(root.querySelector("[data-testid='code-graph-structure-list']")).not.toBeNull();
      expect(root.querySelector("[data-testid='code-graph-detail']")).not.toBeNull();
    } finally {
      consoleError.mockRestore();
      getContext.mockRestore();
    }

    (root.querySelector("[data-graph-view='task']") as HTMLButtonElement).click();
    await flushUntil(() => root.querySelector("[data-testid='task-graph-visualization']") !== null);
    expect(root.querySelector(".os-graph-hero-panel h2")?.textContent).toBe("Graph Surface");
    expect((root.querySelector("[data-testid='workspace-lower-columns']") as HTMLElement).style.getPropertyValue("--os-left-column")).toBe("52%");

    await handle.destroy();
  });

  it("restores graph surface state and lower columns across round trips", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const getContext = jest.spyOn(HTMLCanvasElement.prototype, "getContext").mockImplementation(function (
      contextId: string,
    ) {
      if (contextId.startsWith("webgl")) {
        return null;
      }
      if (contextId !== "2d") return null;
      return {
        setTransform: jest.fn(),
        fillRect: jest.fn(),
        beginPath: jest.fn(),
        closePath: jest.fn(),
        moveTo: jest.fn(),
        lineTo: jest.fn(),
        stroke: jest.fn(),
        arc: jest.fn(),
        fill: jest.fn(),
        set fillStyle(_value: string) {},
        set strokeStyle(_value: string) {},
        set lineWidth(_value: number) {},
        set globalAlpha(_value: number) {},
      } as unknown as CanvasRenderingContext2D;
    });
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport: buildTransport(),
      graphAdapter: createFixtureGraphAdapter(),
    });

    try {
      await flushUntil(() => root.querySelector("[data-node-id='desktop-alpha']") !== null);
      const resizer = () => root.querySelector("[data-pane-resizer='lower-columns']") as HTMLElement;
      const lowerColumns = () => root.querySelector("[data-testid='workspace-lower-columns']") as HTMLElement;

      resizer().dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true }));
      expect(lowerColumns().style.getPropertyValue("--os-left-column")).toBe("52%");

      (root.querySelector("[data-graph-view='knowledge']") as HTMLButtonElement).click();
      await flushUntil(() => root.querySelector("[data-kg-node-id='concept:coe-465']") !== null);
      // Knowledge defaults to a narrow entity list beside the inspector.
      expect(lowerColumns().style.getPropertyValue("--os-left-column")).toBe("34%");
      (root.querySelector("[data-kg-node-id='concept:coe-465']") as HTMLButtonElement).click();
      await flushUntil(() => root.querySelector(".os-kg-list li.is-selected [data-kg-node-id='concept:coe-465']") !== null);
      resizer().dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowLeft", bubbles: true }));
      expect(lowerColumns().style.getPropertyValue("--os-left-column")).toBe("32%");

      (root.querySelector("[data-graph-view='code']") as HTMLButtonElement).click();
      await flushUntil(() => root.querySelector("[data-testid='code-graph-renderer']") !== null);
      expect(lowerColumns().style.getPropertyValue("--os-left-column")).toBe("50%");

      (root.querySelector("[data-graph-view='task']") as HTMLButtonElement).click();
      await flushUntil(() => root.querySelector("[data-testid='task-graph-visualization']") !== null);
      expect(lowerColumns().style.getPropertyValue("--os-left-column")).toBe("52%");
      expect(root.querySelector("[data-node-id='desktop-alpha']")?.classList.contains("is-selected")).toBe(true);

      (root.querySelector("[data-graph-view='knowledge']") as HTMLButtonElement).click();
      await flushUntil(() => root.querySelector(".os-kg-list li.is-selected [data-kg-node-id='concept:coe-465']") !== null);
      expect(lowerColumns().style.getPropertyValue("--os-left-column")).toBe("32%");
    } finally {
      getContext.mockRestore();
      await handle.destroy();
    }
  });

  it("gives every skip-level blocker its own routing lane, beyond the hue palette", async () => {
    const sourceCount = 7;
    const nodeFor = (index: number, overrides: Partial<TaskGraphNode>): TaskGraphNode => ({
      schema_version: schemaVersionV1(),
      node_id: `lane-node-${index}`,
      kind: "issue",
      identifier: `LANE-${index}`,
      title: `Lane test ${index}`,
      state: "In Progress",
      state_category: "in_progress",
      children: [],
      blocked_by: [],
      labels: [],
      ...overrides,
    });
    const sources = Array.from({ length: sourceCount }, (_, index) => nodeFor(index, {}));
    // Targets sit far below their blockers so every link is a skip (span > 1).
    const targets = Array.from({ length: sourceCount }, (_, index) => nodeFor(100 + index, {
      state: "Todo",
      state_category: "todo",
      blocked_by: [`LANE-${index}`],
    }));
    const laneGraph: TaskGraphSnapshot = {
      schema_version: schemaVersionV1(),
      project_id: "proj-alpha",
      generated_at: "2025-09-01T00:00:00Z",
      root_ids: [sources[0].node_id],
      nodes: [...sources, ...targets],
    };
    const root = document.createElement("div");
    document.body.appendChild(root);
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport: buildTransport({ taskGraph: laneGraph }),
    });

    try {
      await flushUntil(() => root.querySelectorAll(".os-task-graph-link-skip").length >= sourceCount);
      const skips = Array.from(root.querySelectorAll(".os-task-graph-link-skip"));
      const routeXs = new Set(skips.map((path) => path.getAttribute("d")?.match(/Q (\S+) /)?.[1]));
      // Seven blockers → seven distinct gutter rails, even though the hue
      // palette (5 entries) cycles.
      expect(routeXs.size).toBe(sourceCount);
      const stage = root.querySelector<HTMLElement>("[data-testid='task-graph-visualization']");
      const gutter = Number.parseInt(stage?.style.getPropertyValue("--os-tg-gutter") ?? "0", 10);
      expect(gutter).toBeGreaterThanOrEqual(30 + sourceCount * 11);
    } finally {
      await handle.destroy();
    }
  });

  it("offers a home path back to the full Knowledge Graph after narrowing", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport: buildTransport(),
      graphAdapter: createFixtureGraphAdapter(),
    });

    try {
      await flushUntil(() => root.querySelector("[data-node-id='desktop-alpha']") !== null);
      (root.querySelector("[data-graph-view='knowledge']") as HTMLButtonElement).click();
      await flushUntil(() => root.querySelector(".os-kg-list [data-kg-node-id='concept:coe-465']") !== null);
      expect(root.querySelector("[data-testid='knowledge-graph-reset']")).toBeNull();

      // Keyboard focus narrows to the neighborhood; the home button appears.
      (root.querySelector(".os-kg-list [data-kg-node-id='concept:coe-465']") as HTMLButtonElement).focus();
      await flushUntil(() => root.querySelector("[data-testid='knowledge-graph-reset']") !== null);

      (root.querySelector("[data-testid='knowledge-graph-reset']") as HTMLButtonElement).click();
      await flushUntil(() => root.querySelector("[data-testid='knowledge-graph-reset']") === null);
      expect(root.querySelector(".os-kg-list li.is-selected")).toBeNull();

      // Escape offers the same escape hatch.
      (root.querySelector(".os-kg-list [data-kg-node-id='concept:coe-465']") as HTMLButtonElement).click();
      await flushUntil(() => root.querySelector("[data-testid='knowledge-graph-reset']") !== null);
      root.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
      await flushUntil(() => root.querySelector("[data-testid='knowledge-graph-reset']") === null);
    } finally {
      await handle.destroy();
    }
  });

  it("renders the selected concept's memory capsule and follows capsule links", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport: buildTransport(),
      graphAdapter: createFixtureGraphAdapter(),
    });

    try {
      await flushUntil(() => root.querySelector("[data-node-id='desktop-alpha']") !== null);
      (root.querySelector("[data-graph-view='knowledge']") as HTMLButtonElement).click();
      await flushUntil(() => root.querySelector(".os-kg-list [data-kg-node-id='concept:coe-465']") !== null);

      (root.querySelector(".os-kg-list [data-kg-node-id='concept:coe-465']") as HTMLButtonElement).click();
      await flushUntil(() => root.querySelector("[data-testid='knowledge-graph-capsule-body']") !== null);

      const capsule = root.querySelector("[data-testid='knowledge-graph-capsule']");
      expect(capsule?.textContent).toContain("Shared graph frontend package and reducers");
      expect(capsule?.textContent).toContain("issue: COE-465");
      expect(fixtureConceptDetail.links[0]?.target).toBe("tag:graph-view");

      // The capsule carries a copyable deep link to itself.
      const copy = root.querySelector<HTMLButtonElement>("[data-testid='knowledge-graph-copy-deeplink']");
      expect(copy?.dataset.kgCopyDeeplink).toBe("opensymphony://memory/local-default/concepts/issues/COE-465");

      // Breadcrumb reflects the drill trail back to the atlas.
      expect(root.querySelector("[data-testid='knowledge-graph-breadcrumb']")?.textContent).toContain("COE-465");

      // Following a capsule link selects the linked node and drills into
      // its area, even when starting from the atlas.
      (root.querySelector("[data-kg-link-target='tag:graph-view']") as HTMLButtonElement).click();
      await flushUntil(() =>
        root.querySelector("[data-testid='knowledge-graph-inspector'] h3")?.textContent === "graph-view",
      );
      expect(root.querySelector("[data-testid='knowledge-graph-inspector'] dl")?.textContent).toContain("tag");
      expect(root.querySelector("[data-testid='knowledge-graph-breadcrumb']")?.textContent).toContain("Graph View");
    } finally {
      await handle.destroy();
    }
  });

  it("surfaces capsule fetch failures with a retry that recovers", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    let failures = 1;
    const graphAdapter: GraphDataAdapter = {
      ...createFixtureGraphAdapter(),
      async getConceptDetail(bundleId, conceptId) {
        if (failures > 0) {
          failures -= 1;
          throw new Error("capsule endpoint offline");
        }
        return { ...fixtureConceptDetail, bundle_id: bundleId, concept_id: conceptId };
      },
    };
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport: buildTransport(),
      graphAdapter,
    });

    try {
      await flushUntil(() => root.querySelector("[data-node-id='desktop-alpha']") !== null);
      (root.querySelector("[data-graph-view='knowledge']") as HTMLButtonElement).click();
      await flushUntil(() => root.querySelector(".os-kg-list [data-kg-node-id='concept:coe-465']") !== null);

      (root.querySelector(".os-kg-list [data-kg-node-id='concept:coe-465']") as HTMLButtonElement).click();
      await flushUntil(() => root.querySelector("[data-testid='knowledge-graph-capsule-error']") !== null);
      expect(root.querySelector("[data-testid='knowledge-graph-capsule-error']")?.textContent).toContain("capsule endpoint offline");

      (root.querySelector("[data-testid='knowledge-graph-capsule-retry']") as HTMLButtonElement).click();
      await flushUntil(() => root.querySelector("[data-testid='knowledge-graph-capsule-body']") !== null);
    } finally {
      await handle.destroy();
    }
  });

  it("resolves deep links whose bundle is queued behind an in-flight graph load", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const bundleB = {
      ...fixtureGraphSnapshot,
      bundle_id: "bundle-b",
      cursor: { sequence: 1, partition: "memory-graph:bundle-b" },
      nodes: fixtureGraphSnapshot.nodes.map((node) => ({ ...node, bundle_id: "bundle-b" })),
    };
    let releaseFirstSnapshot: (() => void) | null = null;
    const firstSnapshotGate = new Promise<void>((resolve) => {
      releaseFirstSnapshot = resolve;
    });
    let snapshotCalls = 0;
    const graphAdapter: GraphDataAdapter = {
      ...createFixtureGraphAdapter(),
      async listBundles() {
        return {
          schema_version: schemaVersionV1(),
          bundles: [
            { id: "local-default", title: "Default", okf_version: "0.1", visibility: "private" as const, concept_count: 1 },
            { id: "bundle-b", title: "Bundle B", okf_version: "0.1", visibility: "private" as const, concept_count: 1 },
          ],
        };
      },
      async getGraphSnapshot(bundleId) {
        snapshotCalls += 1;
        if (snapshotCalls === 1) await firstSnapshotGate;
        return bundleId === "bundle-b" ? bundleB : fixtureGraphSnapshot;
      },
      async getConceptDetail(bundleId, conceptId) {
        return { ...fixtureConceptDetail, bundle_id: bundleId, concept_id: conceptId };
      },
    };
    const handle = renderOpenSymphonyApp({ root, mode: "desktop", transport: buildTransport(), graphAdapter });

    try {
      await flushUntil(() => root.querySelector("[data-graph-view='knowledge']") !== null);
      // Opening the pane starts a default-bundle load; the deep link lands
      // while that load is still in flight, so its bundle gets queued.
      (root.querySelector("[data-graph-view='knowledge']") as HTMLButtonElement).click();
      const openPromise = handle.openMemoryDeepLink("opensymphony://memory/bundle-b/concepts/issues/COE-465");
      await flushAsync();
      releaseFirstSnapshot?.();

      expect(await openPromise).toBe(true);
      expect(root.querySelector("[data-testid='knowledge-graph-inspector'] h3")?.textContent)
        .toContain("COE-465");
      await flushUntil(() => root.querySelector("[data-testid='knowledge-graph-capsule-body']") !== null);
    } finally {
      await handle.destroy();
    }
  });

  it("discards in-flight capsule responses superseded by an accepted graph refresh", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const transport = new LiveEventTransport({
      baseUri: "http://127.0.0.1:2468",
      health: capabilities,
      snapshot: dashboard,
      taskGraph,
      runDetails: [runDetail],
    });
    let releaseFirstDetail: (() => void) | null = null;
    const firstDetailGate = new Promise<void>((resolve) => {
      releaseFirstDetail = resolve;
    });
    let snapshotReads = 0;
    let detailCalls = 0;
    const graphAdapter: GraphDataAdapter = {
      ...createFixtureGraphAdapter(),
      async getGraphSnapshot() {
        snapshotReads += 1;
        return snapshotReads > 1
          ? {
              ...fixtureGraphSnapshot,
              cursor: { ...fixtureGraphSnapshot.cursor, sequence: 2 },
              metrics: { orphan_count: 0, broken_link_count: 0, stale_concept_count: 0, warning_count: 1 },
            }
          : fixtureGraphSnapshot;
      },
      async getConceptDetail(bundleId, conceptId) {
        detailCalls += 1;
        if (detailCalls === 1) await firstDetailGate;
        return {
          ...fixtureConceptDetail,
          bundle_id: bundleId,
          concept_id: conceptId,
          body_markdown: `# capsule fetch ${detailCalls}`,
        };
      },
    };
    const handle = renderOpenSymphonyApp({ root, mode: "desktop", transport, graphAdapter });

    try {
      await flushUntil(() => root.querySelector("[data-graph-view='knowledge']") !== null);
      (root.querySelector("[data-graph-view='knowledge']") as HTMLButtonElement).click();
      await flushUntil(() => root.querySelector(".os-kg-list [data-kg-node-id='concept:coe-465']") !== null);

      // Start the capsule fetch, then let an accepted refresh land while it
      // is still in flight.
      (root.querySelector(".os-kg-list [data-kg-node-id='concept:coe-465']") as HTMLButtonElement).click();
      await flushUntil(() => detailCalls === 1);
      transport.emit({
        schema_version: schemaVersionV1(),
        cursor: { sequence: 30, partition: "events" },
        entity_ref: { kind: "unknown", id: "memory-graph:local-default" },
        event_kind: "memory_graph_updated",
        emitted_at: "2026-06-28T00:02:00Z",
        payload: {
          schema_version: schemaVersionV1(),
          bundle_id: "local-default",
          cursor: { sequence: 2, partition: "memory-graph:local-default" },
          updated_at: "2026-06-28T00:02:00Z",
        },
      });
      await flushUntil(() => snapshotReads === 2);
      await flushUntil(() => root.querySelector("[data-testid='knowledge-graph-status']")?.textContent?.includes("Graph warnings") ?? false);

      // The pre-refresh response must be discarded and refetched, not
      // written back over the invalidated cache.
      releaseFirstDetail?.();
      await flushUntil(() =>
        root.querySelector("[data-testid='knowledge-graph-capsule-body']")?.textContent?.includes("capsule fetch 2") ?? false,
      );
      expect(detailCalls).toBe(2);
    } finally {
      await handle.destroy();
    }
  });

  it("navigates memory deep links into a drilled capsule and steps back out with Escape", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport: buildTransport(),
      graphAdapter: createFixtureGraphAdapter({
        bundles: graphVizFixtureBundleList,
        snapshot: graphVizFixtureSnapshot,
        communities: graphVizFixtureCommunityList,
        conceptDetail: (_bundleId, conceptId) => graphVizFixtureConceptDetail(conceptId),
      }),
    });

    try {
      await flushUntil(() => root.querySelector("[data-node-id='desktop-alpha']") !== null);

      expect(await handle.openMemoryDeepLink("not-a-deep-link")).toBe(false);
      expect(await handle.openMemoryDeepLink("opensymphony://memory/viz-workbench/concepts/missing")).toBe(false);

      const opened = await handle.openMemoryDeepLink(
        "opensymphony://memory/viz-workbench/concepts/concepts/code-intelligence-01",
      );
      expect(opened).toBe(true);

      // The deep link lands on the Knowledge Graph pane, drilled into the
      // concept's area with the capsule open.
      expect(root.querySelector("[data-testid='graph-hero']")?.getAttribute("data-active-graph-surface")).toBe("knowledge");
      const breadcrumb = () => root.querySelector("[data-testid='knowledge-graph-breadcrumb']")?.textContent ?? "";
      expect(breadcrumb()).toContain("Code Intelligence");
      expect(breadcrumb()).toContain("Tree-sitter Provider Skeleton");
      expect(root.querySelector("[data-testid='knowledge-graph-inspector'] h3")?.textContent)
        .toBe("Tree-sitter Provider Skeleton");
      await flushUntil(() => root.querySelector("[data-testid='knowledge-graph-capsule-body']") !== null);
      expect(root.querySelector("[data-testid='knowledge-graph-capsule-body']")?.textContent).toContain("Summary");

      // The drilled view is filtered to the area's full membership,
      // including multi-area concepts whose primary community differs.
      const areaMemberCount = graphVizFixtureSnapshot.communities
        .find((community) => community.id === "area:code-intelligence")!.node_ids.length;
      expect(root.querySelectorAll("[data-testid='knowledge-graph-node-list'] li").length).toBe(areaMemberCount);

      // Escape pops one level at a time: capsule → area → atlas.
      root.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
      await flushUntil(() => !breadcrumb().includes("Tree-sitter Provider Skeleton"));
      expect(breadcrumb()).toContain("Code Intelligence");

      root.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
      await flushUntil(() => root.querySelector("[data-testid='knowledge-graph-breadcrumb']") === null);
      expect(root.querySelectorAll("[data-testid='knowledge-graph-node-list'] li").length)
        .toBe(graphVizFixtureSnapshot.nodes.length);

      // Community deep links drill straight into the area.
      expect(await handle.openMemoryDeepLink("opensymphony://memory/viz-workbench/communities/area%3Agateway")).toBe(true);
      await flushUntil(() => breadcrumb().includes("Gateway"));

      // A community deep link into the already-drilled area lands on the
      // area view: the stale capsule selection is cleared, not kept.
      expect(await handle.openMemoryDeepLink("opensymphony://memory/viz-workbench/concepts/concepts/gateway-01")).toBe(true);
      await flushUntil(() => breadcrumb().includes("Gateway DTO Boundary Checklist"));
      expect(await handle.openMemoryDeepLink("opensymphony://memory/viz-workbench/communities/area%3Agateway")).toBe(true);
      await flushUntil(() => !breadcrumb().includes("Gateway DTO Boundary Checklist"));
      expect(breadcrumb()).toContain("Gateway");
    } finally {
      await handle.destroy();
    }
  });

  it("opens Code Graph deep links, loads symbol detail, and applies surface filters", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const fixtureCodeGraphAdapter = createFixtureCodeGraphAdapter();
    const graphRequests: Array<{ mode?: string; includeStale?: boolean; path?: string; symbolKey?: string }> = [];
    const repoRequests: Array<{ includeStale?: boolean }> = [];
    const codeGraphAdapter = {
      ...fixtureCodeGraphAdapter,
      async listRepos(options?: Parameters<typeof fixtureCodeGraphAdapter.listRepos>[0]) {
        repoRequests.push(options ?? {});
        return fixtureCodeGraphAdapter.listRepos(options);
      },
      async getGraphSnapshot(repoId: string, options?: Parameters<typeof fixtureCodeGraphAdapter.getGraphSnapshot>[1]) {
        graphRequests.push(options ?? {});
        return fixtureCodeGraphAdapter.getGraphSnapshot(repoId, options);
      },
    };
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport: buildTransport(),
      graphAdapter: createFixtureGraphAdapter(),
      codeGraphAdapter,
    });

    try {
      await flushUntil(() => root.querySelector("[data-node-id='desktop-alpha']") !== null);
      expect(await handle.openCodeDeepLink("opensymphony://code/opensymphony/unknown/value")).toBe(false);

      expect(await handle.openCodeDeepLink(
        "opensymphony://code/opensymphony/symbols/codeGraphReducer?depth=2&seed=code-fixture",
      )).toBe(true);
      await flushUntil(() => root.querySelector("[data-testid='code-graph-structure-list']") !== null);
      expect(root.querySelector("[data-testid='graph-hero']")?.getAttribute("data-active-graph-surface")).toBe("code");
      await flushUntil(() => root.querySelector("[data-testid='code-graph-detail'] h3")?.textContent === "codeGraphReducer");
      expect(root.querySelector("[data-testid='code-graph-raw-record']")).toBeNull();

      expect(await handle.openCodeDeepLink("opensymphony://code/opensymphony/files/packages/graph/src/index.ts")).toBe(true);
      await flushUntil(() => root.querySelector("[data-code-mode='file']")?.classList.contains("is-selected") ?? false);
      const focusApp = handle as unknown as { onCodeNodeFocused(nodeId: string): void; state: { codeGraph: { mode: string } } };
      focusApp.onCodeNodeFocused("symbol:codeGraphReducer");
      expect(focusApp.state.codeGraph.mode).toBe("file");
      const staleFilter = root.querySelector<HTMLInputElement>("[data-code-filter='freshness'][data-code-filter-value='stale']");
      expect(staleFilter).not.toBeNull();
      staleFilter!.click();
      await flushUntil(() => root.querySelectorAll("[data-testid='code-graph-structure-list'] li").length === 1);
      expect(root.querySelector("[data-testid='code-graph-structure-list']")?.textContent).toContain("codeGraphReducer");
      await flushUntil(() => graphRequests.some((request) => request.includeStale === true));
      const readsBeforeFilterReset = graphRequests.length;
      root.querySelector<HTMLButtonElement>("[data-code-filter-reset]")?.click();
      await flushUntil(() => graphRequests.length > readsBeforeFilterReset && graphRequests.at(-1)?.includeStale === false);
      await flushUntil(() => repoRequests.some((request) => request.includeStale === true));
      const readsBeforeUnknownFilter = graphRequests.length;
      root.querySelector<HTMLInputElement>("[data-code-filter='freshness'][data-code-filter-value='unknown']")?.click();
      await flushUntil(() => graphRequests.length > readsBeforeUnknownFilter && graphRequests.at(-1)?.includeStale === true);

      expect(await handle.openCodeDeepLink("opensymphony://code/opensymphony/files/packages/missing.ts")).toBe(false);
      expect(await handle.openCodeDeepLink("opensymphony://code/opensymphony/diff/base-rev/head-rev")).toBe(true);
      await flushUntil(() => root.querySelector("[data-code-mode='diff']")?.classList.contains("is-selected") ?? false);

      expect(await handle.openCodeDeepLink("opensymphony://code/opensymphony/atlas")).toBe(true);
      await flushUntil(() => root.querySelector("[data-code-mode='atlas']")?.classList.contains("is-selected") ?? false);
      const diffModeButton = root.querySelector<HTMLButtonElement>("[data-code-mode='diff']");
      expect(diffModeButton?.disabled).toBe(true);
      diffModeButton?.click();
      expect(root.querySelector("[data-code-mode='atlas']")?.classList.contains("is-selected")).toBe(true);
      const readsBeforeInvalidMode = graphRequests.length;
      root.querySelector<HTMLButtonElement>("[data-code-mode='file']")?.click();
      await Promise.resolve();
      expect(graphRequests.length).toBe(readsBeforeInvalidMode);
      const app = handle as unknown as {
        state: { codeGraph: { snapshot: { nodes: CodeGraphNode[] } | null; mode: string; filters: { pathPrefixes: string[]; communities: string[] } } };
        drillIntoCodeNode(node: CodeGraphNode): void;
      };
      const community = app.state.codeGraph.snapshot?.nodes.find((node) => node.kind === "community");
      expect(community).toBeDefined();
      app.drillIntoCodeNode(community!);
      await flushUntil(() => app.state.codeGraph.filters.communities.length === 1);
      expect(app.state.codeGraph.filters.pathPrefixes).toEqual([]);
      expect(app.state.codeGraph.mode).toBe("atlas");
      const directory = app.state.codeGraph.snapshot?.nodes.find((node) => node.kind === "directory");
      expect(directory).toBeDefined();
      app.drillIntoCodeNode(directory!);
      await flushUntil(() => app.state.codeGraph.filters.pathPrefixes.includes("packages/graph"));
      expect(app.state.codeGraph.mode).toBe("atlas");
    } finally {
      await handle.destroy();
    }
  });

  it("rejects Code Graph diff links when the overlay cannot be loaded", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const fixtureCodeGraphAdapter = createFixtureCodeGraphAdapter();
    const codeGraphAdapter = {
      ...fixtureCodeGraphAdapter,
      async getDiffOverlay(): Promise<never> {
        throw new Error("diff overlay unavailable");
      },
    };
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport: buildTransport(),
      codeGraphAdapter,
    });

    try {
      expect(await handle.openCodeDeepLink("opensymphony://code/opensymphony/diff/base-rev/head-rev")).toBe(false);
    } finally {
      await handle.destroy();
    }
  });

  it("shows a graph-record fallback when Code Graph symbol detail is unavailable", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const fixtureCodeGraphAdapter = createFixtureCodeGraphAdapter();
    const codeGraphAdapter = {
      ...fixtureCodeGraphAdapter,
      async getSymbolDetail(): Promise<never> {
        throw new Error("symbol detail unavailable");
      },
    };
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport: buildTransport(),
      codeGraphAdapter,
    });

    try {
      expect(await handle.openCodeDeepLink("opensymphony://code/opensymphony/symbols/codeGraphReducer")).toBe(true);
      await flushUntil(() => root.querySelector("[data-testid='code-graph-file-fallback']")?.textContent?.includes("Symbol detail unavailable") ?? false);
      expect(root.querySelector("[data-testid='code-graph-detail-loading']")).toBeNull();
      expect(await handle.openCodeDeepLink("opensymphony://code/opensymphony/files/packages/graph/src/index.ts")).toBe(true);
      await flushUntil(() => root.querySelector("[data-code-mode='file']")?.classList.contains("is-selected") ?? false);
      await flushUntil(() => root.querySelector("[data-testid='code-graph-file-fallback']")?.textContent?.includes("No symbol detail is required") ?? false);
    } finally {
      await handle.destroy();
    }
  });

  it("refreshes Code Graph snapshots without losing selection or drag overrides", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const transport = new LiveEventTransport({
      baseUri: "http://127.0.0.1:2468",
      health: capabilities,
      snapshot: dashboard,
      taskGraph,
      runDetails: [runDetail],
    });
    const fixtureAdapter = createFixtureCodeGraphAdapter();
    let snapshotReads = 0;
    let repoReads = 0;
    const codeGraphAdapter = {
      ...fixtureAdapter,
      async listRepos(options?: Parameters<typeof fixtureAdapter.listRepos>[0]) {
        repoReads += 1;
        return fixtureAdapter.listRepos(options);
      },
      async getGraphSnapshot(repoId: string, options?: Parameters<typeof fixtureAdapter.getGraphSnapshot>[1]) {
        const snapshot = await fixtureAdapter.getGraphSnapshot(repoId, options);
        snapshotReads += 1;
        return snapshotReads === 1
          ? snapshot
          : { ...snapshot, cursor: { ...snapshot.cursor, sequence: snapshot.cursor.sequence + snapshotReads } };
      },
    };
    const handle = renderOpenSymphonyApp({ root, mode: "desktop", transport, codeGraphAdapter });

    try {
      await flushUntil(() => root.querySelector("[data-node-id='desktop-alpha']") !== null);
      expect(await handle.openCodeDeepLink("opensymphony://code/opensymphony/symbols/codeGraphReducer")).toBe(true);
      await flushUntil(() => root.querySelector("[data-testid='code-graph-structure-list']") !== null);
      const app = handle as unknown as { codeGraphView: { overrides: Map<string, { x: number; y: number }> }; state: { codeGraph: { selectedNodeIds: string[]; stale: boolean; layoutStatus: string } } };
      app.codeGraphView.overrides.set("symbol:codeGraphReducer", { x: 42, y: 24 });
      await flushUntil(() => app.state.codeGraph.layoutStatus === "ready");
      const selectedBefore = [...app.state.codeGraph.selectedNodeIds];
      const repoReadsBeforeUpdate = repoReads;
      transport.emit({
        schema_version: schemaVersionV1(),
        cursor: { sequence: 90, partition: "events" },
        entity_ref: { kind: "unknown", id: "code-graph:opensymphony" },
        event_kind: "code_graph_updated",
        emitted_at: "2026-07-11T19:10:00Z",
        payload: {
          schema_version: schemaVersionV1(),
          repo_id: "opensymphony",
          cursor: { sequence: 10, partition: "code-graph:opensymphony" },
          updated_at: "2026-07-11T19:10:00Z",
        },
      });
      await flushUntil(() => snapshotReads >= 2 && repoReads > repoReadsBeforeUpdate && !app.state.codeGraph.stale && app.state.codeGraph.layoutStatus === "ready");
      expect(app.state.codeGraph.selectedNodeIds).toEqual(selectedBefore);
      expect(app.codeGraphView.overrides.get("symbol:codeGraphReducer")).toEqual({ x: 42, y: 24 });
    } finally {
      await handle.destroy();
    }
  });

  it("recomputes Code Graph layout after the stage resizes", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport: buildTransport(),
      codeGraphAdapter: createFixtureCodeGraphAdapter(),
    });
    const app = handle as unknown as {
      graphLayoutAdapter: {
        layout: (snapshot: unknown, options: { width: number; height: number }) => Promise<unknown>;
        dispose(): void;
      };
      render(): void;
      state: { codeGraph: { layoutStatus: string } };
    };
    const originalLayoutAdapter = app.graphLayoutAdapter;
    const layoutSizes: Array<{ width: number; height: number }> = [];
    app.graphLayoutAdapter = {
      layout: (snapshot, options) => {
        layoutSizes.push({ width: options.width, height: options.height });
        return originalLayoutAdapter.layout(snapshot, options);
      },
      dispose: () => originalLayoutAdapter.dispose(),
    };

    try {
      expect(await handle.openCodeDeepLink("opensymphony://code/opensymphony/atlas")).toBe(true);
      await flushUntil(() => app.state.codeGraph.layoutStatus === "ready" && layoutSizes.length >= 1);
      const stage = root.querySelector<HTMLElement>("[data-kg-stage]");
      expect(stage).not.toBeNull();
      let stageWidth = 720;
      Object.defineProperty(stage!, "getBoundingClientRect", {
        configurable: true,
        value: () => ({ width: stageWidth, height: 420 }),
      });
      stageWidth = 1400;
      app.render();
      await flushUntil(() => app.state.codeGraph.layoutStatus === "ready" && layoutSizes.length >= 2);
      expect(layoutSizes[1].width).toBeGreaterThan(layoutSizes[0].width);
    } finally {
      await handle.destroy();
    }
  });

  it("queues Code Graph navigation while a snapshot request is in flight", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const fixtureCodeGraphAdapter = createFixtureCodeGraphAdapter();
    let reads = 0;
    let releaseFirstRead: (() => void) | null = null;
    const firstRead = new Promise<void>((resolve) => {
      releaseFirstRead = resolve;
    });
    const codeGraphAdapter = {
      ...fixtureCodeGraphAdapter,
      async getGraphSnapshot(repoId: string, options?: Parameters<typeof fixtureCodeGraphAdapter.getGraphSnapshot>[1]) {
        reads += 1;
        const snapshot = await fixtureCodeGraphAdapter.getGraphSnapshot(repoId, options);
        if (reads === 1) await firstRead;
        return snapshot;
      },
    };
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport: buildTransport(),
      codeGraphAdapter,
    });

    try {
      const atlasNavigation = handle.openCodeDeepLink("opensymphony://code/opensymphony/atlas");
      await flushUntil(() => reads === 1);
      const fileNavigation = handle.openCodeDeepLink("opensymphony://code/opensymphony/files/packages/graph/src/index.ts");
      releaseFirstRead!();
      await expect(fileNavigation).resolves.toBe(true);
      await atlasNavigation;
      const app = handle as unknown as { state: { codeGraph: { mode: string; snapshot: { mode: string } | null } } };
      expect(reads).toBe(2);
      expect(app.state.codeGraph.mode).toBe("file");
      expect(app.state.codeGraph.snapshot?.mode).toBe("file");
    } finally {
      await handle.destroy();
    }
  });

  it("does not let a superseded Code Graph failure poison the current load", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const fixtureCodeGraphAdapter = createFixtureCodeGraphAdapter();
    let reads = 0;
    let rejectFirstRead: ((error: Error) => void) | null = null;
    const firstRead = new Promise<CodeGraphSnapshot>((_, reject) => {
      rejectFirstRead = reject;
    });
    let resolveSecondRead: ((snapshot: CodeGraphSnapshot) => void) | null = null;
    const secondRead = new Promise<CodeGraphSnapshot>((resolve) => {
      resolveSecondRead = resolve;
    });
    const codeGraphAdapter = {
      ...fixtureCodeGraphAdapter,
      async getGraphSnapshot(repoId: string, options?: Parameters<typeof fixtureCodeGraphAdapter.getGraphSnapshot>[1]) {
        reads += 1;
        if (reads === 1) return firstRead;
        if (reads === 2) return secondRead;
        return fixtureCodeGraphAdapter.getGraphSnapshot(repoId, options);
      },
    };
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport: buildTransport(),
      codeGraphAdapter,
    });

    try {
      const firstNavigation = handle.openCodeDeepLink("opensymphony://code/opensymphony/atlas");
      await flushUntil(() => reads === 1);
      const app = handle as unknown as {
        resetCodeGraphView(): void;
        state: { codeGraph: { layoutStatus: string; snapshot: CodeGraphSnapshot | null } };
      };
      app.resetCodeGraphView();
      rejectFirstRead!(new Error("stale Code Graph failure"));
      await flushUntil(() => reads === 2);
      expect(app.state.codeGraph.layoutStatus).not.toBe("failed");
      resolveSecondRead!(await fixtureCodeGraphAdapter.getGraphSnapshot("opensymphony", { mode: "atlas" }));
      await firstNavigation;
      await flushUntil(() => app.state.codeGraph.snapshot?.mode === "atlas");
    } finally {
      await handle.destroy();
    }
  });

  it("refreshes the Knowledge Graph on memory_graph_updated events", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const transport = new LiveEventTransport({
      baseUri: "http://127.0.0.1:2468",
      health: capabilities,
      snapshot: dashboard,
      taskGraph,
      runDetails: [runDetail],
    });
    let resolveRefresh: (() => void) | null = null;
    const refreshGate = new Promise<void>((resolve) => {
      resolveRefresh = resolve;
    });
    let reads = 0;
    const graphAdapter: GraphDataAdapter = {
      ...createFixtureGraphAdapter(),
      async getGraphSnapshot() {
        reads += 1;
        if (reads > 1) await refreshGate;
        return reads > 1
          ? {
              ...fixtureGraphSnapshot,
              cursor: { ...fixtureGraphSnapshot.cursor, sequence: 2 },
              metrics: { orphan_count: 0, broken_link_count: 0, stale_concept_count: 1, warning_count: 2 },
            }
          : fixtureGraphSnapshot;
      },
    };
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport,
      graphAdapter,
    });

    await flushUntil(() => root.querySelector("[data-graph-view='knowledge']") !== null);
    (root.querySelector("[data-graph-view='knowledge']") as HTMLButtonElement).click();
    await flushUntil(() => root.querySelector("[data-testid='knowledge-graph-node-list']")?.textContent?.includes("COE-465") ?? false);

    transport.emit({
      schema_version: schemaVersionV1(),
      cursor: { sequence: 20, partition: "events" },
      entity_ref: { kind: "unknown", id: "memory-graph:local-default" },
      event_kind: "memory_graph_updated",
      emitted_at: "2026-06-28T00:01:00Z",
      payload: {
        schema_version: schemaVersionV1(),
        bundle_id: "local-default",
        cursor: { sequence: 2, partition: "memory-graph:local-default" },
        updated_at: "2026-06-28T00:01:00Z",
      },
    });

    await flushUntil(() => root.querySelector("[data-testid='knowledge-graph-status']")?.textContent?.includes("Graph stale") ?? false);
    resolveRefresh?.();
    await flushUntil(() => root.querySelector("[data-testid='knowledge-graph-status']")?.textContent?.includes("Graph warnings") ?? false);
    expect(root.querySelector("[data-testid='knowledge-graph-metrics']")?.textContent).toContain("2");
    expect(reads).toBe(2);

    await handle.destroy();
  });

  it("cancels scheduled Knowledge Graph draws when disposed", () => {
    const root = document.createElement("div");
    const layout = computeGraphLayout(fixtureGraphSnapshot, { mode: "atlas" });
    root.innerHTML = renderKnowledgeGraphSurface({
      snapshot: fixtureGraphSnapshot,
      layout,
      state: { ...initialGraphState, layoutStatus: "ready" },
    });
    document.body.appendChild(root);

    const originalRequestAnimationFrame = globalThis.requestAnimationFrame;
    const originalCancelAnimationFrame = globalThis.cancelAnimationFrame;
    const requestAnimationFrameMock = jest.fn((_callback: FrameRequestCallback) => 123);
    const cancelAnimationFrameMock = jest.fn();
    globalThis.requestAnimationFrame = requestAnimationFrameMock;
    globalThis.cancelAnimationFrame = cancelAnimationFrameMock;

    const getContext = jest.spyOn(HTMLCanvasElement.prototype, "getContext").mockImplementation((
      contextId: string,
    ) => {
      if (contextId !== "2d") return null;
      return {
        setTransform: jest.fn(),
        fillRect: jest.fn(),
        beginPath: jest.fn(),
        closePath: jest.fn(),
        moveTo: jest.fn(),
        lineTo: jest.fn(),
        stroke: jest.fn(),
        arc: jest.fn(),
        fill: jest.fn(),
        set fillStyle(_value: string) {},
        set strokeStyle(_value: string) {},
        set lineWidth(_value: number) {},
        set globalAlpha(_value: number) {},
      } as unknown as CanvasRenderingContext2D;
    });
    try {
      mountKnowledgeGraphRenderer(root, {
        snapshot: fixtureGraphSnapshot,
        layout,
        selectedNodeIds: [],
        view: createKnowledgeGraphViewState(),
        onSelect: jest.fn(),
        onFocus: jest.fn(),
      });
      const canvas = root.querySelector<HTMLCanvasElement>("[data-testid='knowledge-graph-canvas']");
      canvas?.dispatchEvent(new WheelEvent("wheel", { deltaY: -1, bubbles: true, cancelable: true }));

      expect(requestAnimationFrameMock).toHaveBeenCalledTimes(1);
      disposeKnowledgeGraphRenderer(root);
      expect(cancelAnimationFrameMock).toHaveBeenCalledWith(123);
    } finally {
      getContext.mockRestore();
      if (originalRequestAnimationFrame) {
        globalThis.requestAnimationFrame = originalRequestAnimationFrame;
      } else {
        delete (globalThis as { requestAnimationFrame?: typeof requestAnimationFrame }).requestAnimationFrame;
      }
      if (originalCancelAnimationFrame) {
        globalThis.cancelAnimationFrame = originalCancelAnimationFrame;
      } else {
        delete (globalThis as { cancelAnimationFrame?: typeof cancelAnimationFrame }).cancelAnimationFrame;
      }
      root.remove();
    }
  });

  it("drills into an area when a stationary click lands on its cloud", () => {
    const root = document.createElement("div");
    const layout = computeGraphLayout(graphVizFixtureSnapshot, { kind: "force", width: 1280, height: 900 });
    root.innerHTML = renderKnowledgeGraphSurface({
      snapshot: graphVizFixtureSnapshot,
      layout,
      state: { ...initialGraphState, layoutStatus: "ready" },
    });
    document.body.appendChild(root);
    const onSelectArea = jest.fn();
    try {
      mountKnowledgeGraphRenderer(root, {
        snapshot: graphVizFixtureSnapshot,
        layout,
        selectedNodeIds: [],
        view: createKnowledgeGraphViewState(),
        onSelect: jest.fn(),
        onFocus: jest.fn(),
        onSelectArea,
      });
      const canvas = root.querySelector<HTMLCanvasElement>("[data-testid='knowledge-graph-canvas']")!;
      const scene = (canvas as HTMLCanvasElement & { __kgDebug?: { scene: GraphScene } }).__kgDebug!.scene;
      expect(scene.hulls.length).toBeGreaterThan(0);

      // Find a point that lies on an area cloud but not on any node, so the
      // click resolves as a drill instead of a node selection.
      let target: { x: number; y: number; areaId: string } | null = null;
      for (let y = 4; y < 360 && !target; y += 8) {
        for (let x = 4; x < 640 && !target; x += 8) {
          const hull = hitTestHull(scene, x, y);
          if (hull && hull.labelAlpha > 0.05 && hitTestScene(scene, x, y) === null) {
            target = { x, y, areaId: hull.areaId };
          }
        }
      }
      expect(target).not.toBeNull();

      // jsdom does not route dispatched pointer events through on* handler
      // properties, so invoke the renderer's handlers directly.
      const pointer = (type: string, x: number, y: number) =>
        new MouseEvent(type, { clientX: x, clientY: y, button: 0, bubbles: true }) as PointerEvent;
      canvas.onpointerdown!(pointer("pointerdown", target!.x, target!.y));
      canvas.onpointerup!(pointer("pointerup", target!.x, target!.y));
      expect(onSelectArea).toHaveBeenCalledWith(target!.areaId);

      // The same gesture with movement stays a pan and never drills.
      onSelectArea.mockClear();
      canvas.onpointerdown!(pointer("pointerdown", target!.x, target!.y));
      canvas.onpointermove!(pointer("pointermove", target!.x + 24, target!.y + 18));
      canvas.onpointerup!(pointer("pointerup", target!.x + 24, target!.y + 18));
      expect(onSelectArea).not.toHaveBeenCalled();

      // Option-drag orbits grab the scene: dragging right swings the scene
      // right (yaw decreases, inverted from the raw delta) while dragging
      // down tilts it down (pitch follows the raw delta) — tuned by feel,
      // see the orbit handler comment.
      const view = (canvas as HTMLCanvasElement & { __kgDebug?: { camera: { yaw: number; pitch: number } } }).__kgDebug!;
      const before = { ...view.camera };
      const orbitPointer = (type: string, x: number, y: number) =>
        new MouseEvent(type, { clientX: x, clientY: y, button: 0, altKey: true, bubbles: true }) as PointerEvent;
      canvas.onpointerdown!(orbitPointer("pointerdown", target!.x, target!.y));
      canvas.onpointermove!(orbitPointer("pointermove", target!.x + 40, target!.y + 30));
      canvas.onpointerup!(orbitPointer("pointerup", target!.x + 40, target!.y + 30));
      const after = (canvas as HTMLCanvasElement & { __kgDebug?: { camera: { yaw: number; pitch: number } } }).__kgDebug!.camera;
      expect(after.yaw).toBeLessThan(before.yaw);
      expect(after.pitch).toBeGreaterThan(before.pitch);
    } finally {
      disposeKnowledgeGraphRenderer(root);
      root.remove();
    }
  });

  it("resolves relative markdown capsule link targets against snapshot nodes", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    // OKF capsules can store link targets verbatim as relative markdown
    // paths ("../concepts/x.md"); the graph snapshot only knows the
    // resolved concept id ("concepts/x").
    const graphAdapter = createFixtureGraphAdapter({
      bundles: graphVizFixtureBundleList,
      snapshot: graphVizFixtureSnapshot,
      communities: graphVizFixtureCommunityList,
      conceptDetail: (_bundleId, conceptId) => {
        const detail = graphVizFixtureConceptDetail(conceptId);
        if (!detail) return null;
        return {
          ...detail,
          links: detail.links.map((link) => ({ ...link, target: `../${link.target}.md` })),
        };
      },
    });
    const handle = renderOpenSymphonyApp({ root, mode: "desktop", transport: buildTransport(), graphAdapter });

    try {
      await flushUntil(() => root.querySelector("[data-node-id='desktop-alpha']") !== null);
      expect(await handle.openMemoryDeepLink(
        "opensymphony://memory/viz-workbench/concepts/concepts/code-intelligence-01",
      )).toBe(true);
      await flushUntil(() => root.querySelector("[data-testid='knowledge-graph-capsule'] [data-kg-link-target^='../']") !== null);

      const link = root.querySelector<HTMLElement>("[data-testid='knowledge-graph-capsule'] [data-kg-link-target^='../']")!;
      const target = link.dataset.kgLinkTarget!;
      const resolvedConceptId = target.replace(/^(\.\.\/)+/, "").replace(/\.md$/, "");
      const expectedLabel = graphVizFixtureSnapshot.nodes.find((node) => node.concept_id === resolvedConceptId)!.label;

      link.click();
      await flushUntil(() =>
        root.querySelector("[data-testid='knowledge-graph-inspector'] h3")?.textContent === expectedLabel,
      );
    } finally {
      await handle.destroy();
    }
  });

  it("renders URL citation targets as external links, not graph buttons", () => {
    const concept = fixtureGraphSnapshot.nodes.find((node) => node.kind === "concept")!;
    const html = renderKnowledgeGraphInspector({
      snapshot: fixtureGraphSnapshot,
      layout: null,
      state: { ...initialGraphState, selectedNodeIds: [concept.id] },
      conceptDetail: {
        ...fixtureConceptDetail,
        citations: [
          { id: "1", target: "https://linear.app/x/issue/COE-200", label: "COE-200" },
          { id: "2", target: "issues/COE-465", label: "in-graph citation" },
        ],
      },
    });
    const root = document.createElement("div");
    root.innerHTML = html;
    const citations = root.querySelector("[data-testid='knowledge-graph-capsule-citations']")!;
    const anchor = citations.querySelector("a")!;
    expect(anchor.getAttribute("href")).toBe("https://linear.app/x/issue/COE-200");
    expect(anchor.textContent).toBe("COE-200");
    const buttons = Array.from(citations.querySelectorAll<HTMLElement>("[data-kg-link-target]"));
    expect(buttons.map((button) => button.dataset.kgLinkTarget)).toEqual(["issues/COE-465"]);
  });

  it("renders capsule citations as navigable graph links", () => {
    const concept = graphVizFixtureSnapshot.nodes.find(
      (node) => node.kind === "concept" && (graphVizFixtureConceptDetail(node.concept_id!)?.citations.length ?? 0) > 0,
    );
    expect(concept).toBeDefined();
    const detail = graphVizFixtureConceptDetail(concept!.concept_id!)!;
    const html = renderKnowledgeGraphInspector({
      snapshot: graphVizFixtureSnapshot,
      layout: null,
      state: { ...initialGraphState, selectedNodeIds: [concept!.id] },
      conceptDetail: detail,
    });
    const root = document.createElement("div");
    root.innerHTML = html;
    const citationButtons = Array.from(
      root.querySelectorAll<HTMLElement>("[data-testid='knowledge-graph-capsule-citations'] [data-kg-link-target]"),
    );
    expect(citationButtons.length).toBe(detail.citations.length);
    expect(citationButtons.map((button) => button.dataset.kgLinkTarget)).toEqual(
      detail.citations.map((citation) => citation.target),
    );
  });

  it("flags only truncated entity-list names for the instant hover tooltip", () => {
    const root = document.createElement("div");
    root.innerHTML = renderKnowledgeGraphNodeList(fixtureGraphSnapshot, []);
    document.body.appendChild(root);
    try {
      const buttons = Array.from(root.querySelectorAll<HTMLElement>(".os-kg-list [data-kg-node-id]"));
      expect(buttons.length).toBeGreaterThan(1);
      const [truncated, fitting] = buttons;
      // jsdom has no layout; emulate one ellipsized and one fitting row.
      Object.defineProperty(truncated, "scrollWidth", { value: 300, configurable: true });
      Object.defineProperty(truncated, "clientWidth", { value: 180, configurable: true });
      Object.defineProperty(fitting, "scrollWidth", { value: 120, configurable: true });
      Object.defineProperty(fitting, "clientWidth", { value: 180, configurable: true });

      bindKnowledgeGraphListNavigation(root, { onSelect: jest.fn(), onFocus: jest.fn() });
      expect(truncated.dataset.kgOverflow).toBe(truncated.textContent);
      expect(truncated.getAttribute("title")).toBe(truncated.textContent);
      expect(fitting.dataset.kgOverflow).toBeUndefined();
      expect(fitting.getAttribute("title")).toBeNull();

      // A relayout that makes the name fit clears the tooltip again.
      Object.defineProperty(truncated, "scrollWidth", { value: 100, configurable: true });
      bindKnowledgeGraphListNavigation(root, { onSelect: jest.fn(), onFocus: jest.fn() });
      expect(truncated.dataset.kgOverflow).toBeUndefined();
      expect(truncated.getAttribute("title")).toBeNull();
    } finally {
      root.remove();
    }
  });

  it("keeps scale rendering accessible with LOD labels and reduced motion", () => {
    const snapshot = createScaleGraphSnapshot(5_000);
    const layout = computeGraphLayout(snapshot, { kind: "force", width: 1280, height: 720 });
    const selectedNodeId = "concept:scale-1";
    const root = document.createElement("div");
    // The list and inspector render in the lower workspace columns; compose
    // them alongside the surface the way the app shell does.
    const composeView = (selectedIds: string[]) => {
      const state = { ...initialGraphState, selectedNodeIds: selectedIds, layoutStatus: "ready" as const };
      return renderKnowledgeGraphSurface({ snapshot, layout, state })
        + renderKnowledgeGraphInspector({ snapshot, layout, state })
        + renderKnowledgeGraphNodeList(snapshot, selectedIds);
    };
    root.innerHTML = composeView([selectedNodeId]);
    document.body.appendChild(root);
    // Labels are created imperatively by the renderer with zoom-based LOD;
    // the server-rendered surface starts with an empty overlay layer, and
    // selected nodes always earn a label regardless of zoom.
    expect(root.querySelectorAll(".os-kg-label").length).toBe(0);
    mountKnowledgeGraphRenderer(root, {
      snapshot,
      layout,
      selectedNodeIds: [selectedNodeId],
      view: createKnowledgeGraphViewState(),
      onSelect: jest.fn(),
      onFocus: jest.fn(),
    });
    expect(root.querySelectorAll(".os-kg-label").length).toBeLessThanOrEqual(80);
    expect(
      root.querySelector(`.os-kg-label[data-kg-node-id='${selectedNodeId}']`)?.classList.contains("is-selected"),
    ).toBe(true);
    disposeKnowledgeGraphRenderer(root);
    expect(root.querySelector("[data-testid='knowledge-graph-inspector'] dl")?.textContent).toContain("concept");
    expect(root.querySelector("[data-testid='knowledge-graph-inspector'] dl div")).toBeNull();
    expect(root.querySelector(".os-kg-list [data-kg-node-id='concept:scale-1']")?.getAttribute("aria-current")).toBe("true");
    root.innerHTML = composeView([]);
    expect(root.querySelector("[data-testid='knowledge-graph-inspector']")?.textContent).toContain("No node selected");

    const originalMatchMedia = globalThis.matchMedia;
    const originalRequestAnimationFrame = globalThis.requestAnimationFrame;
    const requestAnimationFrameMock = jest.fn();
    globalThis.matchMedia = jest.fn().mockReturnValue({ matches: true }) as unknown as typeof matchMedia;
    globalThis.requestAnimationFrame = requestAnimationFrameMock as unknown as typeof requestAnimationFrame;
    const getContext = jest.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(null);
    try {
      root.innerHTML = renderKnowledgeGraphSurface({
        snapshot,
        layout,
        state: {
          ...initialGraphState,
          selectedNodeIds: [selectedNodeId],
          layoutStatus: "ready",
        },
      });
      mountKnowledgeGraphRenderer(root, {
        snapshot,
        layout,
        selectedNodeIds: [selectedNodeId],
        view: createKnowledgeGraphViewState(),
        onSelect: jest.fn(),
        onFocus: jest.fn(),
      });
      const canvas = root.querySelector<HTMLCanvasElement>("[data-testid='knowledge-graph-canvas']");
      expect(canvas?.dataset.reducedMotion).toBe("true");
      canvas?.dispatchEvent(new WheelEvent("wheel", { deltaY: -1, bubbles: true, cancelable: true }));
      expect(requestAnimationFrameMock).not.toHaveBeenCalled();
      disposeKnowledgeGraphRenderer(root);
    } finally {
      getContext.mockRestore();
      globalThis.matchMedia = originalMatchMedia;
      if (originalRequestAnimationFrame) {
        globalThis.requestAnimationFrame = originalRequestAnimationFrame;
      } else {
        delete (globalThis as { requestAnimationFrame?: typeof requestAnimationFrame }).requestAnimationFrame;
      }
      root.remove();
    }
  });

  it("does not switch selected graph bundles for background memory_graph_updated events", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const transport = new LiveEventTransport({
      baseUri: "http://127.0.0.1:2468",
      health: capabilities,
      snapshot: dashboard,
      taskGraph,
      runDetails: [runDetail],
    });
    const selectedSnapshot = {
      ...fixtureGraphSnapshot,
      bundle_id: "selected-bundle",
      cursor: { sequence: 1, partition: "memory-graph:selected-bundle" },
      nodes: fixtureGraphSnapshot.nodes.map((node) => ({
        ...node,
        bundle_id: "selected-bundle",
        label: node.id === "concept:coe-465" ? "Selected Bundle Concept" : node.label,
      })),
    };
    const backgroundSnapshot = {
      ...fixtureGraphSnapshot,
      bundle_id: "background-bundle",
      cursor: { sequence: 2, partition: "memory-graph:background-bundle" },
      nodes: fixtureGraphSnapshot.nodes.map((node) => ({
        ...node,
        bundle_id: "background-bundle",
        label: node.id === "concept:coe-465" ? "Background Bundle Concept" : node.label,
      })),
    };
    const reads: string[] = [];
    const graphAdapter: GraphDataAdapter = {
      ...createFixtureGraphAdapter(),
      async listBundles() {
        return {
          schema_version: schemaVersionV1(),
          bundles: [
            {
              id: "selected-bundle",
              title: "Selected Bundle",
              okf_version: "0.1",
              visibility: "private",
              concept_count: 1,
              updated_at: "2026-06-28T00:00:00Z",
            },
            {
              id: "background-bundle",
              title: "Background Bundle",
              okf_version: "0.1",
              visibility: "private",
              concept_count: 1,
              updated_at: "2026-06-28T00:00:00Z",
            },
          ],
        };
      },
      async getGraphSnapshot(bundleId) {
        reads.push(bundleId);
        return bundleId === "background-bundle" ? backgroundSnapshot : selectedSnapshot;
      },
    };
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport,
      graphAdapter,
    });

    await flushUntil(() => root.querySelector("[data-graph-view='knowledge']") !== null);
    (root.querySelector("[data-graph-view='knowledge']") as HTMLButtonElement).click();
    await flushUntil(() => root.querySelector("[data-testid='knowledge-graph-node-list']")?.textContent?.includes("Selected Bundle Concept") ?? false);

    transport.emit({
      schema_version: schemaVersionV1(),
      cursor: { sequence: 21, partition: "events" },
      entity_ref: { kind: "unknown", id: "memory-graph:background-bundle" },
      event_kind: "memory_graph_updated",
      emitted_at: "2026-06-28T00:02:00Z",
      payload: {
        schema_version: schemaVersionV1(),
        bundle_id: "background-bundle",
        cursor: { sequence: 2, partition: "memory-graph:background-bundle" },
        updated_at: "2026-06-28T00:02:00Z",
      },
    });

    await flushUntil(() => reads.length >= 2);
    expect(reads).toEqual(["selected-bundle", "selected-bundle"]);
    expect(root.querySelector("[data-testid='knowledge-graph-node-list']")?.textContent).toContain("Selected Bundle Concept");
    expect(root.querySelector("[data-testid='knowledge-graph-node-list']")?.textContent).not.toContain("Background Bundle Concept");
    expect(root.querySelector("[data-testid='knowledge-graph-status']")?.textContent).not.toContain("Graph stale");

    await handle.destroy();
  });

  it("preserves selected graph nodes during same-bundle memory_graph_updated refreshes", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const transport = new LiveEventTransport({
      baseUri: "http://127.0.0.1:2468",
      health: capabilities,
      snapshot: dashboard,
      taskGraph,
      runDetails: [runDetail],
    });
    let reads = 0;
    const graphAdapter: GraphDataAdapter = {
      ...createFixtureGraphAdapter(),
      async getGraphSnapshot() {
        reads += 1;
        return {
          ...fixtureGraphSnapshot,
          cursor: { ...fixtureGraphSnapshot.cursor, sequence: reads },
        };
      },
    };
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport,
      graphAdapter,
    });

    await flushUntil(() => root.querySelector("[data-graph-view='knowledge']") !== null);
    (root.querySelector("[data-graph-view='knowledge']") as HTMLButtonElement).click();
    await flushUntil(() => root.querySelector("[data-testid='knowledge-graph-node-list']")?.textContent?.includes("COE-465") ?? false);

    const nodeButton = root.querySelector("[data-kg-node-id='concept:coe-465']") as HTMLButtonElement;
    nodeButton.click();
    await flushUntil(() => root.querySelector(".os-kg-list li.is-selected [data-kg-node-id='concept:coe-465']") !== null);

    transport.emit({
      schema_version: schemaVersionV1(),
      cursor: { sequence: 22, partition: "events" },
      entity_ref: { kind: "unknown", id: `memory-graph:${fixtureGraphSnapshot.bundle_id}` },
      event_kind: "memory_graph_updated",
      emitted_at: "2026-06-28T00:03:00Z",
      payload: {
        schema_version: schemaVersionV1(),
        bundle_id: fixtureGraphSnapshot.bundle_id,
        cursor: { sequence: 2, partition: fixtureGraphSnapshot.cursor.partition },
        updated_at: "2026-06-28T00:03:00Z",
      },
    });

    await flushUntil(() => reads >= 2);
    expect(root.querySelector(".os-kg-list li.is-selected [data-kg-node-id='concept:coe-465']")).not.toBeNull();

    await handle.destroy();
  });

  it("serializes overlapping Knowledge Graph loads from pane switches and events", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const transport = new LiveEventTransport({
      baseUri: "http://127.0.0.1:2468",
      health: capabilities,
      snapshot: dashboard,
      taskGraph,
      runDetails: [runDetail],
    });
    let resolveFirstRead: (() => void) | null = null;
    const firstReadGate = new Promise<void>((resolve) => {
      resolveFirstRead = resolve;
    });
    let reads = 0;
    const graphAdapter: GraphDataAdapter = {
      ...createFixtureGraphAdapter(),
      async getGraphSnapshot() {
        reads += 1;
        if (reads === 1) {
          await firstReadGate;
          return fixtureGraphSnapshot;
        }
        return {
          ...fixtureGraphSnapshot,
          cursor: { ...fixtureGraphSnapshot.cursor, sequence: 2 },
          metrics: { orphan_count: 0, broken_link_count: 0, stale_concept_count: 1, warning_count: 2 },
        };
      },
    };
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport,
      graphAdapter,
    });

    await flushUntil(() => root.querySelector("[data-graph-view='knowledge']") !== null);
    (root.querySelector("[data-graph-view='knowledge']") as HTMLButtonElement).click();
    await flushUntil(() => reads === 1);

    transport.emit({
      schema_version: schemaVersionV1(),
      cursor: { sequence: 22, partition: "events" },
      entity_ref: { kind: "unknown", id: "memory-graph:local-default" },
      event_kind: "memory_graph_updated",
      emitted_at: "2026-06-28T00:03:00Z",
      payload: {
        schema_version: schemaVersionV1(),
        bundle_id: "local-default",
        cursor: { sequence: 2, partition: "memory-graph:local-default" },
        updated_at: "2026-06-28T00:03:00Z",
      },
    });

    await flushUntil(() => root.querySelector("[data-testid='knowledge-graph-status']")?.textContent?.includes("Graph stale") ?? false);
    expect(reads).toBe(1);
    resolveFirstRead?.();
    await flushUntil(() => root.querySelector("[data-testid='knowledge-graph-status']")?.textContent?.includes("Graph warnings") ?? false);
    expect(reads).toBe(2);

    await handle.destroy();
  });

  it("groups desktop task graph rows by explicit project metadata", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport: buildTransport({ taskGraph: projectSetTaskGraph }),
    });

    await flushUntil(
      () => root.querySelector("[data-project-group='alpha-project']") !== null,
    );

    const headings = Array.from(root.querySelectorAll(".os-project-group-header"));
    expect(headings.map((heading) => heading.textContent)).toEqual([
      expect.stringContaining("alpha-project | Alpha Project"),
      expect.stringContaining("beta-project | Beta Project"),
    ]);
    // Done nodes (completed-prereq) render in the Completed pane, not the
    // Current pane's project groups.
    expect(headings[0]?.textContent).toContain("issues=3 running=1 todo=2");
    expect(headings[0]?.textContent).toContain("blocked=1");
    expect(headings[1]?.textContent).toContain("issues=2 running=0 todo=2");
    expect(headings[1]?.textContent).toContain("blocked=2");
    expect(root.querySelector("[data-project-group='alpha-project'] [data-node-id='desktop-alpha']")).not.toBeNull();
    expect(root.querySelector("[data-project-group='beta-project'] [data-node-id='hosted-auth']")).not.toBeNull();

    await handle.destroy();
  });

  it("sorts desktop project groups and keeps mixed-metadata rows grouped", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const alpha = {
      ...projectSetTaskGraph.nodes.find((node) => node.node_id === "desktop-alpha")!,
      project_name: undefined,
    };
    // Not Done here: a terminal state would drop the node from the Current
    // pane and this test is about mixed project metadata, not state buckets.
    const alphaNamed = {
      ...projectSetTaskGraph.nodes.find((node) => node.node_id === "completed-prereq")!,
      state: "Todo",
      state_category: "todo" as const,
    };
    const beta = projectSetTaskGraph.nodes.find((node) => node.node_id === "hosted-auth")!;
    const unassigned = {
      ...taskGraph.nodes.find((node) => node.node_id === "follow-up")!,
      project_id: undefined,
      project_slug: undefined,
      project_name: undefined,
    };
    const nameOnly = {
      ...taskGraph.nodes.find((node) => node.node_id === "app-shell")!,
      project_id: undefined,
      project_slug: undefined,
      project_name: "Name Only Project",
    };
    const mixedTaskGraph: TaskGraphSnapshot = {
      ...taskGraph,
      nodes: [beta, unassigned, alpha, nameOnly, alphaNamed],
      root_ids: [],
    };
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport: buildTransport({ taskGraph: mixedTaskGraph }),
    });

    await flushUntil(
      () => root.querySelectorAll(".os-project-group-header").length === 4,
    );

    const headings = Array.from(root.querySelectorAll(".os-project-group-header"))
      .map((heading) => heading.textContent ?? "");
    expect(headings[0]).toContain("alpha-project | Alpha Project");
    expect(headings[1]).toContain("beta-project | Beta Project");
    expect(headings[2]).toContain("Name Only Project");
    expect(headings[3]).toContain("unassigned");
    expect(headings[3]).not.toContain("unassigned | Unassigned");
    expect(root.querySelector("[data-project-group='Name Only Project'] [data-node-id='app-shell']")).not.toBeNull();
    expect(root.querySelector("[data-project-group='__opensymphony_unassigned__'] [data-node-id='follow-up']")).not.toBeNull();

    await handle.destroy();
  });

  it("does not group web task graph rows when project metadata is present", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const handle = renderOpenSymphonyApp({
      root,
      mode: "web",
      transport: buildTransport({ taskGraph: projectSetTaskGraph }),
    });

    await flushUntil(
      () => root.querySelector("[data-node-id='desktop-alpha']") !== null,
    );

    expect(root.querySelector(".os-project-group-header")).toBeNull();

    await handle.destroy();
  });

  it("renders a project heading for explicit single-project snapshots", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const singleProjectTaskGraph: TaskGraphSnapshot = {
      ...taskGraph,
      nodes: taskGraph.nodes.map((node) => ({
        ...node,
        project_id: "proj-alpha",
        project_slug: "alpha-project",
        project_name: "Alpha Project",
      })),
    };
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport: buildTransport({ taskGraph: singleProjectTaskGraph }),
    });

    await flushUntil(
      () => root.querySelector(".os-project-group-header") !== null,
    );

    expect(root.querySelectorAll(".os-project-group-header")).toHaveLength(1);
    expect(root.querySelector(".os-project-group-header")?.textContent).toContain("alpha-project | Alpha Project");

    await handle.destroy();
  });

  it("collapses and expands desktop project groups without clearing selected run detail", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport: buildTransport({ taskGraph: projectSetTaskGraph }),
    });

    await flushUntil(() => root.querySelector(".os-run-head strong")?.textContent === "COE-449");
    const alphaHeader = root.querySelector("[data-project-group-toggle='alpha-project']") as HTMLButtonElement;
    alphaHeader.click();
    await flushUntil(
      () => root.querySelector("[data-project-group='alpha-project'] [data-node-id='desktop-alpha']") === null,
    );

    expect(root.querySelector("[data-project-group='beta-project'] [data-node-id='hosted-auth']")).not.toBeNull();
    expect(root.querySelector(".os-run-head strong")?.textContent).toBe("COE-449");
    expect(root.querySelector("[data-project-group-toggle='alpha-project']")?.getAttribute("aria-expanded")).toBe("false");
    expect(root.querySelector("[data-project-group-toggle='alpha-project']")?.getAttribute("aria-controls")).toBe("os-project-group-alpha-project");
    expect(root.querySelector("#os-project-group-alpha-project")?.getAttribute("role")).toBe("region");

    (root.querySelector("[data-project-group-toggle='alpha-project']") as HTMLButtonElement).click();
    await flushUntil(
      () => root.querySelector("[data-project-group='alpha-project'] [data-node-id='desktop-alpha']") !== null,
    );

    const restoredAlphaNodes = Array.from(
      root.querySelectorAll("[data-project-group='alpha-project'] [data-node-id]"),
    ).map((node) => node.getAttribute("data-node-id"));
    expect(restoredAlphaNodes).toEqual(["m7-milestone", "app-shell", "desktop-alpha", "follow-up"]);
    expect(root.querySelector(".os-run-head strong")?.textContent).toBe("COE-449");

    await handle.destroy();
  });

  it("uses the shared project grouping fixture for desktop groups and dependency signals", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport: buildTransport({
        taskGraph: sharedProjectGroupingTaskGraph,
        runDetails: [
          { ...runDetail, run_id: "COE-703", issue_id: "COE-703", issue_identifier: "COE-703" },
        ],
      }),
    });

    await flushUntil(
      () => root.querySelector("[data-project-group='alpha-project']") !== null,
    );

    const headings = Array.from(root.querySelectorAll(".os-project-group-header"))
      .map((heading) => heading.textContent ?? "");
    expect(headings).toEqual([
      expect.stringContaining("alpha-project | Alpha Project"),
      expect.stringContaining("beta-project | Beta Project"),
      expect.stringContaining("unassigned"),
    ]);
    expect(headings[0]).toContain("issues=2 running=1 todo=1 blocked=1");
    expect(headings[1]).toContain("issues=2 running=0 todo=2 blocked=1");
    expect(headings[2]).toContain("issues=1 running=0 todo=1 blocked=0");
    expect(root.querySelector("[data-project-group='__opensymphony_unassigned__'] [data-node-id='COE-705']")).not.toBeNull();
    // Dependencies now surface as connector glyphs + arrows instead of a text
    // line: ">" downstream (blocks), "<" upstream (blocked, visible or hidden).
    expect(root.querySelector("[data-testid='task-graph-link'][data-link-from='COE-700'][data-link-to='COE-701']")).not.toBeNull();
    expect(root.querySelector("[data-node-id='COE-700'] .os-node-gutter")?.textContent).toContain(">");
    expect(root.querySelector("[data-node-id='COE-701'] .os-node-gutter")?.textContent).toContain("<");
    expect(root.querySelector("[data-node-id='COE-702'] .os-node-gutter")?.textContent).toContain("<");
    // COE-704 is terminal, so it is not an active blocker of COE-703 (no "<").
    expect(root.querySelector("[data-node-id='COE-703'] .os-node-gutter")?.textContent ?? "").not.toContain("<");

    (root.querySelector("[data-project-group-toggle='beta-project']") as HTMLButtonElement).click();
    await flushUntil(
      () => root.querySelector("[data-project-group='beta-project'] [data-node-id='COE-702']") === null,
    );
    expect(root.querySelector("[data-project-group-toggle='beta-project']")?.getAttribute("aria-expanded")).toBe("false");

    (root.querySelector("[data-project-group-toggle='beta-project']") as HTMLButtonElement).click();
    await flushUntil(
      () => root.querySelector("[data-project-group='beta-project'] [data-node-id='COE-703']") !== null,
    );
    (root.querySelector("[data-node-id='COE-703']") as HTMLElement).click();
    await flushUntil(
      () => root.querySelector("[data-testid='dependency-detail']")?.textContent?.includes("completed blockers COE-704") ?? false,
    );

    await handle.destroy();
  });

  it("refreshes selected run evidence from live gateway events without remounting", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const transport = new LiveEventTransport({
      baseUri: "http://127.0.0.1:2468",
      health: capabilities,
      snapshot: dashboard,
      taskGraph,
      runDetails: [runDetail],
      runFiles: [
        { runId: "COE-449", files: changedFiles },
      ],
      runDiffs: [
        { runId: "COE-449", filePath: "src/config.ts", diff: fileDiff },
      ],
      runEvents: [
        runEvents,
      ],
    });
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport,
    });
    const warnSpy = jest.spyOn(console, "warn").mockImplementation(() => undefined);

    try {
      await flushUntil(() => root.textContent?.includes("src/config.ts") ?? false);

      transport.failNextSnapshot("transient snapshot failure");
      transport.emit({
        schema_version: schemaVersionV1(),
        cursor: { sequence: 1, partition: "events" },
        entity_ref: { kind: "run", id: "COE-449" },
        event_kind: "run.completed",
        emitted_at: "2025-09-01T00:00:30Z",
        payload: { run_id: "COE-449" },
      });
      await flushUntil(() =>
        warnSpy.mock.calls.some((call) =>
          call[0] === "[opensymphony] live gateway refresh failed; event stream remains active"
          && (call[1] as { error?: string }).error === "transient snapshot failure"
        ),
      );
      expect(root.textContent).not.toContain("Live data stale");

      transport.failNextSnapshot("persistent snapshot failure");
      transport.emit({
        schema_version: schemaVersionV1(),
        cursor: { sequence: 2, partition: "events" },
        entity_ref: { kind: "run", id: "COE-449" },
        event_kind: "run.completed",
        emitted_at: "2025-09-01T00:00:45Z",
        payload: { run_id: "COE-449" },
      });
      await flushUntil(() => root.textContent?.includes("Live data stale") ?? false);
      expect(root.textContent).toContain("persistent snapshot failure");

      const nextFiles: ChangedFileEntry[] = [
        {
          path: "src/live-update.ts",
          change_kind: "modified",
          lines_added: 4,
          lines_removed: 1,
        },
      ];
      transport.setRunFiles("COE-449", nextFiles);
      transport.setRunDiff("COE-449", "src/live-update.ts", {
        ...fileDiff,
        run_id: "COE-449",
        file_path: "src/live-update.ts",
      });
      transport.updateRunDetail({
        ...runDetail,
        status: "released",
        release_reason: "completed",
      });
      transport.setTaskGraph({
        ...taskGraph,
        nodes: taskGraph.nodes.map((node) =>
          node.node_id === "desktop-alpha"
            ? { ...node, state: "Done", state_category: "done" }
            : node,
        ),
      });
      transport.emit({
        schema_version: schemaVersionV1(),
        cursor: { sequence: 3, partition: "events" },
        entity_ref: { kind: "run", id: "COE-449" },
        event_kind: "run.completed",
        emitted_at: "2025-09-01T00:01:00Z",
        payload: { run_id: "COE-449" },
      });

      await flushUntil(() => root.textContent?.includes("src/live-update.ts") ?? false);

      expect(root.textContent).not.toContain("src/config.ts");
      expect(root.textContent).not.toContain("Live data stale");
      expect(root.querySelector(".os-pill")?.textContent).toBe("released");
      // Once the task reaches Done it leaves the Current pane (three-pane
      // task graph): the card must no longer render among current tasks.
      expect(root.querySelector("[data-node-id='desktop-alpha']")).toBeNull();
      expect(root.querySelector("[data-testid='changed-file-item']")?.getAttribute("data-path")).toBe("src/live-update.ts");
    } finally {
      warnSpy.mockRestore();
      await handle.destroy();
    }
  });

  it("keeps the operator-selected task across gateway refreshes", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport: buildTransport({
        runDetails: [
          { ...runDetail, run_id: "COE-450", issue_id: "COE-450", issue_identifier: "COE-450" },
        ],
      }),
    });

    await flushUntil(() => root.querySelector(".os-run-head strong")?.textContent === "COE-449");

    (root.querySelector("[data-node-id='app-shell']") as HTMLElement).click();
    await flushUntil(() => root.querySelector(".os-run-head span")?.textContent === "COE-450");

    await handle.refresh();
    await flushUntil(() => root.querySelector(".os-run-head span")?.textContent === "COE-450");

    expect(root.querySelector("[data-node-id='app-shell']")?.classList.contains("is-selected")).toBe(true);

    await handle.destroy();
  });

  it("resumes restarted live subscriptions after the latest processed cursor", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const transport = new LiveEventTransport({
      baseUri: "http://127.0.0.1:2468",
      health: capabilities,
      snapshot: dashboard,
      taskGraph,
      runDetails: [runDetail],
      runFiles: [{ runId: "COE-449", files: changedFiles }],
      runDiffs: [{ runId: "COE-449", filePath: "src/config.ts", diff: fileDiff }],
      runEvents: [runEvents],
    });
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport,
    });

    try {
      await flushUntil(() => root.textContent?.includes("src/config.ts") ?? false);
      await flushUntil(() => transport.subscriptions.length === 1);

      transport.emit({
        schema_version: schemaVersionV1(),
        cursor: { sequence: 8, partition: "events" },
        entity_ref: { kind: "run", id: "COE-449" },
        event_kind: "run.updated",
        emitted_at: "2025-09-01T00:02:00Z",
        payload: { run_id: "COE-449" },
      });
      await flushUntil(() => transport.snapshotReads >= 2);

      transport.endStream();
      await flushUntil(() => transport.activeStreams === 0);
      await handle.refresh();
      await flushUntil(() => transport.subscriptions.length === 2);

      expect(transport.subscriptions[1]).toEqual({ sequence: 8, partition: "events" });

      const readsAfterRestart = transport.snapshotReads;
      transport.emit({
        schema_version: schemaVersionV1(),
        cursor: { sequence: 8, partition: "events" },
        entity_ref: { kind: "run", id: "COE-449" },
        event_kind: "run.updated",
        emitted_at: "2025-09-01T00:02:05Z",
        payload: { run_id: "COE-449" },
      });
      await flushAsync();
      expect(transport.snapshotReads).toBe(readsAfterRestart);

      transport.emit({
        schema_version: schemaVersionV1(),
        cursor: { sequence: 9, partition: "events" },
        entity_ref: { kind: "run", id: "COE-449" },
        event_kind: "run.updated",
        emitted_at: "2025-09-01T00:02:10Z",
        payload: { run_id: "COE-449" },
      });
      await flushUntil(() => transport.snapshotReads > readsAfterRestart);
    } finally {
      await handle.destroy();
    }
  });

  it("resumes after the cursor from failed live refresh events", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const transport = new LiveEventTransport({
      baseUri: "http://127.0.0.1:2468",
      health: capabilities,
      snapshot: dashboard,
      taskGraph,
      runDetails: [runDetail],
      runFiles: [{ runId: "COE-449", files: changedFiles }],
      runDiffs: [{ runId: "COE-449", filePath: "src/config.ts", diff: fileDiff }],
      runEvents: [runEvents],
    });
    const warnSpy = jest.spyOn(console, "warn").mockImplementation(() => undefined);
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport,
    });

    try {
      await flushUntil(() => root.textContent?.includes("src/config.ts") ?? false);
      await flushUntil(() => transport.subscriptions.length === 1);

      transport.failNextSnapshot("first failed live refresh");
      transport.emit({
        schema_version: schemaVersionV1(),
        cursor: { sequence: 10, partition: "events" },
        entity_ref: { kind: "run", id: "COE-449" },
        event_kind: "run.updated",
        emitted_at: "2025-09-01T00:03:00Z",
        payload: { run_id: "COE-449" },
      });
      await flushUntil(() =>
        warnSpy.mock.calls.some((call) =>
          call[0] === "[opensymphony] live gateway refresh failed; event stream remains active"
          && (call[1] as { error?: string }).error === "first failed live refresh"
        ),
      );
      expect(transport.activeStreams).toBe(1);

      transport.failNextSnapshot("second failed live refresh");
      transport.emit({
        schema_version: schemaVersionV1(),
        cursor: { sequence: 11, partition: "events" },
        entity_ref: { kind: "run", id: "COE-449" },
        event_kind: "run.updated",
        emitted_at: "2025-09-01T00:03:05Z",
        payload: { run_id: "COE-449" },
      });
      await flushUntil(() => root.textContent?.includes("Live data stale: second failed live refresh") ?? false);

      transport.endStream();
      await flushUntil(() => transport.activeStreams === 0);
      await handle.refresh();
      await flushUntil(() => transport.subscriptions.length === 2);

      expect(transport.subscriptions[1]).toEqual({ sequence: 11, partition: "events" });

      const readsAfterRestart = transport.snapshotReads;
      transport.emit({
        schema_version: schemaVersionV1(),
        cursor: { sequence: 11, partition: "events" },
        entity_ref: { kind: "run", id: "COE-449" },
        event_kind: "run.updated",
        emitted_at: "2025-09-01T00:03:10Z",
        payload: { run_id: "COE-449" },
      });
      await flushAsync();
      expect(transport.snapshotReads).toBe(readsAfterRestart);

      transport.emit({
        schema_version: schemaVersionV1(),
        cursor: { sequence: 12, partition: "events" },
        entity_ref: { kind: "run", id: "COE-449" },
        event_kind: "run.updated",
        emitted_at: "2025-09-01T00:03:15Z",
        payload: { run_id: "COE-449" },
      });
      await flushUntil(() => transport.snapshotReads > readsAfterRestart);
    } finally {
      warnSpy.mockRestore();
      await handle.destroy();
    }
  });

  it("ignores unknown live gateway events without refreshing the shell", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const transport = new LiveEventTransport({
      baseUri: "http://127.0.0.1:2468",
      health: capabilities,
      snapshot: dashboard,
      taskGraph,
      runDetails: [runDetail],
      runFiles: [{ runId: "COE-449", files: changedFiles }],
      runDiffs: [{ runId: "COE-449", filePath: "src/config.ts", diff: fileDiff }],
      runEvents: [runEvents],
    });
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport,
    });

    try {
      await flushUntil(() => root.textContent?.includes("src/config.ts") ?? false);
      const readsAfterLoad = transport.snapshotReads;

      transport.emit({
        schema_version: schemaVersionV1(),
        cursor: { sequence: 3, partition: "events" },
        entity_ref: { kind: "unknown", id: "heartbeat" },
        event_kind: "gateway.heartbeat",
        emitted_at: "2025-09-01T00:02:00Z",
      });
      await flushAsync();

      expect(transport.snapshotReads).toBe(readsAfterLoad);
    } finally {
      await handle.destroy();
    }
  });

  it("keeps the shell loaded when the event stream is unavailable", async () => {
    class UnavailableEventTransport extends MockGatewayTransport {
      override async *events(): AsyncIterable<GatewayEnvelope> {
        throw new Error("stream unavailable");
      }
    }
    const root = document.createElement("div");
    document.body.appendChild(root);
    const warnSpy = jest.spyOn(console, "warn").mockImplementation(() => undefined);
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport: new UnavailableEventTransport({
        baseUri: "http://127.0.0.1:2468",
        health: capabilities,
        snapshot: dashboard,
        taskGraph,
        runDetails: [runDetail],
        runFiles: [{ runId: "COE-449", files: changedFiles }],
        runDiffs: [{ runId: "COE-449", filePath: "src/config.ts", diff: fileDiff }],
        runEvents: [runEvents],
      }),
    });

    try {
      await flushUntil(() => root.textContent?.includes("src/config.ts") ?? false);

      expect(root.querySelector(".os-status-connected")).not.toBeNull();
      expect(root.querySelector("[data-testid='changed-file-item']")?.getAttribute("data-path")).toBe("src/config.ts");
      expect(warnSpy).toHaveBeenCalledWith(
        "[opensymphony] gateway event stream unavailable; using periodic refresh fallback",
        expect.objectContaining({
          baseUri: "http://127.0.0.1:2468",
          error: "stream unavailable",
        }),
      );
    } finally {
      warnSpy.mockRestore();
      await handle.destroy();
    }
  });

  it("refreshes selected run evidence from the periodic fallback when live events are silent", async () => {
    jest.useFakeTimers();
    class SilentEventTransport extends MockGatewayTransport {
      override async *events(): AsyncIterable<GatewayEnvelope> {
        await new Promise<never>(() => undefined);
      }
    }
    const root = document.createElement("div");
    document.body.appendChild(root);
    const transport = new SilentEventTransport({
      baseUri: "http://127.0.0.1:2468",
      health: capabilities,
      snapshot: dashboard,
      taskGraph,
      runDetails: [runDetail],
      runFiles: [{ runId: "COE-449", files: changedFiles }],
      runDiffs: [{ runId: "COE-449", filePath: "src/config.ts", diff: fileDiff }],
      runEvents: [runEvents],
    });
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport,
    });

    try {
      await flushMicrotasks();
      expect(root.textContent).toContain("src/config.ts");

      transport.setRunFiles("COE-449", [
        {
          path: "src/poll-refresh.ts",
          change_kind: "modified",
          lines_added: 2,
          lines_removed: 0,
        },
      ]);
      transport.setRunDiff("COE-449", "src/poll-refresh.ts", {
        ...fileDiff,
        run_id: "COE-449",
        file_path: "src/poll-refresh.ts",
      });

      jest.advanceTimersByTime(5_000);
      await flushMicrotasks();

      expect(root.textContent).toContain("src/poll-refresh.ts");
      expect(root.textContent).not.toContain("src/config.ts");
    } finally {
      await handle.destroy();
      jest.useRealTimers();
    }
  });

  it("edits an API-compatible model profile and shows a redacted credential reference", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const profiles = defaultModelProfiles();
    const modelProfileController = buildModelProfileController(profiles);
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport: buildTransport(),
      modelProfileController,
      initialModelProfiles: profiles,
    });

    await flushUntil(() => root.querySelector("[data-toggle-settings='model']") !== null);

    expect(root.querySelector("[data-testid='model-profile-panel']")).toBeNull();
    const collapsedToggle = root.querySelector("[data-toggle-settings='model']") as HTMLButtonElement;
    expect(collapsedToggle).not.toBeNull();
    expect(collapsedToggle.classList.contains("os-model-gear")).toBe(true);
    expect(collapsedToggle.getAttribute("aria-expanded")).toBe("false");
    expect(collapsedToggle.textContent).not.toContain("Collapse");
    expect(collapsedToggle.textContent).not.toContain("Edit");
    expect(root.querySelector("[data-model-credential-ref]")).toBeNull();
    await expandSettingsPanel(root, "model", "[data-model-credential-ref]");
    const expandedToggle = root.querySelector("[data-settings-modal='model'] [data-toggle-settings='model']") as HTMLButtonElement;
    expect(root.querySelector(".os-model-panel h2")?.textContent).toBe("Model Configuration");
    expect(root.querySelector("[data-testid='model-redacted-credential']")?.textContent).toContain("API key not configured");
    expect(expandedToggle.textContent?.trim()).toBe("x");
    expect(expandedToggle.getAttribute("aria-expanded")).toBe("true");
    expect(expandedToggle.textContent).not.toContain("Collapse");
    expect(expandedToggle.textContent).not.toContain("Edit");
    expect((root.querySelector("[data-model-credential-ref]") as HTMLInputElement).type).toBe("password");
    expect(root.textContent).not.toContain("Cost Profile");
    expect(root.textContent).not.toContain("Context Window");
    expect(root.textContent).not.toContain("Recommended For");
    expect(root.textContent).not.toContain("Reasoning");
    expect(root.textContent).not.toContain("Subscription Provider");
    expect(root.textContent).not.toContain("Credential Storage");

    (root.querySelector("[data-model-name]") as HTMLInputElement).value = "provider/custom-model-name";
    (root.querySelector("[data-model-base-url]") as HTMLInputElement).value = "https://models.example.test/v1";
    (root.querySelector("[data-model-credential-ref]") as HTMLInputElement).value = "local_keychain:custom-api-key";
    (root.querySelector("[data-model-harnesses]") as HTMLInputElement).value = "openhands_agent_server, custom_harness";
    (root.querySelector("[data-save-model-profile]") as HTMLButtonElement).click();

    await flushUntil(() =>
      modelProfileController.saved.some((profile) => profile.model === "provider/custom-model-name"),
    );

    const saved = modelProfileController.saved.find((profile) => profile.model === "provider/custom-model-name");
    expect(saved?.mode).toBe("api_key");
    expect(saved?.baseUrl).toBe("https://models.example.test/v1");
    expect(saved?.apiKeyRef).toBe("local_keychain:custom-api-key");
    expect(saved?.harnesses).toContain("custom_harness");
    await flushUntil(() =>
      root.querySelector("[data-testid='model-redacted-credential']")?.textContent?.includes("API key configured") ?? false,
    );

    await handle.destroy();
  });

  it("rejects raw secrets and mismatched credential reference prefixes", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const modelProfileController = buildModelProfileController();
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport: buildTransport(),
      modelProfileController,
    });

    await expandSettingsPanel(root, "model", "[data-model-credential-ref]");

    (root.querySelector("[data-model-name]") as HTMLInputElement).value = "provider/custom-model-name";
    (root.querySelector("[data-model-credential-ref]") as HTMLInputElement).value = "sk-secret-value-123456789";
    (root.querySelector("[data-save-model-profile]") as HTMLButtonElement).click();
    await flushUntil(() => root.textContent?.includes("API key secret must use local_keychain:") ?? false);

    (root.querySelector("[data-model-credential-ref]") as HTMLInputElement).value = "openhands_auth:openai";
    (root.querySelector("[data-save-model-profile]") as HTMLButtonElement).click();
    await flushUntil(() => root.textContent?.includes("API key secret must use local_keychain:") ?? false);

    expect(modelProfileController.saved.some((profile) => profile.model === "provider/custom-model-name")).toBe(false);

    await handle.destroy();
  });

  it("edits a subscription-backed model profile", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const profiles = defaultModelProfiles();
    profiles[1] = {
      ...profiles[1],
      subscriptionCredential: {
        ...profiles[1].subscriptionCredential!,
        authMethod: "browser",
        openBrowser: true,
        forceLogin: true,
        accountIdentityHeader: "X-OpenSymphony-Account",
      },
    };
    const modelProfileController = buildModelProfileController(profiles);
    const handle = renderOpenSymphonyApp({
      root,
      mode: "web",
      transport: buildTransport(),
      modelProfileController,
      initialModelProfiles: profiles,
    });

    await expandSettingsPanel(root, "model", "[data-model-profile-select]");

    (root.querySelector("[data-model-profile-select]") as HTMLSelectElement).value = "openai-subscription";
    (root.querySelector("[data-model-profile-select]") as HTMLSelectElement).dispatchEvent(
      new Event("change", { bubbles: true }),
    );
    await flushUntil(() => (root.querySelector("[data-model-mode]") as HTMLSelectElement).value === "subscription");
    expect((root.querySelector("[data-model-credential-ref]") as HTMLInputElement).type).toBe("text");
    expect(root.textContent).toContain("OpenHands Auth Directory Env (OpenHands only)");
    expect(root.querySelector("[data-testid='model-redacted-credential']")?.textContent).toContain("Codex CLI login via gateway readiness");
    expect(root.querySelector("[data-testid='model-redacted-credential']")?.textContent).toContain("OpenHands auth dir env OPENHANDS_AUTH_DIR");

    (root.querySelector("[data-model-name]") as HTMLInputElement).value = "codex-subscription-preview";
    (root.querySelector("[data-model-credential-ref]") as HTMLInputElement).value = "OPENHANDS_AUTH_DIR";
    (root.querySelector("[data-model-harnesses]") as HTMLInputElement).value = "openhands_agent_server, codex_app_server";
    (root.querySelector("[data-save-model-profile]") as HTMLButtonElement).click();

    await flushUntil(() =>
      modelProfileController.saved.some((profile) => profile.model === "codex-subscription-preview"),
    );

    const saved = modelProfileController.saved.find((profile) => profile.model === "codex-subscription-preview");
    expect(saved?.mode).toBe("subscription");
    expect(saved?.apiKeyRef).toBeNull();
    expect(saved?.subscriptionCredential?.authDirectoryEnv).toBe("OPENHANDS_AUTH_DIR");
    expect(saved?.subscriptionCredential?.provider).toBe("openai");
    expect(saved?.subscriptionCredential?.authMethod).toBe("browser");
    expect(saved?.subscriptionCredential?.openBrowser).toBe(true);
    expect(saved?.subscriptionCredential?.forceLogin).toBe(true);
    expect(saved?.subscriptionCredential?.accountIdentityHeader).toBe("X-OpenSymphony-Account");
    expect(saved?.credentialStorage).toBe("openhands_auth_directory");
    expect(saved?.harnesses).toEqual(["openhands_agent_server", "codex_app_server"]);

    await handle.destroy();
  });

  it("preserves API-key credential storage when editing a profile", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const profiles = defaultModelProfiles();
    profiles[0] = {
      ...profiles[0],
      credentialStorage: "openhands_auth_directory",
      apiKeyRef: "openhands_auth:openai-api-key",
    };
    const modelProfileController = buildModelProfileController(profiles);
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport: buildTransport(),
      modelProfileController,
      initialModelProfiles: profiles,
    });

    await expandSettingsPanel(root, "model", "[data-model-credential-ref]");
    (root.querySelector("[data-model-name]") as HTMLInputElement).value = "provider/custom-model-name";
    (root.querySelector("[data-model-credential-ref]") as HTMLInputElement).value = "openhands_auth:edited-api-key";
    (root.querySelector("[data-save-model-profile]") as HTMLButtonElement).click();

    await flushUntil(() =>
      modelProfileController.saved.some((profile) => profile.model === "provider/custom-model-name"),
    );

    const saved = modelProfileController.saved.find((profile) => profile.model === "provider/custom-model-name");
    expect(saved?.mode).toBe("api_key");
    expect(saved?.credentialStorage).toBe("openhands_auth_directory");
    expect(saved?.apiKeyRef).toBe("openhands_auth:edited-api-key");

    await handle.destroy();
  });

  it("normalizes an empty API-key secret reference to null", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const profiles = defaultModelProfiles();
    profiles[0] = {
      ...profiles[0],
      apiKeyRef: "local_keychain:openai-api-key",
    };
    const modelProfileController = buildModelProfileController(profiles);
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport: buildTransport(),
      modelProfileController,
      initialModelProfiles: profiles,
    });

    await expandSettingsPanel(root, "model", "[data-model-credential-ref]");
    const profileId = (root.querySelector("[data-model-profile-select]") as HTMLSelectElement).value;
    (root.querySelector("[data-model-credential-ref]") as HTMLInputElement).value = "";
    (root.querySelector("[data-save-model-profile]") as HTMLButtonElement).click();

    await flushUntil(() =>
      modelProfileController.saved.some((profile) => profile.id === profileId && profile.apiKeyRef === null),
    );
    const saved = modelProfileController.saved.find((profile) => profile.id === profileId);
    expect(saved?.apiKeyRef).toBeNull();

    await handle.destroy();
  });

  it("preserves model profile order when editing", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const modelProfileController = buildModelProfileController();
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport: buildTransport(),
      modelProfileController,
    });

    await expandSettingsPanel(root, "model", "[data-model-profile-select]");
    const before = Array.from(root.querySelectorAll<HTMLOptionElement>("[data-model-profile-select] option"))
      .map((option) => option.value);
    (root.querySelector("[data-model-name]") as HTMLInputElement).value = "provider/order-preserved";
    (root.querySelector("[data-save-model-profile]") as HTMLButtonElement).click();

    await flushUntil(() =>
      modelProfileController.saved.some((profile) => profile.model === "provider/order-preserved"),
    );
    const after = Array.from(root.querySelectorAll<HTMLOptionElement>("[data-model-profile-select] option"))
      .map((option) => option.value);
    expect(after).toEqual(before);

    await handle.destroy();
  });

  it("rerenders model credential fields when mode changes", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const modelProfileController = buildModelProfileController();
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport: buildTransport(),
      modelProfileController,
    });

    await expandSettingsPanel(root, "model", "[data-model-mode]");
    (root.querySelector("[data-model-credential-ref]") as HTMLInputElement).value = "local_keychain:openai-api-key";
    const modeSelect = root.querySelector("[data-model-mode]") as HTMLSelectElement;
    modeSelect.value = "subscription";
    modeSelect.dispatchEvent(new Event("change", { bubbles: true }));

    await flushUntil(() => (root.querySelector("[data-model-mode]") as HTMLSelectElement).value === "subscription");
    expect(root.textContent).toContain("OpenHands Auth Directory Env");
    const credentialInput = root.querySelector("[data-model-credential-ref]") as HTMLInputElement;
    expect(credentialInput.type).toBe("text");
    expect(credentialInput.value).toBe("");

    await handle.destroy();
  });

  it("creates and removes model profiles from the panel", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const confirmSpy = jest.spyOn(window, "confirm")
      .mockReturnValueOnce(false)
      .mockReturnValueOnce(true);
    const modelProfileController = buildModelProfileController();
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport: buildTransport(),
      modelProfileController,
    });

    await expandSettingsPanel(root, "model", "[data-new-model-profile]");
    const startingCount = root.querySelectorAll("[data-model-profile-select] option").length;
    (root.querySelector("[data-new-model-profile]") as HTMLButtonElement).click();

    await flushUntil(() =>
      root.querySelectorAll("[data-model-profile-select] option").length === startingCount + 1,
    );
    const createdId = (root.querySelector("[data-model-profile-select]") as HTMLSelectElement).value;
    expect(modelProfileController.saved.some((profile) => profile.id === createdId)).toBe(true);

    (root.querySelector("[data-remove-model-profile]") as HTMLButtonElement).click();
    await flushAsync();
    expect(modelProfileController.saved.some((profile) => profile.id === createdId)).toBe(true);

    (root.querySelector("[data-remove-model-profile]") as HTMLButtonElement).click();
    await flushUntil(() =>
      !modelProfileController.saved.some((profile) => profile.id === createdId),
    );
    expect(root.querySelectorAll("[data-model-profile-select] option")).toHaveLength(startingCount);
    expect(confirmSpy).toHaveBeenCalledTimes(2);

    confirmSpy.mockRestore();
    await handle.destroy();
  });

  it("deactivates the active model profile with the explicit Active checkbox", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const modelProfileController = buildModelProfileController();
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport: buildTransport(),
      modelProfileController,
    });

    await expandSettingsPanel(root, "model", "[data-model-active]");
    expect((root.querySelector("[data-model-active]") as HTMLInputElement).checked).toBe(true);

    (root.querySelector("[data-model-active]") as HTMLInputElement).checked = false;
    (root.querySelector("[data-save-model-profile]") as HTMLButtonElement).click();

    await flushUntil(() =>
      modelProfileController.saved.find((profile) => profile.id === "openai-api-compatible")?.active === false,
    );
    expect((root.querySelector("[data-model-active]") as HTMLInputElement).checked).toBe(false);

    await handle.destroy();
  });

  it("keeps model profile save failures separate from gateway connection health", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const modelProfileController = buildModelProfileController();
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport: buildTransport(),
      modelProfileController: {
        ...modelProfileController,
        async storeProfile() {
          throw new Error("secure settings unavailable");
        },
      },
    });

    await expandSettingsPanel(root, "model", "[data-model-credential-ref]");
    await flushUntil(() => root.querySelector(".os-status-connected") !== null);

    (root.querySelector("[data-model-name]") as HTMLInputElement).value = "provider/custom-model-name";
    (root.querySelector("[data-model-credential-ref]") as HTMLInputElement).value = "local_keychain:custom-api-key";
    (root.querySelector("[data-save-model-profile]") as HTMLButtonElement).click();

    await flushUntil(() =>
      root.querySelector("[data-testid='model-profile-error']")?.textContent?.includes("Model profile save failed: secure settings unavailable") ?? false,
    );
    expect(root.querySelector(".os-topbar p")?.textContent).not.toContain("Model profile save failed");
    expect(root.querySelector(".os-status-connected")).not.toBeNull();
    expect(root.querySelector(".os-status-failed")).toBeNull();

    await handle.destroy();
  });

  it("keeps model profile load failures separate from gateway connection health", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const modelProfileController = buildModelProfileController();
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport: buildTransport(),
      modelProfileController: {
        ...modelProfileController,
        async listProfiles() {
          throw new Error("settings store unavailable");
        },
      },
    });

    await flushUntil(() =>
      root.querySelector("[data-testid='model-profile-error']")?.textContent?.includes("Model profiles unavailable: settings store unavailable") ?? false,
    );
    await flushUntil(() => root.querySelector(".os-status-connected") !== null);
    expect(root.querySelector(".os-topbar p")?.textContent).not.toContain("Model profiles unavailable");
    expect(root.querySelector(".os-status-failed")).toBeNull();

    await handle.destroy();
  });

  it("reports session-only model profile persistence in the panel", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const modelProfileController = buildModelProfileController();
    modelProfileController.persistence = {
      kind: "session",
      label: "Model profiles are session-only because host storage is unavailable.",
    };
    const handle = renderOpenSymphonyApp({
      root,
      mode: "web",
      transport: buildTransport(),
      modelProfileController,
    });

    await flushUntil(() => root.querySelector("[data-testid='model-persistence-status']") !== null);

    expect(root.querySelector("[data-testid='model-persistence-status']")?.textContent).toContain("session-only");
    expect(root.querySelector(".os-model-persistence-session")).not.toBeNull();

    await handle.destroy();
  });

  it("surfaces model profile quarantine warnings in the panel", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const modelProfileController = buildModelProfileController();
    modelProfileController.quarantineMessages = [
      "Dropped invalid model profile raw-secret: API key secret must use local_keychain:<name>",
    ];
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport: buildTransport(),
      modelProfileController,
    });

    await flushUntil(() =>
      root.querySelector("[data-testid='model-profile-error']")?.textContent?.includes("Dropped invalid model profile raw-secret") ?? false,
    );

    expect(root.querySelector("[data-testid='model-profile-error']")?.textContent).toContain("Model profile storage warning");

    await handle.destroy();
  });

  it("uses the model profile controller warning drain when available", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const modelProfileController = buildModelProfileController();
    modelProfileController.quarantineMessages = ["stale warning"];
    modelProfileController.takeQuarantineMessages = jest.fn(() => [
      "Dropped model profile with missing id",
    ]);
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport: buildTransport(),
      modelProfileController,
    });

    await flushUntil(() =>
      root.querySelector("[data-testid='model-profile-error']")?.textContent?.includes("missing id") ?? false,
    );

    expect(modelProfileController.takeQuarantineMessages).toHaveBeenCalled();
    expect(modelProfileController.quarantineMessages).toEqual(["stale warning"]);

    await handle.destroy();
  });

  it("reports a failed connection instead of falling back to fixture data", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport: buildTransport({ failHealth: true }),
    });

    await flushUntil(
      () =>
        root.querySelector("[data-opensymphony-app-shell='mounted']") !== null,
    );

    await flushUntil(() => root.querySelector(".os-status-failed") !== null);

    expect(root.querySelector(".os-status-failed")).not.toBeNull();
    expect(root.textContent).toContain("Failed");
    expect(root.textContent).toContain("Gateway unavailable");
    expect(root.textContent).not.toContain("desktop-alpha");

    await handle.destroy();
  });

  it("renders structured native errors with their message", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport: buildTransport({ failTaskGraphStructured: true }),
    });

    await flushUntil(() => root.textContent?.includes("simulated structured task graph failure") ?? false);

    expect(root.textContent).toContain("Task graph unavailable: simulated structured task graph failure");
    expect(root.textContent).not.toContain("[object Object]");

    await handle.destroy();
  });

  it("disables profile save when no profile controller is provided", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport: buildTransport(),
    });

    await expandSettingsPanel(root, "connection", "[data-save-profile]");
    const save = root.querySelector("[data-save-profile]") as HTMLButtonElement;
    expect(save).not.toBeNull();
    expect(save.disabled).toBe(true);

    await handle.destroy();
  });

  it("routes a saved profile through ProfileController and refreshes the active gateway URL", async () => {
    class CloseCountingTransport extends MockGatewayTransport {
      closeCalls = 0;

      override async close(): Promise<void> {
        this.closeCalls += 1;
        await super.close();
      }
    }
    const root = document.createElement("div");
    document.body.appendChild(root);

    const newUrl = "http://127.0.0.1:9001";
    let lastConnect: string | null = null;
    const fixtureCodeGraphAdapter = createFixtureCodeGraphAdapter();
    let codeGraphReads = 0;
    const codeGraphAdapter = {
      ...fixtureCodeGraphAdapter,
      async getGraphSnapshot(repoId: string, options?: Parameters<typeof fixtureCodeGraphAdapter.getGraphSnapshot>[1]) {
        codeGraphReads += 1;
        return fixtureCodeGraphAdapter.getGraphSnapshot(repoId, options);
      },
    };
    const initialTransport = new CloseCountingTransport({
      baseUri: "http://127.0.0.1:2468",
      health: capabilities,
      snapshot: dashboard,
      taskGraph,
      runDetails: [
        runDetail,
        { ...runDetail, run_id: "desktop-alpha", issue_id: "desktop-alpha" },
      ],
      runFiles: [
        { runId: "COE-449", files: changedFiles },
        { runId: "desktop-alpha", files: changedFiles },
      ],
      runDiffs: [
        { runId: "COE-449", filePath: "src/config.ts", diff: fileDiff },
        { runId: "desktop-alpha", filePath: "src/config.ts", diff: { ...fileDiff, run_id: "desktop-alpha" } },
      ],
      runEvents: [
        runEvents,
        { ...runEvents, run_id: "desktop-alpha" },
      ],
    });
    const controller: ProfileController = {
      async listProfiles(): Promise<ConnectionProfile[]> {
        return [];
      },
      async storeProfile(profile: EditableProfileInput): Promise<ConnectionProfile> {
        return {
          id: "profile-saved",
          label: profile.label,
          kind: profile.kind,
          active: true,
          gatewayUrl: profile.gatewayUrl,
          transport: "loopback_http",
          managed: false,
        };
      },
      async setActiveProfile(profileId: string): Promise<ConnectionProfile> {
        return {
          id: profileId,
          label: "Saved profile",
          kind: "external_gateway",
          active: true,
          gatewayUrl: newUrl,
          transport: "loopback_http",
          managed: false,
        };
      },
      async removeProfile(): Promise<ConnectionProfile[]> {
        return [];
      },
    };

    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport: initialTransport,
      profileController: controller,
      codeGraphAdapter,
      onGatewayUrlChanged: async (url) => {
        lastConnect = url;
        return buildTransport();
      },
    });

    expect(await handle.openCodeDeepLink("opensymphony://code/opensymphony/atlas")).toBe(true);
    const readsBeforeGatewaySwitch = codeGraphReads;
    await flushUntil(() => root.querySelector("[data-active-graph-surface='code']") !== null);

    await expandSettingsPanel(root, "connection", "[data-save-profile]");

    const gatewayInput = root.querySelector(
      "[data-profile-gateway]",
    ) as HTMLInputElement;
    const save = root.querySelector("[data-save-profile]") as HTMLButtonElement;

    gatewayInput.value = newUrl;
    save.click();

    await flushUntil(() => lastConnect === newUrl && codeGraphReads > readsBeforeGatewaySwitch);
    expect(lastConnect).toBe(newUrl);
    expect(codeGraphReads).toBeGreaterThan(readsBeforeGatewaySwitch);
    expect(initialTransport.closeCalls).toBe(1);
    expect(save.disabled).toBe(false);

    await handle.destroy();
  });

  it("creates and deletes connection profiles from the panel", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const confirmSpy = jest.spyOn(window, "confirm")
      .mockReturnValueOnce(false)
      .mockReturnValueOnce(true);

    const profiles: ConnectionProfile[] = [
      {
        id: "local",
        label: "Local",
        kind: "local_daemon",
        active: true,
        gatewayUrl: "http://127.0.0.1:2468",
        transport: "loopback_http",
        managed: false,
      },
    ];
    const controller: ProfileController = {
      async listProfiles() {
        return profiles;
      },
      async storeProfile(profile) {
        const saved: ConnectionProfile = {
          id: profile.id ?? "created",
          label: profile.label,
          kind: profile.kind,
          active: true,
          gatewayUrl: profile.gatewayUrl,
          transport: "loopback_http",
          managed: false,
        };
        const index = profiles.findIndex((candidate) => candidate.id === saved.id);
        if (index >= 0) {
          profiles[index] = saved;
        } else {
          profiles.push(saved);
        }
        return saved;
      },
      async setActiveProfile(profileId) {
        const active = profiles.find((profile) => profile.id === profileId);
        if (!active) {
          throw new Error(`Unknown profile: ${profileId}`);
        }
        profiles.forEach((profile) => {
          profile.active = profile.id === profileId;
        });
        return active;
      },
      async removeProfile(profileId) {
        const index = profiles.findIndex((profile) => profile.id === profileId);
        if (index < 0) {
          throw new Error(`Unknown profile: ${profileId}`);
        }
        profiles.splice(index, 1);
        if (profiles[0]) {
          profiles[0].active = true;
        }
        return profiles;
      },
    };
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport: buildTransport(),
      profileController: controller,
    });

    await expandSettingsPanel(root, "connection", "[data-new-profile]");
    (root.querySelector("[data-new-profile]") as HTMLButtonElement).click();
    await flushUntil(() =>
      Array.from(root.querySelectorAll("[data-profile-select] option")).some((option) =>
        option.getAttribute("value") === "created"
      ),
    );
    expect(profiles.some((profile) => profile.id === "created")).toBe(true);

    (root.querySelector("[data-remove-profile]") as HTMLButtonElement).click();
    await flushAsync();
    expect(profiles.some((profile) => profile.id === "created")).toBe(true);

    (root.querySelector("[data-remove-profile]") as HTMLButtonElement).click();
    await flushUntil(() => !profiles.some((profile) => profile.id === "created"));
    expect(root.querySelectorAll("[data-profile-select] option")).toHaveLength(1);
    expect(confirmSpy).toHaveBeenCalledTimes(2);

    confirmSpy.mockRestore();
    await handle.destroy();
  });

  it("renders one connection settings modal in hosted auth placeholders", async () => {
    class AuthSnapshotTransport extends MockGatewayTransport {
      override async snapshot(): Promise<DashboardSnapshot> {
        throw { code: "unauthenticated", message: "sign in required" };
      }
    }
    const root = document.createElement("div");
    document.body.appendChild(root);
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport: new AuthSnapshotTransport({
        baseUri: "http://127.0.0.1:2468",
        health: { ...capabilities, auth_modes: ["oauth"] },
        snapshot: dashboard,
        taskGraph,
        runDetails: [runDetail],
      }),
    });

    await flushUntil(() => root.querySelector("[data-testid='auth-placeholder']") !== null);
    expect(root.querySelector("[data-testid='auth-placeholder']")?.getAttribute("data-auth-state")).toBe("unauthenticated");
    expect(root.querySelector(".os-profile-panel")).toBeNull();

    await expandSettingsPanel(root, "connection", "[data-profile-select]");

    expect(root.querySelectorAll(".os-profile-panel")).toHaveLength(1);
    expect(root.querySelectorAll("[data-profile-select]")).toHaveLength(1);

    await handle.destroy();
  });

  it("renders the profile panel and provided initial profile when no controller is set", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport: buildTransport(),
    });

    await flushUntil(() => root.querySelector("[data-toggle-settings='connection']") !== null);
    expect(root.querySelector(".os-profile-panel")).toBeNull();
    expect(root.querySelector("[data-profile-select]")).toBeNull();
    const collapsedToggle = root.querySelector("[data-toggle-settings='connection']") as HTMLButtonElement;
    expect(collapsedToggle).not.toBeNull();
    expect(collapsedToggle.getAttribute("aria-expanded")).toBe("false");
    expect(collapsedToggle.textContent).not.toContain("Collapse");
    expect(collapsedToggle.textContent).not.toContain("Edit");
    await expandSettingsPanel(root, "connection", "[data-profile-select]");
    const expandedToggle = root.querySelector("[data-settings-modal='connection'] [data-toggle-settings='connection']") as HTMLButtonElement;
    expect(expandedToggle.textContent?.trim()).toBe("x");
    expect(expandedToggle.getAttribute("aria-expanded")).toBe("true");
    expect(expandedToggle.textContent).not.toContain("Collapse");
    expect(expandedToggle.textContent).not.toContain("Edit");
    const select = root.querySelector(
      "[data-profile-select]",
    ) as HTMLSelectElement;
    expect(select).not.toBeNull();
    // Without a profile controller the shell uses the default UI profile.
    expect(select.options.length).toBeGreaterThan(0);
    await handle.destroy();
  });
});

describe("three-pane task graph", () => {
  const backlogNodes = [
    {
      schema_version: schemaVersionV1(),
      node_id: "backlog-a",
      kind: "issue" as const,
      identifier: "COE-460",
      title: "Backlog changelog & publish",
      state: "Backlog",
      state_category: "backlog" as const,
      parent_id: "m7-milestone",
      children: [],
      blocked_by: ["COE-449"],
      labels: ["backlog"],
    },
    {
      schema_version: schemaVersionV1(),
      node_id: "backlog-b",
      kind: "issue" as const,
      identifier: "COE-461",
      title: "Backlog release evidence",
      state: "Backlog",
      state_category: "backlog" as const,
      parent_id: "m7-milestone",
      children: [],
      blocked_by: ["COE-460"],
      labels: ["backlog"],
    },
    {
      schema_version: schemaVersionV1(),
      node_id: "backlog-c",
      kind: "issue" as const,
      identifier: "COE-462",
      title: "Backlog unrelated polish",
      state: "Backlog",
      state_category: "backlog" as const,
      parent_id: "m7-milestone",
      children: [],
      blocked_by: [],
      labels: ["backlog"],
    },
    {
      schema_version: schemaVersionV1(),
      node_id: "canceled-node",
      kind: "issue" as const,
      identifier: "COE-463",
      title: "Canceled experiment",
      state: "Canceled",
      state_category: "canceled" as const,
      parent_id: "m7-milestone",
      children: [],
      blocked_by: [],
      labels: [],
    },
  ];
  const threePaneTaskGraph: TaskGraphSnapshot = {
    ...taskGraph,
    nodes: [...taskGraph.nodes, ...backlogNodes],
  };
  const completedRows = [
    {
      issue_key: "COE-448",
      concept_id: "issues/COE-465",
      bundle_id: "local-default",
      title: "Completed prerequisite",
      state: "Done",
      milestone: "M7",
      url: "https://linear.app/example/issue/COE-448",
      completed_at: "2026-06-10T00:00:00Z",
      prs: [
        {
          number: 700,
          title: "COE-448 first attempt",
          url: "https://github.com/example/repo/pull/700",
          merged: false,
        },
        {
          number: 720,
          title: "COE-448 landed",
          url: "https://github.com/example/repo/pull/720",
          merged: true,
          merged_at: "2026-06-10T00:00:00Z",
        },
      ],
      source: "memory" as const,
    },
    ...Array.from({ length: 30 }, (_, index) => ({
      issue_key: `COE-${400 + index}`,
      concept_id: `issues/COE-${400 + index}`,
      bundle_id: "local-default",
      title: `Historical task ${400 + index}`,
      state: "Done",
      completed_at: new Date(Date.UTC(2026, 4, 1) - index * 86_400_000).toISOString(),
      prs: [
        {
          number: 500 + index,
          title: `COE-${400 + index} landed`,
          url: `https://github.com/example/repo/pull/${500 + index}`,
          merged: true,
          merged_at: new Date(Date.UTC(2026, 4, 1) - index * 86_400_000).toISOString(),
        },
      ],
      source: "memory" as const,
    })),
  ];

  async function mountThreePane() {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport: buildTransport({ taskGraph: threePaneTaskGraph }),
      graphAdapter: createFixtureGraphAdapter({ completedTasks: completedRows }),
    });
    await flushUntil(() =>
      root.querySelectorAll("[data-testid='completed-task-row']").length > 0
      && root.querySelector("[data-tg-pane='backlog'] [data-node-id='backlog-a']") !== null,
    );
    return { root, handle };
  }

  it("renders Completed, Current, and Backlog panes with cross-pane edges", async () => {
    const { root, handle } = await mountThreePane();

    expect(root.querySelector("[data-testid='task-pane-done']")).not.toBeNull();
    expect(root.querySelector("[data-testid='task-pane-current']")).not.toBeNull();
    expect(root.querySelector("[data-testid='task-pane-backlog']")).not.toBeNull();

    // Done nodes leave the Current pane; backlog nodes render in their own.
    expect(root.querySelector("[data-tg-pane='current'] [data-node-id='completed-prereq']")).toBeNull();
    expect(root.querySelector("[data-tg-pane='current'] [data-node-id='backlog-a']")).toBeNull();
    expect(root.querySelectorAll("[data-tg-pane='backlog'] [data-node-id]").length).toBe(3);
    // Canceled nodes have no other pane: they stay visible in Current so
    // the Canceled state filter can still surface them.
    expect(root.querySelector("[data-tg-pane='current'] [data-node-id='canceled-node']")).not.toBeNull();

    // The backlog task's upstream blocker lives in the Current pane (both
    // graph panes count as visible), so its connector glyph shows "<" and the
    // specific blocker is named by the cross-pane edge below.
    expect(root.querySelector("[data-node-id='backlog-a'] .os-node-gutter")?.textContent).toContain("<");

    // The cross-pane edge from the Current blocker into the Backlog exists
    // with the shared link data contract (geometry is measured in-browser).
    const cross = root.querySelector("[data-testid='task-graph-cross-link']");
    expect(cross?.getAttribute("data-link-from")).toBe("desktop-alpha");
    expect(cross?.getAttribute("data-link-to")).toBe("backlog-a");

    // First page: newest completion first, 25 rows per page.
    const rows = Array.from(root.querySelectorAll("[data-testid='completed-task-row']"));
    expect(rows).toHaveLength(25);
    expect(rows[0]?.getAttribute("data-task-key")).toBe("COE-448");

    // Multi-PR presentation: newest PR emphasized, unmerged struck through.
    const firstRowPrs = Array.from(rows[0]?.querySelectorAll(".os-tg-pr") ?? []);
    expect(firstRowPrs.map((pr) => pr.textContent)).toEqual(["#720", "#700"]);
    expect(firstRowPrs[0]?.classList.contains("os-tg-pr-latest")).toBe(true);
    expect(firstRowPrs[1]?.classList.contains("os-tg-pr-unmerged")).toBe(true);

    // Capsule deep link carries the wired opensymphony://memory URL.
    expect(rows[0]?.querySelector("[data-tg-capsule]")?.getAttribute("data-tg-capsule"))
      .toBe("opensymphony://memory/local-default/concepts/issues/COE-465");

    await handle.destroy();
  });

  it("searches, sorts, and paginates the Completed pane", async () => {
    const { root, handle } = await mountThreePane();

    (root.querySelector("[data-tg-done-page='next']") as HTMLButtonElement).click();
    await flushUntil(() =>
      root.querySelectorAll("[data-testid='completed-task-row']").length === completedRows.length - 25,
    );

    (root.querySelector("[data-tg-done-sort='id']") as HTMLButtonElement).click();
    await flushUntil(() =>
      root.querySelector("[data-testid='completed-task-row']")?.getAttribute("data-task-key") === "COE-400",
    );
    expect(
      root.querySelector("[data-tg-done-sort='id']")?.closest("th")?.getAttribute("aria-sort"),
    ).toBe("ascending");

    const search = root.querySelector("[data-tg-done-search]") as HTMLInputElement;
    search.value = "prerequisite";
    search.dispatchEvent(new Event("input", { bubbles: true }));
    // The search input debounces (~180ms) before hitting the adapter, so
    // this wait needs more head-room than the default flush window.
    await flushUntil(
      () => root.querySelectorAll("[data-testid='completed-task-row']").length === 1,
      600,
    );
    expect(
      root.querySelector("[data-testid='completed-task-row']")?.getAttribute("data-task-key"),
    ).toBe("COE-448");

    await handle.destroy();
  });

  it("boldens the ancestry critical path when a backlog task is selected", async () => {
    const { root, handle } = await mountThreePane();

    (root.querySelector("[data-tg-pane='backlog'] [data-node-id='backlog-b']") as HTMLButtonElement).click();
    await flushUntil(() =>
      root.querySelector("[data-node-id='backlog-b']")?.classList.contains("is-selected") ?? false,
    );

    const ancestry = Array.from(root.querySelectorAll(".is-ancestry"))
      .map((path) => `${path.getAttribute("data-link-from")}->${path.getAttribute("data-link-to")}`)
      .sort();
    expect(ancestry).toEqual(["backlog-a->backlog-b", "desktop-alpha->backlog-a"]);
    expect(root.querySelector("[data-node-id='backlog-c']")?.classList.contains("os-tg-dim")).toBe(true);
    expect(root.querySelector("[data-node-id='desktop-alpha']")?.classList.contains("os-tg-ancestry")).toBe(true);
    // Selecting a backlog task never opens a run: the Current selection's
    // run detail panel is untouched.
    expect(root.querySelector(".os-run-head strong")?.textContent).toBe("COE-449");

    // Hovering a Current task spotlights only its own edges, then restores
    // the pinned ancestry emphasis on leave.
    const currentCard = root.querySelector("[data-tg-pane='current'] [data-node-id='desktop-alpha']") as HTMLElement;
    currentCard.dispatchEvent(new Event("pointerenter"));
    const active = Array.from(root.querySelectorAll(".os-task-graph-link.is-active, .os-tg-cross-link.is-active"))
      .map((path) => `${path.getAttribute("data-link-from")}->${path.getAttribute("data-link-to")}`);
    expect(active).toContain("desktop-alpha->backlog-a");
    currentCard.dispatchEvent(new Event("pointerleave"));
    expect(root.querySelectorAll(".os-task-graph-link.is-active, .os-tg-cross-link.is-active").length).toBe(0);
    expect(root.querySelectorAll(".is-ancestry").length).toBe(2);

    await handle.destroy();
  });

  it("collapses and expands the Completed and Backlog panes", async () => {
    const { root, handle } = await mountThreePane();

    (root.querySelector("[data-tg-pane-toggle='done']") as HTMLButtonElement).click();
    await flushUntil(() => root.querySelector("[data-tg-pane='done'][data-collapsed]") !== null);
    expect(root.querySelector("[data-testid='completed-task-row']")).toBeNull();
    expect(root.querySelector("[data-tg-pane='done'] .os-tg-pane-vertical-label")?.textContent).toBe("Completed");

    (root.querySelector("[data-tg-pane-toggle='backlog']") as HTMLButtonElement).click();
    await flushUntil(() => root.querySelector("[data-tg-pane='backlog'][data-collapsed]") !== null);
    expect(root.querySelector("[data-tg-pane='backlog'] [data-node-id]")).toBeNull();
    // The Current pane has no collapse affordance.
    expect(root.querySelector("[data-tg-pane-toggle='current']")).toBeNull();

    (root.querySelector("[data-tg-pane-toggle='done']") as HTMLButtonElement).click();
    (root.querySelector("[data-tg-pane-toggle='backlog']") as HTMLButtonElement).click();
    await flushUntil(() =>
      root.querySelectorAll("[data-testid='completed-task-row']").length === 25
      && root.querySelector("[data-tg-pane='backlog'] [data-node-id='backlog-a']") !== null,
    );

    await handle.destroy();
  });

  it("resizes the Completed and Backlog side panes via their handles", async () => {
    const { root, handle } = await mountThreePane();

    const donePane = () => root.querySelector("[data-tg-pane='done']") as HTMLElement;
    const backlogPane = () => root.querySelector("[data-tg-pane='backlog']") as HTMLElement;
    const doneResizer = () => root.querySelector("[data-tg-resizer='done']") as HTMLElement;
    const backlogResizer = () => root.querySelector("[data-tg-resizer='backlog']") as HTMLElement;

    // A resizer sits between each expanded side pane and the Current pane.
    expect(doneResizer()).not.toBeNull();
    expect(backlogResizer()).not.toBeNull();
    expect(donePane().style.flexBasis).toBe("360px");
    expect(backlogPane().style.flexBasis).toBe("340px");

    // ArrowRight widens Completed (the pane left of its handle).
    doneResizer().dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true }));
    expect(donePane().style.flexBasis).toBe("384px");

    // ArrowRight moves the Backlog handle right, narrowing Backlog.
    backlogResizer().dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true }));
    expect(backlogPane().style.flexBasis).toBe("316px");
    backlogResizer().dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowLeft", bubbles: true }));
    expect(backlogPane().style.flexBasis).toBe("340px");

    await handle.destroy();
  });

  it("hides a side pane's resizer while it is collapsed", async () => {
    const { root, handle } = await mountThreePane();

    expect(root.querySelector("[data-tg-resizer='done']")).not.toBeNull();
    (root.querySelector("[data-tg-pane-toggle='done']") as HTMLButtonElement).click();
    await flushUntil(() => root.querySelector("[data-tg-pane='done'][data-collapsed]") !== null);
    expect(root.querySelector("[data-tg-resizer='done']")).toBeNull();
    // The Backlog resizer is unaffected.
    expect(root.querySelector("[data-tg-resizer='backlog']")).not.toBeNull();

    await handle.destroy();
  });

  it("keeps a backlog selection across refresh without probing its run", async () => {
    const { root, handle } = await mountThreePane();

    (root.querySelector("[data-tg-pane='backlog'] [data-node-id='backlog-b']") as HTMLButtonElement).click();
    await flushUntil(() =>
      root.querySelector("[data-node-id='backlog-b']")?.classList.contains("is-selected") ?? false,
    );

    await handle.refresh();
    await flushUntil(() =>
      root.querySelector("[data-node-id='backlog-b']")?.classList.contains("is-selected") ?? false,
    );
    // The preserved backlog selection must not trigger an openRun probe
    // against /runs/{backlog identifier}.
    expect(root.textContent).not.toContain("Run COE-461 unavailable");

    await handle.destroy();
  });

  it("selects an untracked active task without a spurious run-unavailable banner", async () => {
    // An issue promoted Backlog→Todo arrives from the tracker scan alone:
    // no run_id and no runtime_overlay, and the gateway has no run detail
    // for it. Clicking it must select the card graph-locally, not surface
    // "Run unavailable" noise.
    const promotedGraph: TaskGraphSnapshot = {
      ...threePaneTaskGraph,
      nodes: [
        ...threePaneTaskGraph.nodes,
        {
          schema_version: schemaVersionV1(),
          node_id: "promoted-todo",
          kind: "issue",
          identifier: "COE-533",
          title: "Freshly promoted todo",
          state: "Todo",
          state_category: "todo",
          children: [],
          blocked_by: [],
          labels: [],
        },
      ],
    };
    const root = document.createElement("div");
    document.body.appendChild(root);
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport: buildTransport({ taskGraph: promotedGraph }),
      graphAdapter: createFixtureGraphAdapter({ completedTasks: completedRows }),
    });
    await flushUntil(() => root.querySelector("[data-node-id='promoted-todo']") !== null);

    (root.querySelector("[data-node-id='promoted-todo']") as HTMLButtonElement).click();
    await flushUntil(() =>
      root.querySelector("[data-node-id='promoted-todo']")?.classList.contains("is-selected") ?? false,
    );
    expect(root.textContent).not.toContain("Run COE-533 unavailable");

    await handle.destroy();
  });

  it("refreshes the Completed pane when live updates complete a task or touch memory", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const rows: MemoryCompletedTask[] = [...completedRows];
    let completedCalls = 0;
    const adapter: GraphDataAdapter = {
      ...createFixtureGraphAdapter(),
      getCompletedTasks: async (options) => {
        completedCalls += 1;
        return pageCompletedTasks(rows, options);
      },
    };
    const transport = new LiveEventTransport({
      baseUri: "http://127.0.0.1:2468",
      health: capabilities,
      snapshot: dashboard,
      taskGraph: threePaneTaskGraph,
      runDetails: [runDetail],
    });
    const handle = renderOpenSymphonyApp({ root, mode: "desktop", transport, graphAdapter: adapter });
    await flushUntil(() =>
      root.querySelector("[data-task-key='COE-448']") !== null
      && root.querySelector("[data-tg-pane='current'] [data-node-id='desktop-alpha']") !== null,
    );

    // A run completing moves its node to `done` and captures a fresh
    // completed row: the live refresh must reload the Completed page.
    rows.unshift({
      issue_key: "COE-449",
      concept_id: "",
      title: "Replace stubs with functional app",
      state: "Done",
      completed_at: "2026-07-01T00:00:00Z",
      prs: [],
      source: "orchestrator",
    });
    transport.setTaskGraph({
      ...threePaneTaskGraph,
      nodes: threePaneTaskGraph.nodes.map((node) =>
        node.node_id === "desktop-alpha"
          ? { ...node, state: "Done", state_category: "done" as const }
          : node,
      ),
    });
    transport.emit({
      schema_version: schemaVersionV1(),
      cursor: { sequence: 1, partition: "events" },
      entity_ref: { kind: "run", id: "COE-449" },
      event_kind: "run.completed",
      emitted_at: "2026-07-01T00:00:01Z",
      payload: { run_id: "COE-449" },
    });
    await flushUntil(() => root.querySelector("[data-task-key='COE-449']") !== null, 200);
    expect(root.querySelector("[data-tg-pane='current'] [data-node-id='desktop-alpha']")).toBeNull();

    // Memory updates can add capsules/PR evidence for completed tasks: the
    // page reloads on memory_graph_updated too.
    const callsBeforeMemory = completedCalls;
    transport.emit({
      schema_version: schemaVersionV1(),
      cursor: { sequence: 2, partition: "events" },
      entity_ref: { kind: "unknown", id: "memory-graph:local-default" },
      event_kind: "memory_graph_updated",
      emitted_at: "2026-07-01T00:00:02Z",
      payload: {
        schema_version: schemaVersionV1(),
        bundle_id: "local-default",
        cursor: { sequence: 2, partition: "memory-graph:local-default" },
        updated_at: "2026-07-01T00:00:02Z",
      },
    });
    await flushUntil(() => completedCalls > callsBeforeMemory, 200);

    await handle.destroy();
  });

  it("opens the memory capsule from a completed row via the deep link", async () => {
    const { root, handle } = await mountThreePane();

    (root.querySelector("[data-tg-capsule]") as HTMLButtonElement).click();
    await flushUntil(() =>
      root.querySelector("[data-testid='knowledge-graph-capsule']") !== null
      || (root.querySelector(".os-kg-breadcrumb")?.textContent?.includes("COE-465") ?? false),
    );

    await handle.destroy();
  });

  it("moves a task between Backlog and Current as its status changes, both ways", async () => {
    const transport = new LiveEventTransport({
      baseUri: "http://127.0.0.1:2468",
      health: capabilities,
      snapshot: dashboard,
      taskGraph: threePaneTaskGraph,
      runDetails: [runDetail],
    });
    const root = document.createElement("div");
    document.body.appendChild(root);
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport,
      graphAdapter: createFixtureGraphAdapter({ completedTasks: [] }),
    });
    await flushUntil(() => root.querySelector("[data-tg-pane='backlog'] [data-node-id='backlog-a']") !== null);
    // Starts in Backlog, absent from Current.
    expect(root.querySelector("[data-tg-pane='current'] [data-node-id='backlog-a']")).toBeNull();

    // Backlog -> Todo: the refreshed snapshot recategorizes the node.
    const promoted = (category: "todo" | "backlog") => ({
      ...threePaneTaskGraph,
      nodes: threePaneTaskGraph.nodes.map((node) =>
        node.node_id === "backlog-a"
          ? { ...node, state: category === "todo" ? "Todo" : "Backlog", state_category: category }
          : node,
      ),
    });
    transport.setTaskGraph(promoted("todo"));
    transport.emit({
      schema_version: schemaVersionV1(),
      cursor: { sequence: 1, partition: "events" },
      entity_ref: { kind: "issue", id: "backlog-a", identifier: "COE-460" },
      event_kind: "issue.updated",
      emitted_at: "2026-07-01T00:00:01Z",
      payload: {},
    });
    await flushUntil(() => root.querySelector("[data-tg-pane='current'] [data-node-id='backlog-a']") !== null);
    expect(root.querySelector("[data-tg-pane='backlog'] [data-node-id='backlog-a']")).toBeNull();

    // Todo -> Backlog: it must return to the Backlog pane and leave Current.
    transport.setTaskGraph(promoted("backlog"));
    transport.emit({
      schema_version: schemaVersionV1(),
      cursor: { sequence: 2, partition: "events" },
      entity_ref: { kind: "issue", id: "backlog-a", identifier: "COE-460" },
      event_kind: "issue.updated",
      emitted_at: "2026-07-01T00:00:02Z",
      payload: {},
    });
    await flushUntil(() => root.querySelector("[data-tg-pane='backlog'] [data-node-id='backlog-a']") !== null);
    expect(root.querySelector("[data-tg-pane='current'] [data-node-id='backlog-a']")).toBeNull();

    await handle.destroy();
  });

  it("reloads Completed when the control-plane completed count rises even if the issue leaves the graph", async () => {
    let completedCalls = 0;
    const adapter: GraphDataAdapter = {
      ...createFixtureGraphAdapter(),
      getCompletedTasks: async (options) => {
        completedCalls += 1;
        return pageCompletedTasks([], options);
      },
    };
    const transport = new LiveEventTransport({
      baseUri: "http://127.0.0.1:2468",
      health: capabilities,
      snapshot: dashboard,
      taskGraph: threePaneTaskGraph,
      runDetails: [runDetail],
    });
    const root = document.createElement("div");
    document.body.appendChild(root);
    const handle = renderOpenSymphonyApp({ root, mode: "desktop", transport, graphAdapter: adapter });
    await flushUntil(() => root.querySelector("[data-tg-pane='current']") !== null && completedCalls >= 1);
    const callsBefore = completedCalls;

    // Simulate a completion whose issue is not present in the task graph
    // (e.g. no project metadata): the task graph is unchanged, but the
    // control-plane completed_count rises. The Completed pane must reload.
    transport.setSnapshot({
      ...dashboard,
      projects: dashboard.projects.map((project, index) =>
        index === 0 ? { ...project, completed_count: project.completed_count + 1 } : project,
      ),
    });
    transport.emit({
      schema_version: schemaVersionV1(),
      cursor: { sequence: 1, partition: "events" },
      entity_ref: { kind: "project", id: dashboard.projects[0].project_id },
      event_kind: "snapshot_published",
      emitted_at: "2026-07-01T00:00:01Z",
      payload: {},
    });
    await flushUntil(() => completedCalls > callsBefore, 200);

    await handle.destroy();
  });

  it("bolds the newest PR by number even when an older PR is the merged one", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const rows: MemoryCompletedTask[] = [
      {
        issue_key: "COE-500",
        concept_id: "",
        title: "Merged then abandoned",
        state: "Done",
        completed_at: "2026-06-10T00:00:00Z",
        // Older PR merged; newer PR abandoned (unmerged, no merged_at).
        prs: [
          { number: 100, title: "first, merged", url: "https://example.com/pull/100", merged: true, merged_at: "2026-06-10T00:00:00Z" },
          { number: 200, title: "second, abandoned", url: "https://example.com/pull/200", merged: false },
        ],
        source: "memory",
      },
    ];
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport: buildTransport({ taskGraph: threePaneTaskGraph }),
      graphAdapter: createFixtureGraphAdapter({ completedTasks: rows }),
    });
    await flushUntil(() => root.querySelector("[data-task-key='COE-500']") !== null);

    const prLinks = Array.from(
      root.querySelector("[data-task-key='COE-500']")?.querySelectorAll(".os-tg-pr") ?? [],
    );
    // Newest by number first, and it is the bold "latest" chip even though
    // it is the unmerged one (struck through).
    expect(prLinks.map((pr) => pr.textContent)).toEqual(["#200", "#100"]);
    expect(prLinks[0]?.classList.contains("os-tg-pr-latest")).toBe(true);
    expect(prLinks[0]?.classList.contains("os-tg-pr-unmerged")).toBe(true);
    expect(prLinks[1]?.classList.contains("os-tg-pr-latest")).toBe(false);

    await handle.destroy();
  });

  it("does not enter focused edge-dimming when the selection transitions to done", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const transport = new LiveEventTransport({
      baseUri: "http://127.0.0.1:2468",
      health: capabilities,
      snapshot: dashboard,
      taskGraph: threePaneTaskGraph,
      runDetails: [runDetail],
    });
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport,
      graphAdapter: createFixtureGraphAdapter({ completedTasks: [] }),
    });
    await flushUntil(() => root.querySelector("[data-tg-pane='current'] [data-node-id='desktop-alpha']") !== null);

    // Select the Current task, then complete it via a live event.
    (root.querySelector("[data-tg-pane='current'] [data-node-id='desktop-alpha']") as HTMLButtonElement).click();
    await flushUntil(() => root.querySelector("[data-node-id='desktop-alpha']")?.classList.contains("is-selected") ?? false);

    transport.setTaskGraph({
      ...threePaneTaskGraph,
      nodes: threePaneTaskGraph.nodes.map((node) =>
        node.node_id === "desktop-alpha"
          ? { ...node, state: "Done", state_category: "done" as const }
          : node,
      ),
    });
    transport.emit({
      schema_version: schemaVersionV1(),
      cursor: { sequence: 1, partition: "events" },
      entity_ref: { kind: "run", id: "COE-449" },
      event_kind: "run.completed",
      emitted_at: "2026-07-01T00:00:01Z",
      payload: { run_id: "COE-449" },
    });
    await flushUntil(() => root.querySelector("[data-tg-pane='current'] [data-node-id='desktop-alpha']") === null);

    // The selection now points at a hidden (done) node; the panes must not
    // be stuck in focused mode dimming every edge with nothing highlighted.
    expect(root.querySelector("[data-tg-panes]")?.classList.contains("os-tg-focused")).toBe(false);

    await handle.destroy();
  });

  it("abandons an in-flight completed-tasks request across a context reset", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    let releaseFirst: (() => void) | null = null;
    let calls = 0;
    const adapter: GraphDataAdapter = {
      ...createFixtureGraphAdapter(),
      getCompletedTasks: async (options) => {
        calls += 1;
        if (calls === 1) {
          // Hold the first (prior-context) request open until after the
          // reset so it resolves late.
          await new Promise<void>((resolve) => {
            releaseFirst = resolve;
          });
          return pageCompletedTasks(
            [{
              issue_key: "STALE-1",
              concept_id: "",
              title: "Stale prior-context row",
              state: "Done",
              completed_at: "2026-06-01T00:00:00Z",
              prs: [],
              source: "orchestrator",
            }],
            options,
          );
        }
        return pageCompletedTasks([], options);
      },
    };
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport: buildTransport({ taskGraph: threePaneTaskGraph }),
      graphAdapter: adapter,
    });
    await flushUntil(() => calls >= 1);

    // A context reset (gateway switch / disconnect) clears and invalidates.
    const internal = handle as unknown as { resetCompletedTasks?: () => void };
    expect(typeof internal.resetCompletedTasks).toBe("function");
    internal.resetCompletedTasks!();
    // Now let the prior-context request resolve; its stale seq must prevent
    // it from repopulating the cleared page.
    releaseFirst?.();
    await flushMicrotasks();
    expect(root.querySelector("[data-task-key='STALE-1']")).toBeNull();
    expect(root.textContent).not.toContain("Stale prior-context row");

    await handle.destroy();
  });

  it("clears Completed rows when a context change hits a gateway without the endpoint", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport: buildTransport({ taskGraph: threePaneTaskGraph }),
      // Bare fixture adapter: no getCompletedTasks (models a gateway with
      // no memory endpoint) but seeded with a stale prior page.
      graphAdapter: {
        ...createFixtureGraphAdapter(),
        getCompletedTasks: undefined,
      },
    });
    await flushUntil(() => root.querySelector("[data-tg-pane='current'] [data-node-id='desktop-alpha']") !== null);

    // The Completed table shows its unavailable state, never rows from a
    // different context.
    expect(root.querySelector("[data-testid='completed-tasks-unavailable']")).not.toBeNull();
    expect(root.querySelector("[data-testid='completed-task-row']")).toBeNull();

    await handle.destroy();
  });

  it("reloads the Completed page when a done row's PR or date changes while staying done", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const rows: MemoryCompletedTask[] = [
      {
        issue_key: "COE-448",
        concept_id: "issues/COE-465",
        bundle_id: "local-default",
        title: "Completed prerequisite",
        state: "Done",
        completed_at: "2026-06-10T00:00:00Z",
        prs: [{ number: 700, title: "COE-448 landed", url: "https://example.com/pull/700", merged: true, merged_at: "2026-06-10T00:00:00Z" }],
        source: "memory",
      },
    ];
    let completedCalls = 0;
    const adapter: GraphDataAdapter = {
      ...createFixtureGraphAdapter(),
      getCompletedTasks: async (options) => {
        completedCalls += 1;
        return pageCompletedTasks(rows, options);
      },
    };
    // Snapshot where the prerequisite is already Done (renders in Completed).
    const doneGraph: TaskGraphSnapshot = {
      ...threePaneTaskGraph,
      nodes: threePaneTaskGraph.nodes.map((node) =>
        node.node_id === "completed-prereq"
          ? { ...node, url: "https://linear.app/example/issue/COE-448" }
          : node,
      ),
    };
    const transport = new LiveEventTransport({
      baseUri: "http://127.0.0.1:2468",
      health: capabilities,
      snapshot: dashboard,
      taskGraph: doneGraph,
      runDetails: [runDetail],
    });
    const handle = renderOpenSymphonyApp({ root, mode: "desktop", transport, graphAdapter: adapter });
    await flushUntil(() => root.querySelector("[data-task-key='COE-448']") !== null);
    const callsBefore = completedCalls;

    // The done node stays done, but its URL changes (e.g. a PR link landed).
    // The fingerprint includes row-relevant fields, so the page reloads.
    rows[0] = { ...rows[0], prs: [...rows[0].prs, { number: 701, title: "COE-448 follow-up", url: "https://example.com/pull/701", merged: true, merged_at: "2026-06-11T00:00:00Z" }] };
    transport.setTaskGraph({
      ...doneGraph,
      nodes: doneGraph.nodes.map((node) =>
        node.node_id === "completed-prereq"
          ? { ...node, updated_at: "2026-06-11T00:00:00Z" }
          : node,
      ),
    });
    transport.emit({
      schema_version: schemaVersionV1(),
      cursor: { sequence: 1, partition: "events" },
      entity_ref: { kind: "issue", id: "completed-prereq", identifier: "COE-448" },
      event_kind: "issue.updated",
      emitted_at: "2026-06-11T00:00:01Z",
      payload: {},
    });
    await flushUntil(() => completedCalls > callsBefore, 200);

    await handle.destroy();
  });
});
