/**
 * @jest-environment jsdom
 *
 * Live-refresh behavior regression tests.
 *
 * The 5s poll used to re-apply a selection captured before its awaits (so a
 * task the user clicked mid-refresh was reverted), null evidence state while
 * loading (panels flashed empty), and rebuild the whole DOM via innerHTML
 * (focus and input state were lost). These tests pin the fixed behavior:
 * refreshes resolve selection at apply time, abandon their results when the
 * user navigates mid-flight, and re-render without stealing focus.
 */

import { renderOpenSymphonyApp } from "../src/app-shell.js";
import { MockGatewayTransport } from "@opensymphony/api-client";
import { schemaVersionV1 } from "@opensymphony/gateway-schema";
import type {
  ChangedFileEntry,
  DashboardSnapshot,
  FileDiffPage,
  GatewayCapabilities,
  RunDetail,
  RunEventPage,
  TaskGraphSnapshot,
} from "@opensymphony/gateway-schema";

const capabilities: GatewayCapabilities = {
  schema_version: schemaVersionV1(),
  gateway_version: "live-refresh-test",
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
    running_issue_count: 1,
    retry_queue_depth: 0,
    total_input_tokens: 100,
    total_output_tokens: 50,
    total_cache_read_tokens: 10,
    total_cost_micros: 1,
  },
  projects: [
    {
      project_id: "proj-alpha",
      name: "Alpha",
      milestone_count: 1,
      issue_count: 2,
      running_count: 1,
      completed_count: 0,
      failed_count: 0,
    },
  ],
  recent_events: [],
};

const taskGraph: TaskGraphSnapshot = {
  schema_version: schemaVersionV1(),
  project_id: "proj-alpha",
  generated_at: "2025-09-01T00:00:00Z",
  root_ids: ["node-a"],
  nodes: [
    {
      schema_version: schemaVersionV1(),
      node_id: "node-a",
      kind: "issue",
      identifier: "RUN-A",
      title: "First task",
      state: "In Progress",
      state_category: "in_progress",
      children: [],
      blocked_by: [],
      labels: [],
    },
    {
      schema_version: schemaVersionV1(),
      node_id: "node-b",
      kind: "issue",
      identifier: "RUN-B",
      title: "Second task",
      state: "Todo",
      state_category: "todo",
      children: [],
      blocked_by: [],
      labels: [],
    },
  ],
};

function buildRunDetail(runId: string): RunDetail {
  return {
    schema_version: schemaVersionV1(),
    run_id: runId,
    issue_id: `issue-${runId}`,
    issue_identifier: runId,
    worker_id: "worker-alpha",
    status: "running",
    claimed_at: "2025-09-01T00:00:00Z",
    started_at: "2025-09-01T00:00:30Z",
    turn_count: 1,
    max_turns: 8,
    input_tokens: 10,
    output_tokens: 5,
    cache_read_tokens: 1,
    runtime_seconds: 30,
    workspace_path: `/tmp/opensymphony/projects/${runId}`,
    safe_actions: {
      retry: false,
      cancel: true,
      rehydrate: true,
      detach: false,
    },
  };
}

const changedFiles: ChangedFileEntry[] = [
  { path: "src/config.ts", change_kind: "modified", lines_added: 12, lines_removed: 3 },
  { path: "src/other.ts", change_kind: "modified", lines_added: 4, lines_removed: 1 },
];

function buildFileDiff(runId: string, filePath: string): FileDiffPage {
  return {
    schema_version: schemaVersionV1(),
    run_id: runId,
    file_path: filePath,
    hunks: [
      {
        file_path: filePath,
        header: "@@ -1 +1 @@",
        start_line: 1,
        old_line_count: 1,
        new_line_count: 1,
        lines: [{ type: "addition", line: `// ${filePath}` }],
      },
    ],
    total_lines_added: 1,
    total_lines_removed: 0,
  };
}

function buildRunEvents(runId: string): RunEventPage {
  return {
    schema_version: schemaVersionV1(),
    run_id: runId,
    events: [
      {
        sequence: 1,
        event_id: `${runId}-evt-1`,
        happened_at: "2025-09-01T00:00:05Z",
        kind: "ActionEvent",
        summary: `action for ${runId}`,
      },
    ],
  };
}

/**
 * Transport whose task-graph reads can be gated so a live refresh can be
 * frozen mid-flight while the test simulates user navigation.
 */
class GatedTransport extends MockGatewayTransport {
  taskGraphReads = 0;
  runDetailGateHits = 0;
  private gateFromRead = Number.POSITIVE_INFINITY;
  private gatedRunDetailId: string | null = null;
  private gateResolvers: Array<() => void> = [];

  /** Gate task-graph reads starting with the given 1-based read number. */
  gateTaskGraphFromRead(read: number): void {
    this.gateFromRead = read;
  }

  /** Gate run-detail reads for one specific run id. */
  gateRunDetailFor(runId: string | null): void {
    this.gatedRunDetailId = runId;
  }

  releaseGatedReads(): void {
    const resolvers = this.gateResolvers.splice(0);
    resolvers.forEach((resolve) => resolve());
  }

  override async taskGraph(projectId: string): Promise<TaskGraphSnapshot> {
    this.taskGraphReads += 1;
    if (this.taskGraphReads >= this.gateFromRead) {
      await new Promise<void>((resolve) => {
        this.gateResolvers.push(resolve);
      });
    }
    return super.taskGraph(projectId);
  }

  override async runDetail(runId: string): Promise<RunDetail> {
    if (runId === this.gatedRunDetailId) {
      this.runDetailGateHits += 1;
      await new Promise<void>((resolve) => {
        this.gateResolvers.push(resolve);
      });
    }
    return super.runDetail(runId);
  }
}

function buildGatedTransport(): GatedTransport {
  return new GatedTransport({
    baseUri: "http://127.0.0.1:2468",
    health: capabilities,
    snapshot: dashboard,
    taskGraph,
    runDetails: [buildRunDetail("RUN-A"), buildRunDetail("RUN-B")],
    runFiles: [
      { runId: "RUN-A", files: changedFiles },
      { runId: "RUN-B", files: changedFiles },
    ],
    runDiffs: [
      { runId: "RUN-A", filePath: "src/config.ts", diff: buildFileDiff("RUN-A", "src/config.ts") },
      { runId: "RUN-A", filePath: "src/other.ts", diff: buildFileDiff("RUN-A", "src/other.ts") },
      { runId: "RUN-B", filePath: "src/config.ts", diff: buildFileDiff("RUN-B", "src/config.ts") },
      { runId: "RUN-B", filePath: "src/other.ts", diff: buildFileDiff("RUN-B", "src/other.ts") },
    ],
    runEvents: [buildRunEvents("RUN-A"), buildRunEvents("RUN-B")],
  });
}

interface LiveRefreshDriver {
  requestLiveRefresh(): Promise<void>;
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

async function mountApp(transport: MockGatewayTransport): Promise<{
  root: HTMLElement;
  handle: Awaited<ReturnType<typeof renderOpenSymphonyApp>>;
}> {
  const root = document.createElement("div");
  document.body.appendChild(root);
  const handle = renderOpenSymphonyApp({
    root,
    mode: "desktop",
    title: "Live Refresh Test",
    transport,
  });
  await flushUntil(() => root.querySelector(".os-run-head strong")?.textContent === "RUN-A");
  await flushUntil(() => root.querySelector("[data-testid='changed-file-item']") !== null);
  return { root, handle };
}

function selectedNode(root: HTMLElement): string | null {
  return root.querySelector<HTMLElement>("[data-node-id].os-node-selected, [data-node-id][aria-selected='true'], [data-node-id][data-selected='true'], [data-node-id].is-selected")?.dataset.nodeId
    ?? root.querySelector(".os-run-head strong")?.textContent
    ?? null;
}

afterEach(() => {
  document.body.replaceChildren();
});

describe("live refresh vs. user navigation", () => {
  it("does not revert a task the user opened while a refresh was in flight", async () => {
    const transport = buildGatedTransport();
    const { root, handle } = await mountApp(transport);
    const driver = handle as unknown as LiveRefreshDriver;

    // Freeze the next task-graph read: the refresh will hang there while the
    // user clicks a different task.
    transport.gateTaskGraphFromRead(transport.taskGraphReads + 1);
    const refresh = driver.requestLiveRefresh();
    await flushUntil(() => transport.taskGraphReads >= 2);

    const nodeB = root.querySelector<HTMLElement>("[data-node-id='node-b']");
    expect(nodeB).not.toBeNull();
    nodeB!.click();
    await flushUntil(() => root.querySelector(".os-run-head strong")?.textContent === "RUN-B");

    transport.releaseGatedReads();
    await refresh;
    await flushAsync();

    // The stale refresh must not restore RUN-A.
    expect(root.querySelector(".os-run-head strong")?.textContent).toBe("RUN-B");
    expect(selectedNode(root)).toContain("node-b");

    await handle.destroy();
  });

  it("preserves the selected diff file and evidence view across a refresh", async () => {
    const transport = buildGatedTransport();
    const { root, handle } = await mountApp(transport);
    const driver = handle as unknown as LiveRefreshDriver;

    // Select the non-default diff file.
    const otherFile = root.querySelector<HTMLElement>("[data-testid='changed-file-item'][data-path='src/other.ts']");
    expect(otherFile).not.toBeNull();
    otherFile!.click();
    await flushUntil(() =>
      root.querySelector("[data-testid='file-diff']")?.getAttribute("data-file-path") === "src/other.ts",
    );

    await driver.requestLiveRefresh();
    await flushAsync();

    const selectedFile = root.querySelector<HTMLElement>("[data-testid='changed-file-item'].os-selected");
    expect(selectedFile?.dataset.path).toBe("src/other.ts");
    expect(root.querySelector("[data-testid='file-diff']")?.getAttribute("data-file-path")).toBe("src/other.ts");

    // Same for the Activity evidence view.
    root.querySelector<HTMLElement>("[data-evidence-view='activity']")!.click();
    await flushUntil(() =>
      root.querySelector("[data-evidence-view='activity']")?.classList.contains("is-selected") ?? false,
    );
    await driver.requestLiveRefresh();
    await flushAsync();
    expect(root.querySelector("[data-evidence-view='activity']")?.classList.contains("is-selected")).toBe(true);

    await handle.destroy();
  });

  it("keeps evidence panels populated during a refresh instead of flashing empty", async () => {
    const transport = buildGatedTransport();
    const { root, handle } = await mountApp(transport);
    const driver = handle as unknown as LiveRefreshDriver;

    // Freeze the refresh mid-flight and force a re-render: previously the
    // refresh nulled runFiles/runDiff up front, so any render in that window
    // showed empty panels.
    transport.gateTaskGraphFromRead(transport.taskGraphReads + 1);
    const refresh = driver.requestLiveRefresh();
    await flushUntil(() => transport.taskGraphReads >= 2);

    expect(root.querySelectorAll("[data-testid='changed-file-item']").length).toBeGreaterThan(0);
    expect(root.querySelector("[data-testid='file-diff']")).not.toBeNull();

    transport.releaseGatedReads();
    await refresh;
    expect(root.querySelectorAll("[data-testid='changed-file-item']").length).toBeGreaterThan(0);
    expect(root.querySelector("[data-testid='file-diff']")).not.toBeNull();

    await handle.destroy();
  });

  it("does not cancel an in-flight task open when a stale diff file is clicked", async () => {
    const transport = buildGatedTransport();
    const { root, handle } = await mountApp(transport);

    // Freeze RUN-B's detail read so opening node-b hangs mid-flight.
    transport.gateRunDetailFor("RUN-B");
    root.querySelector<HTMLElement>("[data-node-id='node-b']")!.click();
    await flushUntil(() => transport.runDetailGateHits >= 1);

    // While RUN-B is loading, RUN-A's changed files are still on screen;
    // clicking one used to bump the shared epoch and abort the open.
    const staleFile = root.querySelector<HTMLElement>("[data-testid='changed-file-item'][data-path='src/other.ts']");
    expect(staleFile).not.toBeNull();
    staleFile!.click();
    await flushAsync();

    transport.gateRunDetailFor(null);
    transport.releaseGatedReads();
    await flushUntil(() => root.querySelector(".os-run-head strong")?.textContent === "RUN-B");

    // The opened run applied; its diff selection reset to the new run's data.
    expect(root.querySelector(".os-run-head strong")?.textContent).toBe("RUN-B");

    await handle.destroy();
  });

  it("keeps focus and typed text in the task filter across a refresh", async () => {
    const transport = buildGatedTransport();
    const { root, handle } = await mountApp(transport);
    const driver = handle as unknown as LiveRefreshDriver;

    const search = root.querySelector<HTMLInputElement>("[data-tg-filter='search']");
    expect(search).not.toBeNull();
    search!.focus();
    search!.value = "half-typed query";

    await driver.requestLiveRefresh();
    await flushAsync();

    const searchAfter = root.querySelector<HTMLInputElement>("[data-tg-filter='search']");
    expect(document.activeElement).toBe(searchAfter);
    expect(searchAfter?.value).toBe("half-typed query");

    await handle.destroy();
  });
});
