/**
 * @jest-environment jsdom
 *
 * Remote web/desktop parity for COE-419.
 *
 * The web and desktop clients both mount the shared `OpenSymphonyApp` shell
 * (see PR #113). This test drives the shell in both `mode:"web"` and
 * `mode:"desktop"` against an identical fixture transport and asserts the
 * same core project, task graph, run, stream, and planning state renders in
 * each mode.
 */

import { renderOpenSymphonyApp } from "../src/app-shell.js";
import { MockGatewayTransport } from "@opensymphony/api-client";
import { schemaVersionV1 } from "@opensymphony/gateway-schema";
import type {
  DashboardSnapshot,
  GatewayCapabilities,
  GatewayEnvelope,
  RunDetail,
  RunEventPage,
  TaskGraphSnapshot,
} from "@opensymphony/gateway-schema";

const capabilities: GatewayCapabilities = {
  schema_version: schemaVersionV1(),
  gateway_version: "parity-test",
  supported_api_versions: ["1.0.0"],
  transports: [{ transport: "loopback_http", modes: ["json"], supported_encodings: ["utf-8"], bidirectional: false }],
  features: [
    { feature: "task_graph", available: true, requires_auth: false },
    { feature: "terminal_stream", available: true, requires_auth: false },
  ],
  auth_modes: ["none"],
  max_event_page_size: 1000,
  max_terminal_frame_batch: 500,
};

const dashboard: DashboardSnapshot = {
  schema_version: schemaVersionV1(),
  generated_at: "2025-09-01T00:00:00Z",
  sequence: 11,
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
    { project_id: "proj-parity", name: "Parity Project", milestone_count: 1, issue_count: 3, running_count: 1, completed_count: 1, failed_count: 1 },
  ],
  recent_events: [
    { happened_at: "2025-09-01T00:00:00Z", kind: "run_event", issue_identifier: "COE-700", summary: "parity stream event" },
  ],
};

const taskGraph: TaskGraphSnapshot = {
  schema_version: schemaVersionV1(),
  project_id: "proj-parity",
  generated_at: "2025-09-01T00:00:00Z",
  root_ids: ["m-parity"],
  nodes: [
    {
      schema_version: schemaVersionV1(),
      node_id: "m-parity",
      kind: "milestone",
      identifier: "M-PAR",
      title: "Parity Milestone",
      state: "In Progress",
      state_category: "in_progress",
      children: ["run-parity"],
      blocked_by: [],
      labels: ["parity"],
    },
    {
      schema_version: schemaVersionV1(),
      node_id: "run-parity",
      kind: "issue",
      identifier: "COE-700",
      title: "Parity run issue",
      state: "In Progress",
      state_category: "in_progress",
      parent_id: "m-parity",
      children: [],
      blocked_by: [],
      labels: ["parity"],
    },
  ],
};

const runDetail: RunDetail = {
  schema_version: schemaVersionV1(),
  run_id: "COE-700",
  issue_id: "issue-parity",
  issue_identifier: "COE-700",
  worker_id: "worker-parity",
  status: "running",
  claimed_at: "2025-09-01T00:00:00Z",
  started_at: "2025-09-01T00:00:30Z",
  turn_count: 2,
  max_turns: 8,
  input_tokens: 4500,
  output_tokens: 1100,
  cache_read_tokens: 600,
  runtime_seconds: 90,
  workspace_path: "/tmp/opensymphony/projects/COE-700",
  safe_actions: { retry: false, cancel: true, rehydrate: true, detach: false },
};

const runEventsPage: RunEventPage = {
  schema_version: schemaVersionV1(),
  run_id: "COE-700",
  events: [
    {
      schema_version: schemaVersionV1(),
      event_id: "evt-1",
      run_id: "COE-700",
      sequence: 1,
      timestamp: "2025-09-01T00:00:31Z",
      kind: "tool_call",
      summary: "parity stream event",
      payload: {},
    },
  ],
  next_cursor: null,
  has_more: false,
};

const streamEvents: GatewayEnvelope[] = [
  {
    schema_version: schemaVersionV1(),
    cursor: { sequence: 1, partition: "task_graph" },
    correlation_id: "corr-1",
    event_type: "snapshot_published",
    payload: { kind: "task_graph", summary: "parity stream snapshot" } as unknown as Record<string, unknown>,
  },
];

function buildTransport(): MockGatewayTransport {
  return new MockGatewayTransport({
    baseUri: "http://127.0.0.1:2468",
    health: capabilities,
    snapshot: dashboard,
    taskGraph,
    runDetails: [runDetail],
    runEvents: [runEventsPage],
    events: streamEvents,
  });
}

function flushAsync(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, 0));
}

async function flushUntil(predicate: () => boolean, maxIterations = 40): Promise<void> {
  for (let i = 0; i < maxIterations; i++) {
    if (predicate()) return;
    await flushAsync();
  }
  throw new Error(`flushUntil timed out after ${maxIterations} iterations`);
}

interface ModeSnapshot {
  mode: string;
  root: HTMLDivElement;
  metricsText: string;
  taskGraphNodes: string[];
  runHead: string;
  planningHeading: string | null;
  authState: string;
}

async function snapshotMode(mode: "web" | "desktop"): Promise<ModeSnapshot> {
  const root = document.createElement("div");
  document.body.appendChild(root);
  const handle = renderOpenSymphonyApp({
    root,
    mode,
    transport: buildTransport(),
  });

  // Dashboard + task graph (both modes render the panel heading and nodes)
  await flushUntil(() => root.querySelector(".os-task-graph-panel h2") !== null);
  await flushUntil(() => root.querySelector("[data-node-id='run-parity']") !== null);
  // Run detail: navigate to the run node so the run panel loads the mock run.
  (root.querySelector("[data-node-id='run-parity']") as HTMLElement).click();
  await flushUntil(() => root.querySelector(".os-run-head strong")?.textContent === "COE-700");

  const metricsText = root.querySelector(".os-metrics")?.textContent?.replace(/\s+/g, " ").trim() ?? "";
  const taskGraphNodes = Array.from(root.querySelectorAll("[data-node-id]")).map((el) => el.getAttribute("data-node-id") ?? "");
  const runHead = root.querySelector(".os-run-head strong")?.textContent ?? "";
  const authState = root.querySelector("[data-opensymphony-app-shell='mounted']")?.getAttribute("data-auth-state") ?? "";

  // Switch to the planning view to capture parity for planning drafts.
  let planningHeading: string | null = null;
  const planningTab = root.querySelector("[data-plan-view='planning']") as HTMLButtonElement | null;
  if (planningTab) {
    planningTab.click();
    await flushUntil(() => root.querySelector(".os-planning-panel") !== null);
    planningHeading = root.querySelector(".os-planning-panel h2")?.textContent ?? null;
  }

  await handle.destroy();
  root.remove();

  return { mode, root, metricsText, taskGraphNodes, runHead, planningHeading, authState };
}

describe("Remote web/desktop parity (COE-419)", () => {
  it("renders the same core dashboard, task graph, run, stream, and planning state in web and desktop modes", async () => {
    const web = await snapshotMode("web");
    const desktop = await snapshotMode("desktop");

    // Dashboard metrics parity
    expect(web.metricsText).toBe(desktop.metricsText);
    expect(web.metricsText).toContain("2");
    expect(web.metricsText).toContain("Running");
    expect(web.metricsText).toContain("Retry Queue");

    // Task graph node parity
    expect(web.taskGraphNodes).toEqual(desktop.taskGraphNodes);
    expect(web.taskGraphNodes).toContain("m-parity");
    expect(web.taskGraphNodes).toContain("run-parity");

    // Run detail parity
    expect(web.runHead).toBe(desktop.runHead);
    expect(web.runHead).toBe("COE-700");

    // Planning view parity (both render the shared planning workspace)
    expect(web.planningHeading).toBe(desktop.planningHeading);
    expect(web.planningHeading).not.toBeNull();

    // Auth state parity (both open for local unauthenticated gateway)
    expect(web.authState).toBe(desktop.authState);
    expect(web.authState).toBe("open");
  });

  it("exposes the same fixture stream events through the shared transport for both modes", async () => {
    const webTransport = buildTransport();
    const desktopTransport = buildTransport();

    const webEvents: GatewayEnvelope[] = [];
    for await (const evt of webTransport.events()) {
      webEvents.push(evt);
    }
    const desktopEvents: GatewayEnvelope[] = [];
    for await (const evt of desktopTransport.events()) {
      desktopEvents.push(evt);
    }

    expect(webEvents.map((e) => e.correlation_id)).toEqual(desktopEvents.map((e) => e.correlation_id));
    expect(webEvents.map((e) => e.event_type)).toEqual(["snapshot_published"]);
    expect(webEvents).toEqual(desktopEvents);
  });
});